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


def prepare_inputs(artifact_path, source_paths, out, ledger_path=None):
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
        raise EditorialError('source_unavailable', '无法完成来源核验：存在没有完整来源快照的文稿引用', 'source', sorted(needed - {s['url'] for s in sources}))
    packet = {'artifact_sha256': digest(artifact), 'artifact_file': str(Path(artifact_path).resolve()),
              'sources': sources, 'units': units(artifact), 'dimensions': sorted(DIMENSIONS),
              'instruction': 'Read every full source snapshot body and final unit. Return concrete source/draft differences and a pass/revise/reject/unverifiable conclusion. Program acceptance only binds evidence; it does not prove semantic truth.'}
    packet['program_context'] = {k:artifact[k] for k in ('title', 'opening', 'closing') if k in artifact}
    if ledger_path:
        from editorial_selection import selection
        packet['selection'] = selection(artifact, ledger_path, artifact_path)
    elif 'agent-runs' in Path(artifact_path).resolve().parts and ('sections' in artifact or 'stories' in artifact):
        raise EditorialError('selection_ledger_missing', 'Prepare the reconciled candidate ledger before Radio review', 'selection')
    save(out, packet)
    request = {'input_file': str(Path(out).resolve()), 'input_sha256': digest(packet),
               'artifact_sha256': digest(artifact), 'units': len(packet['units']), 'sources': len(sources)}
    from review_batches import prepare_batches
    request['review_batches'] = prepare_batches(packet, request, root / '.editorial' / 'review-batches' / digest(packet))
    request_path = Path(out).with_suffix('.request.json')
    save(request_path, request)
    return dict(request, request_file=str(request_path.resolve()))


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
    if packet.get('selection'):
        from editorial_selection import selection
        current = selection(artifact, packet['selection']['file'], review_path)
        if current != packet['selection']:
            raise EditorialError('stale_selection_review', 'Candidate/depth ledger changed after source review', 'selection')
    elif 'agent-runs' in Path(review_path).resolve().parts and ('sections' in artifact or 'stories' in artifact):
        raise EditorialError('selection_ledger_missing', 'Production Radio review must bind the complete candidate ledger', 'selection')
    verify_receipts(review, root)
    expected = {row['unit']: row for row in units(artifact)}
    context = {k:artifact[k] for k in ('title', 'opening', 'closing') if k in artifact}
    if packet.get('program_context', context) != context:
        raise EditorialError('stale_source_review', 'Program opening/closing context differs from the final artifact')
    def without_labels(value):
        text = re.sub(r'(?:/sections/\d+)?(?:/stories/\d+)?/(?:script|audio_script)', '', str(value))
        text = re.sub(r'\b[0-9a-f]{8,64}\b', '', text, flags=re.I)
        return ''.join(c.casefold() for c in text if c.isalnum())
    for role in review.get('reviews', []):
        comparisons = role.get('comparisons')
        if not isinstance(comparisons, list) or len(comparisons) != len(expected) or {c.get('unit') for c in comparisons if isinstance(c, dict)} != set(expected):
            raise EditorialError('comparison_missing', 'Each reviewer must compare every display/compressed/spoken unit with full original inputs')
        events = {u.get('event_key') for u in expected.values() if u.get('event_key')}
        if len(events) > 1 and all(c.get('conclusion') == 'pass' for c in comparisons):
            assessments = {without_labels(c.get('assessment')) for c in comparisons}
            observations = {without_labels(a.get('observation')) for c in comparisons for a in c.get('evidence', [])}
            if len(assessments) == 1 or len(observations) == 1:
                raise EditorialError('comparison_template', 'Verdict templates repeated across distinct events do not establish source/draft correspondence')
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
            if row.get('differences') != [] or generic_verdict(row.get('assessment')):
                raise EditorialError('comparison_incomplete', 'Passing comparison requires a concrete assessment and no unresolved material differences', unit=row['unit'])
            anchors = row.get('evidence')
            if not isinstance(anchors, list) or not anchors:
                raise EditorialError('comparison_evidence_missing', 'Comparison must quote actual source and final manuscript passages', unit=row['unit'])
            if {a.get('source_id') for a in anchors} != set(ids):
                raise EditorialError('comparison_evidence_missing', 'Quote the actual contribution of every source for this unit', unit=row['unit'])
            for anchor in anchors:
                ident = anchor.get('source_id')
                original, final = anchor.get('source_excerpt'), anchor.get('draft_excerpt')
                if ident not in ids or not passage_match(sources[ident]['body'], original) or not passage_match(unit['text'], final) or generic_verdict(anchor.get('observation')):
                    raise EditorialError('comparison_evidence_mismatch', 'Source/final excerpts do not match the actual inputs', source_ids=ids, unit=row['unit'])
    return packet


def generic_verdict(value):
    if not isinstance(value, str) or not value.strip():
        return True
    normalized = ''.join(c.casefold() for c in value if c.isalnum())
    return normalized in {'pass', '通过', '通过审核', '事实无误', '听感无误', '事实完全匹配无出入', '无差异', '准确', '正确', '已核实'}


def passage_match(text, excerpt):
    """Require a complete sentence/paragraph anchor; no length/word quota."""
    if not isinstance(excerpt, str) or not excerpt.strip():
        return False
    clean, starts, ends = [], {0}, {0}
    for char in text:
        if char in '\r\n':
            starts.add(len(clean)); ends.add(len(clean))
        if char.isspace():
            continue
        if char in '。！？!?；;.':
            ends.add(len(clean))
        clean.append(char)
        if char in '。！？!?；;.':
            starts.add(len(clean)); ends.add(len(clean))
    body = ''.join(clean); quote = ''.join(excerpt.split())
    ends.add(len(body))
    offset = body.find(quote)
    while offset >= 0:
        if offset in starts and offset + len(quote) in ends:
            return True
        offset = body.find(quote, offset + 1)
    return False


def verify_receipts(review, root):
    from review_batches import merge_parts
    receipts = review.get('receipts', [])
    rows = review.get('reviews', [])
    if not isinstance(receipts, list) or len(receipts) < 2 or {r.get('role') for r in receipts} != {'facts', 'listening'}:
        raise EditorialError('review_receipt_missing', 'Preserve actual receipts from both reviewers; partial responses must not be filled by the editor')
    paths, parts = set(), {'facts': [], 'listening': []}
    for item in receipts:
        path = bounded_path(root, item.get('file', ''))
        if path in paths:
            raise EditorialError('review_receipt_copied', 'Facts/listening must retain separate actual receipts')
        paths.add(path); original = read(path)
        if digest(original) != item.get('sha256') or original.get('role') != item['role']:
            raise EditorialError('review_receipt_changed', 'Raw reviewer receipt changed after assembly')
        if any(original.get(k) != review.get(k) for k in ('artifact_sha256', 'input_file', 'input_sha256')):
            raise EditorialError('stale_review_receipt', 'Reviewer receipt belongs to another original input/final manuscript')
        parts[item['role']].append(original)
    merged = []
    for role in ('facts', 'listening'):
        try: actual = merge_parts(parts[role], role)
        except ValueError as exc: raise EditorialError('review_receipt_changed', str(exc)) from None
        row = next((r for r in rows if r.get('role') == role), None)
        if actual != row:
            raise EditorialError('review_receipt_changed', 'Assembled review differs from the actual raw parts')
        merged.append(actual.get('comparisons'))
    if review.get('mode') == 'native_subagents' and merged[0] == merged[1]:
        raise EditorialError('review_receipt_copied', 'Identical comparisons copied to both roles are not independent review results')


def assemble_review(artifact_path, input_path, facts_path, listening_path, out, mode='native_subagents', unavailable_reason=None):
    from review_batches import merge_parts
    root = run_root(artifact_path); packet_path = bounded_path(root, input_path)
    packet = read(packet_path); rows, receipts = [], []
    for role, given in (('facts', facts_path), ('listening', listening_path)):
        parts = []
        for value in given if isinstance(given, (list, tuple)) else [given]:
            path = bounded_path(root, value); row = read(path)
            if row.get('role') != role:
                raise EditorialError('review_role_missing', 'Reviewer receipt must name its actual facts/listening role')
            parts.append(row); item={'role': role, 'file': str(path), 'sha256': digest(row)}
            origin=path.with_suffix('.origin.json')
            if origin.exists(): item['origin_file']=str(origin)
            receipts.append(item)
        try: rows.append(merge_parts(parts, role))
        except ValueError as exc: raise EditorialError('review_receipt_changed', str(exc)) from None
    result = {'artifact_sha256': packet['artifact_sha256'], 'input_file': str(packet_path),
              'input_sha256': digest(packet), 'mode': mode, 'reviews': rows, 'receipts': receipts}
    if unavailable_reason: result['unavailable_reason'] = unavailable_reason
    save(out, result)  # preserve rejection and every raw part; never fill/change verdicts
    verify_comparison(read(artifact_path), result, out)
    return {'review_file': str(Path(out).resolve()), 'review_sha256': digest(result),
            'semantic_truth_verified_by_program': False}


def validate_issue_transition(previous, event):
    if event.get('action') not in {'error_recorded', 'review_approved', 'source_retrieved', 'submission_confirmed'}:
        raise ValueError('undefined issue transition')
    current = event.get('data', {}).get('issues', [])
    before = {row['id']: row for row in previous.get('issues', [])}
    after = {row['id']: row for row in current}
    # There is no verified refresh-result reconciliation command yet. Taking a
    # source inventory or publishing a draft cannot establish refresh completion.
    for ident, row in before.items():
        if row.get('stage') == 'refresh':
            newer = after.get(ident)
            if newer is None or {k:v for k,v in row.items() if k != 'last_seen'} != {k:v for k,v in newer.items() if k != 'last_seen'}:
                raise ValueError('refresh issue changed without verified refresh evidence')
    for ident, row in after.items():
        if ident not in before and row.get('stage') == 'refresh' and row.get('disposition') not in ('blocked', 'revision_limit_reached'):
            raise ValueError('refresh issue cannot start resolved')


def issue_events(root):
    path = Path(root) / '.editorial' / 'issues.events.jsonl'
    events, previous = [], None
    if path.exists():
        try:
            for line in path.read_text().splitlines():
                event = json.loads(line)
                hashed = {k: v for k, v in event.items() if k != 'sha256'}
                if event.get('previous') != previous or digest(hashed) != event.get('sha256'):
                    raise ValueError('chain mismatch')
                validate_issue_transition(events[-1]['data'] if events else {'issues': []}, event)
                events.append(event); previous = event['sha256']
        except (ValueError, OSError):
            raise EditorialError('issue_audit_tampered', 'Editorial issue event history is invalid; retain it for inspection', 'audit') from None
    return events


def issues(root):
    path = Path(root) / 'editorial-issues.json'
    events = issue_events(root)
    if not events:
        if path.exists() and read(path).get('issues'):
            raise EditorialError('issue_audit_unverified', 'Legacy issue summary lacks original event history; its resolved flags are unverified', 'audit')
        return {'version': 2, 'issues': []}
    data = events[-1]['data']
    if not path.exists() or read(path) != data:
        raise EditorialError('issue_audit_tampered', 'Issue summary was edited outside verified helper transitions; do not treat rewritten resolved flags as acceptance', 'audit')
    return data


def persist_issues(root, data, action, evidence=None):
    events = issue_events(root)
    event = {'version': 1, 'previous': events[-1]['sha256'] if events else None,
             'at': int(time.time()), 'action': action, 'evidence': evidence or {}, 'data': data}
    try:
        validate_issue_transition(events[-1]['data'] if events else {'issues': []}, event)
    except ValueError:
        raise EditorialError('unverified_issue_transition', 'Issue resolution lacks a defined evidence-verified helper transition; preserve the unresolved record', 'audit') from None
    event['sha256'] = digest(event)
    path = Path(root) / '.editorial' / 'issues.events.jsonl'
    path.parent.mkdir(parents=True, exist_ok=True)
    with os.fdopen(os.open(path, os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o600), 'a') as handle:
        handle.write(json.dumps(event, ensure_ascii=False) + '\n')
        handle.flush(); os.fsync(handle.fileno())
    save(Path(root) / 'editorial-issues.json', data)


def report_issues(root):
    try:
        data = issues(root)
    except EditorialError as exc:
        return [{'id': exc.code, 'stage': 'audit', 'reason': str(exc), 'disposition': 'blocked', 'artifact': 'editorial-issues.json', 'source_ids': []}]
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
    persist_issues(root, data, 'error_recorded', {'issue_id': identity}); write_issue_report(root)
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
    reviewed_sources = {s['source_id'] for s in packet['sources']} | {s['url'] for s in packet['sources']}
    for row in data['issues']:
        if row['stage'] == 'source' and row.get('artifact') == Path(packet['artifact_file']).name and row.get('artifact_sha256'):
            row['disposition'] = 'corrected_and_reviewed'; row['resolved_by_sha256'] = sha
        elif row['stage'] == 'source' and set(row['source_ids']) <= reviewed_sources and row['source_ids']:
            row['disposition'] = 'source_retrieved_and_reviewed'; row['resolved_by_sha256'] = sha
        if row['stage'] in ('validation', 'review') and row.get('job_id') == state.get('job_id') and row.get('job_id'):
            row['disposition'] = 'corrected_and_reviewed'; row['resolved_by_sha256'] = sha
    if data['issues']:
        persist_issues(root, data, 'review_approved', {'artifact_sha256': sha, 'review_sha256': digest(read(review_path)), 'input_sha256': digest(packet)}); write_issue_report(root)


def source_recovered(args, record):
    with issue_lock(run_root(args.out)):
        return source_recovered_locked(args,record)


def source_recovered_locked(args, record):
    root = run_root(args.out); data = issues(root)
    for row in data['issues']:
        if row['stage'] == 'source' and args.id in row['source_ids'] and row['disposition'] == 'blocked':
            row['disposition'] = 'source_retrieved_requires_semantic_review'
    if data['issues']:
        persist_issues(root, data, 'source_retrieved', {'source_id': args.id, 'source_sha256': digest(record)}); write_issue_report(root)


def submission_reconciled(state, reference):
    if state.get('status') != 'completed': return
    root=run_root(reference)
    with issue_lock(root):
        data=issues(root)
        for row in data['issues']:
            if row['stage'] == 'submit' and row.get('job_id') == state.get('job_id'):
                row['disposition']='submission_confirmed'; row['result_ref']=state.get('result_ref')
        if data['issues']:
            persist_issues(root, data, 'submission_confirmed', {'job_id': state.get('job_id'), 'result_ref': state.get('result_ref')});write_issue_report(root)


def unresolved(rows):
    return any(row['disposition'] in ('blocked', 'revision_limit_reached', 'source_retrieved_requires_semantic_review') for row in rows)
