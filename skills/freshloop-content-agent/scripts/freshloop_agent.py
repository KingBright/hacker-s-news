#!/usr/bin/env python3
"""Transport and checkpoints only. The invoking mature agent authors the artifact."""
import argparse
import contextlib
import fcntl
import hashlib
import json
import os
import re
from pathlib import Path
import ssl
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
from datetime import datetime
from zoneinfo import ZoneInfo

JOB_TYPES = ('radio_program', 'radio_episode', 'reading_article', 'weekly_digest', 'loop_preference_extraction')


def save(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_name(path.name + '.' + uuid.uuid4().hex + '.tmp')
    with os.fdopen(os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), 'w') as f:
        json.dump(value, f, ensure_ascii=False, indent=2)
    os.replace(tmp, path)


def read(path):
    return json.loads(Path(path).read_text())


class ApiError(Exception):
    def __init__(self, status, message):
        self.status = status
        super().__init__(f'HTTP {status}: {message[:500]}')


class Api:
    def __init__(self, base, key, header):
        self.base = base.rstrip('/')
        self.key, self.header = key, header
        parsed = urllib.parse.urlparse(self.base)
        if parsed.scheme not in ('http', 'https') or (parsed.scheme == 'http' and parsed.hostname not in ('127.0.0.1', 'localhost', '::1')):
            raise ValueError('Use HTTPS, or loopback HTTP through a local connection/tunnel')
        self.context = ssl.create_default_context(cafile=os.environ.get('FRESHLOOP_CA_FILE'))

    def call(self, path, body=None, retry=True, binary=False):
        data = None if body is None else json.dumps(body, ensure_ascii=False).encode()
        headers = {'Content-Type': 'application/json'}
        if self.key:
            headers[self.header] = self.key
        for attempt in range(3):
            try:
                request = urllib.request.Request(self.base + path, data=data, headers=headers)
                with urllib.request.urlopen(request, timeout=45, context=self.context) as response:
                    raw = response.read(64 * 1024 * 1024 + 1)
                    if len(raw) > 64 * 1024 * 1024:
                        raise RuntimeError("API response exceeds 64 MiB limit")
                    return raw if binary else (json.loads(raw) if raw else {})
            except urllib.error.HTTPError as exc:
                message = exc.read(2048).decode(errors='replace').replace(self.key, '[redacted]') if self.key else exc.read(2048).decode(errors='replace')
                if exc.code < 500 or not retry or attempt == 2:
                    raise ApiError(exc.code, message) from None
            except (urllib.error.URLError, TimeoutError):
                if not retry or attempt == 2:
                    raise RuntimeError('Request outcome unknown; preserve checkpoint and reconcile before further writes') from None
            time.sleep(2 ** attempt)


def nexus():
    return Api(os.environ['NEXUS_URL'], os.environ['NEXUS_KEY'], 'X-NEXUS-KEY')


def job_path(state, suffix):
    return '/api/internal/agent/jobs/' + urllib.parse.quote(state['job_id'], safe='') + '/' + suffix


def validate_radio_digest(artifact):
    """Structural guard; semantic event independence remains the editor's duty."""
    stories = artifact.get('stories')
    if not isinstance(stories, list) or not stories:
        return ['Radio category digest requires all qualifying stories, at least one']
    sources = artifact.get('sources', [])
    if not isinstance(sources, list):
        return ['Radio sources must be an array']
    source_urls = {s['url'].split('#', 1)[0] for s in sources if isinstance(s, dict) and valid_url(s.get('url'))}
    claims = artifact.get('editorial', {}).get('claims', [])
    claim_urls = {c['source_url'].split('#', 1)[0] for c in claims if isinstance(c, dict) and valid_url(c.get('source_url'))} if isinstance(claims, list) else set()
    compact = lambda text: ''.join(text.split()) if isinstance(text, str) else ''
    manuscript = compact(artifact.get('script'))
    spoken = compact(artifact.get('audio_script', artifact.get('script')))
    errors, keys, titles, bodies = [], set(), set(), set()
    for story in stories:
        if not isinstance(story, dict):
            errors.append('each Radio story must be an object'); continue
        key, title, body = (compact(story.get(k)) for k in ('event_key', 'title', 'script'))
        spoken_body = compact(story.get('audio_script', story.get('script')))
        if not key or not title or key.casefold() in keys or title.casefold() in titles or body in bodies:
            errors.append('Radio stories require distinct event_key, title and body')
        keys.add(key.casefold()); titles.add(title.casefold()); bodies.add(body)
        tier = story.get('tier')
        minimum = 10 if tier == 'brief' else 60
        if tier not in ('major', 'brief'):
            errors.append('Radio story tier must be major or brief')
        if len(body) < minimum or len(spoken_body) < minimum or body not in manuscript or spoken_body not in spoken:
            errors.append('each Radio story must meet its tier length and appear in displayed and spoken manuscripts')
        refs = story.get('source_urls', [])
        if not isinstance(refs, list) or not refs or any(not valid_url(url) for url in refs):
            errors.append('each Radio story needs source_urls'); continue
        urls = {url.split('#', 1)[0] for url in refs}
        if not urls <= source_urls or not urls & claim_urls:
            errors.append('Radio story sources must be declared and backed by editorial claim evidence')
    coverage = artifact.get('editorial', {}).get('radio_coverage', {})
    eligible = coverage.get('eligible_event_keys') if isinstance(coverage, dict) else None
    if not isinstance(eligible, list) or not eligible or any(not isinstance(k, str) or not compact(k) for k in eligible):
        errors.append('Radio requires reviewed coverage with eligible_event_keys')
    else:
        expected = [compact(k).casefold() for k in eligible]
        if coverage.get('reviewed') is not True or len(set(expected)) != len(expected) or set(expected) != keys:
            errors.append('Radio coverage must match every eligible event exactly once')
    return errors


def validate(artifact, job_type):
    errors = []
    if job_type == 'radio_program':
        for field, maximum in (('title', 160), ('opening', 400), ('closing', 200)):
            value = artifact.get(field)
            if not isinstance(value, str) or not value.strip() or len(value) > maximum or '<' in value or 'http' in value:
                errors.append(f'invalid program {field}')
        sections = artifact.get('sections')
        if not isinstance(sections, list) or not 1 <= len(sections) <= 30:
            return errors + ['program requires 1–30 category sections']
        categories = set()
        for section in sections:
            if not isinstance(section, dict):
                errors.append('section must be a Radio artifact'); continue
            category = section.get('category')
            if not isinstance(category, str) or category in categories:
                errors.append('program categories must be distinct')
            else:
                categories.add(category)
            errors.extend(validate(section, 'radio_episode'))
        return errors

    required = {
        'radio_episode': ('title', 'category', 'script'),
        'reading_article': ('title', 'original_url', 'reader_markdown', 'compressed_markdown', 'audio_script'),
        'weekly_digest': ('title', 'digest_markdown', 'audio_script'),
        'loop_preference_extraction': ('post_id', 'user_id', 'status'),
    }[job_type]
    for name in required:
        if not isinstance(artifact.get(name), str) or not artifact[name].strip():
            errors.append(f'missing text field: {name}')
    if job_type == 'loop_preference_extraction':
        if artifact.get('status') not in ('processed', 'skipped'):
            errors.append('Loop artifact must be processed or skipped')
        return errors
    spoken = artifact.get('audio_script', artifact.get('script', ''))
    if not isinstance(spoken, str):
        return errors + ['spoken manuscript must be text']
    chars = sum(not char.isspace() for char in spoken)
    han = sum('\u4e00' <= char <= '\u9fff' for char in spoken)
    if chars < (10 if job_type == 'radio_episode' else 60) or han * 5 < chars:
        errors.append('require a substantive Chinese listening manuscript, not a source copy')
    if any(token in spoken for token in ('```', '&mdash;', '&nbsp;', '$1', '$2', 'http://', 'https://', '![', '<phoneme', '<speak', '<say-as')):
        errors.append('spoken manuscript contains markup, raw URLs or extraction debris')
    if len(spoken) > 6500:
        errors.append('spoken manuscript exceeds 6500 characters; split the editorial scope')
    review = artifact.get('editorial', {})
    if not isinstance(review, dict):
        return errors + ['editorial must be an object']
    if review.get('language') != 'zh-CN' or review.get('ready') is not True:
        errors.append('require reviewed zh-CN manuscript with editorial.ready=true')
    if not review.get('claims') or not review.get('dedup_checked'):
        errors.append('require source-grounded claims and dedup_checked=true')
    scores = review.get('quality', {})
    dimensions = ('fidelity', 'selection', 'listening', 'structure', 'media')
    if not isinstance(scores, dict) or any(type(scores.get(k)) is not int or scores[k] not in (0, 1, 2) for k in dimensions):
        errors.append('editorial.quality requires five integer scores 0..2: ' + ', '.join(dimensions))
    elif scores['fidelity'] == 0 or sum(scores[k] for k in dimensions) < 8:
        errors.append('editorial quality must total at least 8/10 with nonzero fidelity')
    if job_type == 'reading_article':
        reader = artifact.get('reader_markdown', '')
        # Preserve legitimate fenced code examples; reject known page chrome
        # observed in real scheduled-run acceptance, not arbitrary code articles.
        prose = re.sub(r'```[\s\S]*?```', '', reader) if isinstance(reader, str) else ''
        if any(token in prose for token in ('.embed-container{', 'remark\\_config={host:', 'fathom("trackPageview")')) or 'document.querySelectorAll' in prose.split('\n', 1)[0]:
            errors.append('reader_markdown contains page scripts/CSS; extract and review the actual article')
        if '<img ' in prose or re.search(r'!\[[^\]]*\]\(https?://(?:www\.)?(?:youtube\.com/watch|youtu\.be/)', prose):
            errors.append('use absolute Markdown image URLs; render YouTube as an original-video link')
    claims = review.get('claims', [])
    media_items = review.get('media', [])
    if not isinstance(claims, list) or not isinstance(media_items, list):
        return errors + ['editorial claims and media must be arrays']
    for claim in claims:
        if not isinstance(claim, dict) or not claim.get('claim') or not claim.get('evidence') or not valid_url(claim.get('source_url')):
            errors.append('every claim needs source_url and an evidence passage or timestamp')
    for media in media_items:
        if not isinstance(media, dict):
            errors.append('media must be an object'); continue
        if media.get('status') not in ('inspected', 'transcript_only', 'unavailable') or not valid_url(media.get('url')):
            errors.append('media needs a real URL and an honest inspection status')
        if media.get('status') == 'inspected' and not media.get('observation'):
            errors.append('inspected media requires an observation')
    if job_type == 'reading_article' and not valid_url(artifact.get('original_url')):
        errors.append('invalid original_url')
    if job_type == 'radio_episode':
        errors.extend(validate_radio_digest(artifact))
    if job_type == 'weekly_digest':
        if len(set(artifact.get('included_item_ids', []))) < 3:
            errors.append('weekly digest needs at least three distinct published items')
        if not isinstance(artifact.get('week_start'), int) or not isinstance(artifact.get('week_end'), int) or artifact.get('week_start', 0) >= artifact.get('week_end', 0):
            errors.append('invalid weekly interval')
    return errors


def valid_url(value):
    if not isinstance(value, str):
        return False
    p = urllib.parse.urlparse(value)
    return p.scheme in ('http', 'https') and bool(p.hostname) and not p.username


def begin(args):
    api = nexus()
    capabilities = api.call('/api/internal/agent/capabilities')
    if capabilities.get('schema_version', 0) < 2:
        raise RuntimeError('Nexus needs the v2 atomic/idempotent Agent API before scheduling')
    if Path(args.state).exists():
        state = read(args.state)
        if state['job_type'] != args.job_type or state['slot'] != args.slot or state['date'] != args.date:
            raise ValueError('checkpoint belongs to different work; choose another state file')
    else:
        logical = f'freshloop:v2:{args.job_type}:{args.date}:{args.slot}'
        state = {'job_id': logical, 'job_type': args.job_type, 'date': args.date, 'slot': args.slot,
                 'agent_id': os.environ.get('AGENT_ID', 'mature-agent') + ':' + uuid.uuid4().hex, 'status': 'starting'}
        save(args.state, state)
    api.call('/api/internal/agent/jobs', {'id': state['job_id'], 'job_type': state['job_type'], 'context': {
        'production_mode': 'external_agent', 'run_date': args.date, 'slot': args.slot, 'language': 'zh-CN'}, 'input': {}})
    context = api.call(job_path(state, 'context'))
    job = context['job']
    if job['status'] == 'completed':
        state.update(status='completed', result_ref=job.get('result_ref'))
    elif job['status'] in ('failed', 'cancelled') or (job.get('attempt_count') or 0) >= (job.get('max_attempts') or 3) and (job['status'] != 'leased' or (job.get('lease_expires_at') or 0) < time.time()):
        state.update(status='failed', last_error=job.get('last_error') or 'job exhausted or cancelled')
    elif job['status'] == 'leased' and job.get('lease_owner') == state['agent_id'] and (job.get('lease_expires_at') or 0) >= time.time():
        state.update(status='leased', lease_token=job['lease_token'], lease_expires_at=job['lease_expires_at'])
    else:
        # Lease is not blindly retried: if its reply is lost, rerun begin to reconcile exact ownership.
        result = api.call('/api/internal/agent/jobs/lease', {'job_id': state['job_id'], 'job_types': [state['job_type']],
            'agent_id': state['agent_id'], 'lease_seconds': 1800}, retry=False)
        if not result.get('job'):
            state['status'] = 'no_job'
        else:
            state.update(status='leased', lease_token=result['job']['lease_token'], lease_expires_at=result['job']['lease_expires_at'])
    save(args.state, state)
    print(json.dumps({k:state.get(k) for k in ('job_id','status','lease_expires_at','result_ref')}))


@contextlib.contextmanager
def checkpoint_lock(path):
    if not path:
        yield
        return
    target=Path(str(path)+'.lock')
    target.parent.mkdir(parents=True,exist_ok=True)
    with os.fdopen(os.open(target,os.O_RDWR|os.O_CREAT,0o600),'w') as handle:
        try:
            fcntl.flock(handle,fcntl.LOCK_EX|fcntl.LOCK_NB)
        except BlockingIOError:
            raise RuntimeError('another process is using this checkpoint') from None
        yield


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    p = sub.add_parser('begin'); p.add_argument('--job-type', choices=JOB_TYPES, required=True); p.add_argument('--slot', required=True)
    p.add_argument('--date', default=datetime.now(ZoneInfo('Asia/Shanghai')).date().isoformat()); p.add_argument('--state', required=True)
    for command in ('heartbeat', 'status', 'submit', 'fail'):
        p = sub.add_parser(command); p.add_argument('--state', required=True)
        if command == 'submit': p.add_argument('--artifact', required=True)
        if command == 'fail': p.add_argument('--error', required=True)
    p = sub.add_parser('validate'); p.add_argument('--job-type', choices=JOB_TYPES, required=True); p.add_argument('--artifact', required=True)
    p = sub.add_parser('sources'); p.add_argument('--product-line', choices=('radio','reading')); p.add_argument('--offset', type=int, default=0); p.add_argument('--out', required=True)
    p = sub.add_parser('source'); p.add_argument('--id', required=True); p.add_argument('--out', required=True)
    p = sub.add_parser('media'); p.add_argument('--id', required=True); p.add_argument('--index', type=int, required=True); p.add_argument('--out', required=True)
    p = sub.add_parser('refresh'); p.add_argument('--out', required=True)
    args = parser.parse_args()
    with checkpoint_lock(getattr(args,"state",None)):
        return dispatch(args)


def dispatch(args):
    if args.command == 'begin': return begin(args)
    if args.command == 'validate':
        errors=validate(read(args.artifact),args.job_type)
        print(json.dumps({'valid': not errors,'errors':errors}, ensure_ascii=False)); return bool(errors)
    if args.command in ('sources', 'source', 'refresh', 'media'):
        api=Api(os.environ.get('CORTEX_URL','http://127.0.0.1:3721'),os.environ.get('CORTEX_API_KEY',''),'X-CORTEX-KEY')
        if args.command == 'media':
            data=api.call('/api/agent/sources/'+urllib.parse.quote(args.id,safe='')+'/media/'+str(args.index),binary=True)
            path=Path(args.out); path.parent.mkdir(parents=True,exist_ok=True)
            with os.fdopen(os.open(path,os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600),'wb') as f: f.write(data)
            print(json.dumps({'saved':args.out,'bytes':len(data),'sha256':hashlib.sha256(data).hexdigest()})); return
        if args.command == 'sources':
            path='/api/agent/sources?' + urllib.parse.urlencode({k:v for k,v in {'product_line':args.product_line,'offset':args.offset,'limit':30}.items() if v is not None})
            result=api.call(path)
        elif args.command == 'source': result=api.call('/api/agent/sources/'+urllib.parse.quote(args.id,safe='')+'/fetch',{})
        else: result=api.call('/api/agent/sources/refresh',{},retry=False)
        save(args.out,result); print(json.dumps({'saved':args.out})); return
    state=read(args.state); api=nexus()
    if args.command == 'status':
        job=api.call(job_path(state,'context'))['job']
        print(json.dumps({k:job.get(k) for k in ('id','status','result_ref','last_error')})); return
    if 'lease_token' not in state: raise ValueError('no owned lease in checkpoint')
    body={'lease_token':state['lease_token']}
    if args.command == 'heartbeat': body['lease_seconds']=1800
    if args.command == 'fail': body['error']=args.error
    if args.command == 'submit':
        artifact=read(args.artifact); errors=validate(artifact,state['job_type'])
        if errors: raise ValueError('; '.join(errors))
        digest=hashlib.sha256(json.dumps(artifact,sort_keys=True,ensure_ascii=False).encode()).hexdigest()
        if state.get('pending_hash') and state['pending_hash'] != digest:
            raise ValueError('pending submission differs; reconcile status before changing artifact')
        state['pending_hash']=digest; save(args.state,state); body['artifact']=artifact
    try:
        result=api.call(job_path(state,args.command),body)
    except ApiError as exc:
        if args.command == 'submit' and exc.status == 400:
            state.pop('pending_hash',None); save(args.state,state)
        raise
    if args.command == 'heartbeat': state['lease_expires_at']=result['lease_expires_at']
    if args.command == 'submit': state.update(status='completed',result_ref=result.get('result_ref'),voice_job_ids=result.get('voice_job_ids',[]))
    if args.command == 'fail': state['status']='failed'
    save(args.state,state)
    print(json.dumps(result,ensure_ascii=False))


if __name__ == '__main__':
    try:
        sys.exit(main() or 0)
    except (ApiError, RuntimeError, ValueError, KeyError) as exc:
        print(str(exc),file=sys.stderr); sys.exit(1)
