"""Readable, bounded editorial inputs; preserves originals and never writes verdicts."""
import hashlib
import json
from pathlib import Path
import textwrap


def sha(text):
    return hashlib.sha256(text.encode()).hexdigest()


def write_readable(text, directory, label):
    # Added wrapping whitespace is a view only. Source/storage content is untouched.
    lines = []
    for paragraph in text.splitlines():
        lines.extend(textwrap.wrap(paragraph, width=400, replace_whitespace=False,
                                   drop_whitespace=False) or [''])
    view = '\n'.join(lines) + '\n'
    if ''.join(view.split()) != ''.join(text.split()):
        raise ValueError('readable view changed source characters')
    path = directory / (label + '-' + sha(text) + '.txt')
    path.write_text(view); path.chmod(0o600)
    ranges, start, size = [], 1, 0
    for number, line in enumerate(lines, 1):
        count = len((line + '\n').encode())
        if size and (size + count > 24000 or number - start >= 80):
            ranges.append(dict(start_line=start, end_line=number - 1, bytes=size))
            start, size = number, 0
        size += count
    ranges.append(dict(start_line=start, end_line=len(lines), bytes=size))
    plans = []
    for offset in range(0, len(ranges), 20):
        plan = path.with_name(path.stem + '-reads-' + str(offset // 20 + 1) + '.json')
        plan.write_text(json.dumps({'file': str(path.resolve()), 'view_sha256': sha(view),
                                    'ranges': ranges[offset:offset + 20]}, indent=2))
        plan.chmod(0o600); plans.append(str(plan.resolve()))
    return dict(file=str(path.resolve()), text_sha256=sha(text), view_sha256=sha(view),
                lines=len(lines), read_plan_files=plans)


def prepare_batches(packet, request, directory):
    """Each small packet supplies direct paths to full readable originals/units."""
    directory = Path(directory); directory.mkdir(parents=True, exist_ok=True)
    readable = directory / 'readable'; readable.mkdir(exist_ok=True)
    sources = {}
    for source in packet['sources']:
        snapshot = json.loads(Path(source['snapshot']).read_text())
        body = snapshot['body']
        if sha(body) != source['body_sha256']:
            raise ValueError('source snapshot differs before readable packaging')
        sources[source['url']] = dict(source, readable=write_readable(body, readable, 'source-' + source['source_id']))
    unit_views = []
    for unit in packet['units']:
        unit_views.append(dict({k:v for k,v in unit.items() if k != 'text'},
                               readable=write_readable(unit['text'], readable, 'unit')))
    context = packet.get('program_context', {})
    batches = []
    for offset in range(0, len(unit_views), 4):
        selected = unit_views[offset:offset + 4]
        urls = {url for unit in selected for url in unit['source_urls']}
        ident = str(offset // 4 + 1).zfill(3)
        batch = {'batch_id': ident, **request, 'units': selected,
                 'sources': [sources[url] for url in sorted(urls)],
                 'program_context': context,
                 'instruction': 'Read every listed source and final unit using all read-plan ranges. Return only this batch\'s actual partial JSON receipt to the editor; do not write files, use shell/task tools, search unrelated history, or generate a verdict template.'}
        payload = json.dumps(batch, ensure_ascii=False, indent=2)
        if len(payload.encode()) > 32000 or len(payload.splitlines()) > 700:
            raise ValueError('review batch metadata exceeds reader bound; retain inputs')
        path = directory / ('batch-' + ident + '.json')
        path.write_text(payload); path.chmod(0o600)
        batches.append(dict(batch_id=ident, file=str(path.resolve()),
                            units=[unit['unit'] for unit in selected]))
    index = {'artifact_sha256': request['artifact_sha256'], 'input_file': request['input_file'],
             'input_sha256': request['input_sha256'], 'batches': batches,
             'instruction': 'Use the same two original independent reviewers. Send one listed batch at a time; save each actual returned JSON unchanged. Assemble all parts only after every batch has a receipt from each role.'}
    path = directory / 'index.json'
    path.write_text(json.dumps(index, ensure_ascii=False, indent=2)); path.chmod(0o600)
    return str(path.resolve())


def merge_parts(parts, role):
    """Deterministic concatenation of actual raw receipts, never filling opinions."""
    if not parts or any(not isinstance(p, dict) or p.get('role') != role for p in parts):
        raise ValueError('parts must belong to their actual reviewer role')
    if len(parts) == 1:
        return parts[0]
    binding = ('artifact_sha256', 'input_file', 'input_sha256')
    if any(any(p.get(k) != parts[0].get(k) for k in binding) for p in parts):
        raise ValueError('review parts belong to different input/manuscript bindings')
    if any(not isinstance(p.get('comparisons'), list) or
           not isinstance(p.get('blocking'), list) or not isinstance(p.get('warnings'), list)
           for p in parts):
        raise ValueError('partial receipts require actual lists, including blocking/warnings')
    combined = {'role': role, **{k:parts[0][k] for k in binding}}
    for key in ('comparisons', 'blocking', 'warnings', 'checked_artifacts'):
        combined[key] = [value for p in parts for value in p.get(key, [])]
    combined['summary'] = '\n'.join(p.get('summary', '') for p in parts)
    return combined
