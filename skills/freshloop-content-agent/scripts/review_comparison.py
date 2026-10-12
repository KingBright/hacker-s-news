"""Source-bound editorial review and compact run issues; no model/API calls."""
import contextlib
import fcntl
import hashlib
import json
import os
import re
from pathlib import Path
import time
from article_view import article_view

DIMENSIONS = {'actor', 'event', 'time', 'numbers', 'limits', 'core_meaning', 'important_omissions', 'unsupported_additions', 'compression'}
MAX_DRAFTS = 3


class EditorialError(ValueError):
    def __init__(self, code, reason, stage='review', source_ids=None, unit=None):
        super().__init__(reason)
        self.code, self.stage = code, stage
        self.source_ids, self.unit = source_ids or [], unit


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, ensure_ascii=False).encode()).hexdigest()


def save(path, value):
    path = Path(path); path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + '.tmp-' + str(os.getpid()))
    with os.fdopen(os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600), 'w') as f:
        json.dump(value, f, ensure_ascii=False, indent=2)
    os.replace(temporary, path)


def read(path):
    return json.loads(Path(path).read_text())


def run_root(path):
    p = Path(path).resolve()
    parts = p.parts
    if 'agent-runs' in parts:
        index = parts.index('agent-runs')
        if len(parts) > index + 2:
            return Path(*parts[:index + 3])
    return p.parent


def bounded_path(root, value):
    given = Path(value)
    p = given.resolve() if given.is_absolute() or given.exists() else (root / given).resolve()
    if not p.is_relative_to(root.resolve()):
        raise EditorialError('invalid_source_path', 'Review source/input path escapes the private run directory')
    return p


def source_input(path):
    try: record=read(path)
    except (ValueError,OSError) as exc:
        raise EditorialError('source_unavailable','无法完成来源核验：来源快照不可读取（'+type(exc).__name__+'）','source',[Path(path).name]) from None
    if not isinstance(record,dict):
        raise EditorialError('source_unavailable','无法完成来源核验：来源快照必须是对象','source',[Path(path).name])
    ident = record.get('id')
    entry=record.get('entry') if isinstance(record.get('entry'),dict) else {}
    url = entry.get('link')
    view = article_view(record)  # derive from original HTML, not a forged stored view
    if record.get('fetch_error') or not ident or not url or not view.get('available') or not view.get('text', '').strip():
        error=record.get('fetch_error')
        status=re.search(r'(?:HTTP|status|code)[^0-9]{0,8}([45][0-9]{2})',str(error),re.I) if error else None
        detail=('获取失败：HTTP '+status.group(1)) if status else ('获取失败：'+re.sub(r'https?://[^\s]+','[source URL]',str(error))[:200]) if error else view.get('reason','来源标识、URL或原始正文不完整')
        raise EditorialError('source_unavailable', '无法完成来源核验：'+detail, 'source', [ident or Path(path).name])
    return {'source_id': ident, 'url': url, 'body': view['text'], 'root': view['root'],
            'aggregate_date': entry.get('pub_date'),
            'source_file': str(Path(path).resolve()), 'source_sha256': hashlib.sha256(Path(path).read_bytes()).hexdigest(),
            'body_sha256': hashlib.sha256(view['text'].encode()).hexdigest()}


def units(artifact):
    rows = []
    sections = artifact.get('sections')
    if isinstance(sections, list) or isinstance(artifact.get('stories'), list):
        for i, section in enumerate(sections if isinstance(sections, list) else [artifact]):
            for j, story in enumerate(section.get('stories', [])):
                prefix = f'/sections/{i}/stories/{j}' if sections is not None else f'/stories/{j}'
                for field in ('script', 'audio_script'):
                    rows.append({'unit': prefix + '/' + field, 'text': story.get(field, story.get('script', '')),
                                 'source_urls': story.get('source_urls', []), 'tier': story.get('tier'), 'event_key': story.get('event_key')})
            prefix = f'/sections/{i}' if sections is not None else ''
            urls = sorted({u for story in section.get('stories', []) for u in story.get('source_urls', [])})
            for field in ('script','audio_script'):
                rows.append({'unit':prefix+'/'+field,'text':section.get(field,section.get('script','')),'source_urls':urls,'tier':'section'})
    else:
        for field in ('reader_markdown', 'compressed_markdown', 'digest_markdown', 'audio_script'):
            if field in artifact:
                rows.append({'unit': '/' + field, 'text': artifact[field],
                             'source_urls': [c.get('source_url') for c in artifact.get('editorial', {}).get('claims', []) if isinstance(c, dict)],
                             'tier': 'reading' if 'compressed_markdown' in artifact else 'weekly'})
    return rows


def prepare_inputs(artifact_path, source_paths, out):
    root = run_root(artifact_path)
    artifact = read(artifact_path)
    sources = []
    for value in source_paths:
        path = bounded_path(root, value)
        source = source_input(path)
        snapshot = root / '.editorial' / 'sources' / (source['body_sha256'] + '.json')
        if not snapshot.exists():
            save(snapshot, source)
        source['snapshot'] = str(snapshot)
        source.pop('body')
        sources.append(source)
    needed = {url for row in units(artifact) for url in row['source_urls']}
    if not needed <= {s['url'] for s in sources}:
        raise EditorialError('source_unavailable', '无法完成来源核验：存在没有完整来源快照的文稿引用', 'source')
    packet = {'artifact_sha256': digest(artifact), 'artifact_file': str(Path(artifact_path).resolve()),
              'sources': sources, 'units': units(artifact), 'dimensions': sorted(DIMENSIONS),
              'instruction': 'Read every full source snapshot body and final unit. Return concrete source/draft differences and a pass/revise/reject/unverifiable conclusion. Program acceptance only binds evidence; it does not prove semantic truth.'}
    save(out, packet)
    return {'input_file': str(Path(out).resolve()), 'input_sha256': digest(packet), 'units': len(packet['units']), 'sources': len(sources)}


def verify_comparison(artifact, review, review_path):
    root = run_root(review_path)
    packet_path = bounded_path(root, review.get('input_file', ''))
    if not packet_path.is_file():
        raise EditorialError('review_inputs_missing', 'Final review must reference prepared full sources and manuscript inputs')
    packet = read(packet_path)
    if review.get('input_sha256') != digest(packet) or packet.get('artifact_sha256') != digest(artifact) or packet.get('units') != units(artifact):
        raise EditorialError('stale_source_review', 'Source/manuscript input changed; re-prepare inputs and repeat semantic review')
    sources = {}
    for source in packet.get('sources', []):
        current = source_input(bounded_path(root, source['source_file']))
        if current['source_sha256'] != source.get('source_sha256') or current['body_sha256'] != source.get('body_sha256'):
            raise EditorialError('stale_source_review', 'Source snapshot changed after review', source_ids=[current['source_id']])
        snapshot = read(bounded_path(root, source['snapshot']))
        if hashlib.sha256(snapshot.get('body','').encode()).hexdigest() != current['body_sha256']:
            raise EditorialError('stale_source_review', 'Full source input given to the reviewer changed', source_ids=[current['source_id']])
        sources[current['source_id']] = current
    expected = {row['unit']: row for row in units(artifact)}
    for role in review.get('reviews', []):
        comparisons = role.get('comparisons')
        if not isinstance(comparisons, list) or len(comparisons) != len(expected) or {c.get('unit') for c in comparisons if isinstance(c, dict)} != set(expected):
            raise EditorialError('comparison_missing', 'Each reviewer must compare every display/compressed/spoken unit with full original inputs')
        for row in comparisons:
            unit = expected[row['unit']]
            ids = row.get('source_ids', [])
            if not ids or any(i not in sources for i in ids) or not set(unit['source_urls']) <= {sources[i]['url'] for i in ids}:
                raise EditorialError('comparison_source_missing', 'Comparison lacks the actual sources for this manuscript unit', unit=row['unit'])
            if set(row.get('checked_dimensions', [])) != DIMENSIONS:
                raise EditorialError('comparison_incomplete', 'Review must check actors, events, dates, numbers, limits, meaning, omissions, additions and compression', unit=row['unit'])
            conclusion = row.get('conclusion')
            if conclusion not in ('pass', 'revise', 'reject', 'unverifiable'):
                raise EditorialError('comparison_incomplete', 'Review conclusion must be pass/revise/reject/unverifiable', unit=row['unit'])
            if conclusion != 'pass':
                differences = row.get('differences')
                if not isinstance(differences, list) or not differences or any(not isinstance(d, dict) or not d.get('kind') or not d.get('reason') for d in differences):
                    raise EditorialError('difference_missing', 'A non-passing review must state concrete differences, not an empty conclusion', source_ids=ids, unit=row['unit'])
                reason = '; '.join(f"{d['kind']}: {d['reason']}" for d in differences)
                raise EditorialError('semantic_' + str(conclusion), reason, source_ids=ids, unit=row['unit'])
            if row.get('differences') != [] or not row.get('assessment'):
                raise EditorialError('comparison_incomplete', 'Passing comparison requires a concrete assessment and no unresolved material differences', unit=row['unit'])
            anchors = row.get('evidence')
            if not isinstance(anchors, list) or not anchors:
                raise EditorialError('comparison_evidence_missing', 'Comparison must quote actual source and final manuscript passages', unit=row['unit'])
            compact = lambda s: ''.join(str(s).split())
            for anchor in anchors:
                ident = anchor.get('source_id')
                original, final = anchor.get('source_excerpt'), anchor.get('draft_excerpt')
                if ident not in ids or not original or not final or compact(original) not in compact(sources[ident]['body']) or compact(final) not in compact(unit['text']) or not anchor.get('observation'):
                    raise EditorialError('comparison_evidence_mismatch', 'Source/final excerpts do not match the actual inputs', source_ids=ids, unit=row['unit'])
    return packet


def issues(root):
    path = Path(root) / 'editorial-issues.json'
    return read(path) if path.exists() else {'version': 1, 'issues': []}


def report_issues(root):
    data = issues(root)
    return [{'id': i['id'], 'stage': i['stage'], 'reason': i['reason'], 'disposition': i['disposition'],
             'artifact': i.get('artifact'), 'source_ids': i.get('source_ids', [])} for i in data['issues']]


def write_issue_report(root):
    rows = report_issues(root)
    if not rows:
        return
    path = Path(root) / 'run-report.md'
    existing = path.read_text() if path.exists() else '# FreshLoop 编辑运行\n\n**编辑存在异常，尚未完成交付核验。**\n'
    marker = '<!-- editorial-issues -->'
    existing = existing.split(marker, 1)[0].rstrip()
    lines = [marker, '异常文稿与来源核验：']
    for row in rows:
        lines.append(f"- {row['artifact'] or ','.join(row['source_ids']) or row['id']}／{row['stage']}：{row['reason']}；处置：{row['disposition']}。")
    path.write_text(existing + '\n\n' + '\n'.join(lines) + '\n')
    path.chmod(0o600)


@contextlib.contextmanager
def issue_lock(root):
    directory=Path(root)/'.editorial'; directory.mkdir(parents=True,exist_ok=True)
    with os.fdopen(os.open(directory/'issues.lock',os.O_RDWR|os.O_CREAT,0o600),'w') as handle:
        fcntl.flock(handle,fcntl.LOCK_EX)
        yield


def record_issue(args, exc):
    reference=next((getattr(args,k,None) for k in ('artifact','state','out','manifest') if getattr(args,k,None)),None)
    if not reference: return None
    with issue_lock(run_root(reference)):
        return record_issue_locked(args,exc)


def record_issue_locked(args, exc):
    reference = next((getattr(args, key, None) for key in ('artifact', 'state', 'out', 'manifest') if getattr(args, key, None)), None)
    if not reference:
        return None
    root = run_root(reference)
    data = issues(root)
    reason = str(exc)
    for name in ('NEXUS_KEY', 'CORTEX_API_KEY'):
        secret = os.environ.get(name)
        if secret:
            reason = reason.replace(secret, '[redacted]') if len(secret) >= 8 else re.sub(r'(?<!\w)'+re.escape(secret)+r'(?!\w)', '[redacted]', reason)
    state_path = getattr(args, 'state', None)
    state, state_usable = {}, True
    if state_path and Path(state_path).exists():
        try: state=read(state_path)
        except (ValueError,OSError): state_usable=False
        if not isinstance(state,dict): state={};state_usable=False
    if state.get('lease_token'):
        token = state['lease_token']
        reason = reason.replace(token, '[redacted]') if len(token) >= 8 else re.sub(r'(?<!\w)'+re.escape(token)+r'(?!\w)', '[redacted]', reason)
    artifact_path = getattr(args, 'artifact', None)
    artifact_sha = None
    if artifact_path and Path(artifact_path).is_file():
        raw=Path(artifact_path).read_bytes()
        try:
            artifact=json.loads(raw); artifact_sha=digest(artifact)
        except (ValueError,UnicodeError):
            artifact_sha=hashlib.sha256(raw).hexdigest()
            artifact={'unparsed_draft':True,'raw_text':raw.decode(errors='replace')}
        archive = root / '.editorial' / 'artifacts' / (artifact_sha + '.json')
        if not archive.exists(): save(archive, artifact)
    code = getattr(exc, 'code', type(exc).__name__)
    stage = getattr(exc, 'stage', getattr(args, 'command', 'editorial'))
    if artifact_path and isinstance(exc,(json.JSONDecodeError,UnicodeError)): stage='validation'
    source_ids = getattr(exc, 'source_ids', []) or ([args.id] if getattr(args, 'command', None) == 'source' else [])
    identity = digest({'job': state.get('job_id'), 'artifact': artifact_path, 'sha': artifact_sha, 'stage': stage, 'code': code, 'unit': getattr(exc, 'unit', None), 'sources': source_ids, 'reason': reason})
    row = next((r for r in data['issues'] if r['id'] == identity), None)
    if row is None:
        row = {'id': identity, 'stage': stage, 'code': code, 'reason': reason, 'artifact': Path(artifact_path).name if artifact_path else None,
               'artifact_sha256': artifact_sha, 'job_id': state.get('job_id'), 'source_ids': source_ids,
               'unit': getattr(exc, 'unit', None), 'disposition': 'blocked', 'first_seen': int(time.time())}
        data['issues'].append(row)
    row['last_seen'] = int(time.time())
    if state_path and state_usable and artifact_sha and stage in ('validation', 'review'):
        attempted = set(state.get('editorial_drafts', [])); attempted.add(artifact_sha)
        state['editorial_drafts'] = sorted(attempted)
        if len(attempted) >= MAX_DRAFTS:
            state['editorial_status'] = 'failed'
            row['disposition'] = 'revision_limit_reached'
        save(state_path, state)
    save(root / 'editorial-issues.json', data); write_issue_report(root)
    return {'editorial_error': code, 'stage': stage, 'reason': reason, 'disposition': row['disposition'],
            'issue_id': identity, 'issues_file': str(root / 'editorial-issues.json')}


def approved(artifact, review_path, state):
    with issue_lock(run_root(review_path)):
        return approved_locked(artifact,review_path,state)


def approved_locked(artifact, review_path, state):
    root = run_root(review_path); sha = digest(artifact)
    archive = root / '.editorial' / 'artifacts' / (sha + '.json')
    if not archive.exists(): save(archive, artifact)
    save(root / '.editorial' / 'reviews' / (sha + '.json'), read(review_path))
    data = issues(root)
    packet = read(bounded_path(root, read(review_path)['input_file']))
    reviewed_sources = {s['source_id'] for s in packet['sources']}
    for row in data['issues']:
        if row['stage'] == 'source' and set(row['source_ids']) <= reviewed_sources and row['source_ids']:
            row['disposition'] = 'source_retrieved_and_reviewed'; row['resolved_by_sha256'] = sha
        if row['stage'] in ('validation', 'review') and row.get('job_id') == state.get('job_id') and row.get('job_id'):
            row['disposition'] = 'corrected_and_reviewed'; row['resolved_by_sha256'] = sha
    if data['issues']:
        save(root / 'editorial-issues.json', data); write_issue_report(root)


def source_recovered(args, record):
    with issue_lock(run_root(args.out)):
        return source_recovered_locked(args,record)


def source_recovered_locked(args, record):
    root = run_root(args.out); data = issues(root)
    for row in data['issues']:
        if row['stage'] == 'source' and args.id in row['source_ids'] and row['disposition'] == 'blocked':
            row['disposition'] = 'source_retrieved_requires_semantic_review'
    if data['issues']:
        save(root / 'editorial-issues.json', data); write_issue_report(root)


def submission_reconciled(state, reference):
    if state.get('status') != 'completed': return
    root=run_root(reference)
    with issue_lock(root):
        data=issues(root)
        for row in data['issues']:
            if row['stage'] == 'submit' and row.get('job_id') == state.get('job_id'):
                row['disposition']='submission_confirmed'; row['result_ref']=state.get('result_ref')
        if data['issues']:
            save(root/'editorial-issues.json',data);write_issue_report(root)


def unresolved(rows):
    return any(row['disposition'] in ('blocked', 'revision_limit_reached', 'source_retrieved_requires_semantic_review') for row in rows)
