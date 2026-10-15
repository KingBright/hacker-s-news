"""Import actual Antigravity reviewer outputs/read coverage. Never invent opinions."""
import contextlib
import hashlib
import json
from pathlib import Path
import re
import sqlite3
import uuid


PROFILES={'facts':'freshloop-fact-auditor','listening':'freshloop-listening-editor'}


def state_root():
    return Path.home()/'.gemini/antigravity'


def parse_receipt(text):
    candidates=[text.strip()]
    candidates += re.findall(r'```json\s*(.*?)\s*```',text,re.S)
    for value in candidates:
        try:result=json.loads(value)
        except (ValueError,TypeError):continue
        if isinstance(result,dict) and result.get('role') in PROFILES:return result
    return None


def native_result(conversation, role, batch_file, retain_raw=None):
    from review_comparison import EditorialError, digest, read, bounded_path, run_root, source_input
    uuid.UUID(conversation)
    base=state_root();db=base/'conversation_summaries.db'
    with contextlib.closing(sqlite3.connect(db.as_uri()+'?mode=ro',uri=True)) as conn:
        row=conn.execute('SELECT agent_name,parent_conversation_id FROM conversation_summaries WHERE conversation_id=?',(conversation,)).fetchone()
    if not row or row[0]!=PROFILES[role] or not row[1]:
        raise EditorialError('review_origin_role_mismatch','Receipt must originate from the actual independent reviewer profile, not the editor','review')
    batch=read(batch_file);wanted={u['unit'] for u in batch['units']}
    path=base/'brain'/conversation/'.system_generated/logs/transcript_full.jsonl'
    rows=[json.loads(line) for line in path.read_text().splitlines() if line.strip()]
    selected=None;coverage={}
    for index,step in enumerate(rows):
        for call in step.get('tool_calls',[]):
            if call.get('name')=='view_file':
                args=call.get('args',{});file=args.get('AbsolutePath')
                next_row=rows[index+1] if index+1<len(rows) else {}
                # Native JSONL has no call IDs: accept only an unambiguous single
                # read followed by the platform's completed generic tool result.
                output=next_row.get('content','') if len(step.get('tool_calls',[]))==1 and next_row.get('type')=='GENERIC' and next_row.get('status')=='DONE' else ''
                match=re.search(r'Showing lines (\d+) to (\d+)',output)
                if file and match and 'Content truncated' not in output and 'Encountered error' not in output:
                    coverage.setdefault(str(Path(file).resolve()),[]).append(tuple(map(int,match.groups())))
            if call.get('name')=='send_message' and call.get('args',{}).get('Recipient')==row[1]:
                raw=parse_receipt(call.get('args',{}).get('Message',''))
                if raw and raw.get('role')==role and all(raw.get(k)==batch.get(k) for k in ('input_file','input_sha256','artifact_sha256')) and len(raw.get('comparisons',[]))==len(wanted) and {c.get('unit') for c in raw.get('comparisons',[])}==wanted:
                    selected=(raw,step['step_index'],'send_message',{k:list(v) for k,v in coverage.items()})
        if step.get('type')=='PLANNER_RESPONSE' and step.get('content'):
            raw=parse_receipt(step['content'])
            if raw and raw.get('role')==role and all(raw.get(k)==batch.get(k) for k in ('input_file','input_sha256','artifact_sha256')) and len(raw.get('comparisons',[]))==len(wanted) and {c.get('unit') for c in raw.get('comparisons',[])}==wanted:
                selected=(raw,step['step_index'],'response',{k:list(v) for k,v in coverage.items()})
    if selected is None:
        raise EditorialError('review_native_response_missing','No actual reviewer JSON response matches this batch and input binding','review')
    raw,step,kind,seen=selected
    if retain_raw is not None:
        from review_comparison import save
        save(retain_raw,{'provider':'antigravity','conversation_id':conversation,'role':role,'origin_verified':False,'actual_raw_response':raw})
    root=run_root(batch['input_file']);packet=read(bounded_path(root,batch['input_file']))
    if digest(packet)!=batch['input_sha256']:
        raise EditorialError('stale_review_receipt','Review batch no longer matches its original source/manuscript packet','review')
    batch_path=bounded_path(root,batch_file)
    batch_lines=len(batch_path.read_text().splitlines())
    batch_seen={n for start,end in seen.get(str(batch_path),[]) for n in range(start,end+1)}
    if not set(range(1,batch_lines+1))<=batch_seen:
        raise EditorialError('review_batch_unread','Actual reviewer did not read the complete bounded batch metadata','review')
    expected_units={u['unit']:u for u in packet['units']}
    required_urls=set()
    for descriptor in batch['units']:
        canonical=expected_units.get(descriptor['unit'])
        if canonical is None or {k:v for k,v in descriptor.items() if k!='readable'}!={k:v for k,v in canonical.items() if k!='text'}:
            raise EditorialError('review_batch_changed','Batch units differ from the complete bound packet','review')
        required_urls.update(canonical['source_urls'])
    canonical_sources={s['url']:s for s in packet['sources']}
    if {s['url'] for s in batch['sources']}!=required_urls or len(batch['sources'])!=len(required_urls) or batch.get('program_context',{})!=packet.get('program_context',{}):
        raise EditorialError('review_batch_changed','Batch must retain every source and context required by its final units','review')
    for descriptor in batch['sources']:
        if {k:v for k,v in descriptor.items() if k!='readable'}!=canonical_sources[descriptor['url']]:
            raise EditorialError('review_batch_changed','Batch source descriptor differs from its bound original','review')
        current = source_input(bounded_path(root, descriptor['source_file']))
        if any(current[key] != descriptor[key] for key in ('source_sha256', 'body_sha256')):
            raise EditorialError('stale_source_review', 'Source snapshot changed after review', source_ids=[current['source_id']])
    originals={s['source_id']:read(bounded_path(root,s['snapshot']))['body'] for s in packet['sources']}
    unit_text={u['unit']:u['text'] for u in packet['units']}
    for descriptor in batch['sources']+batch['units']:
        view=descriptor['readable'];file=bounded_path(root,view['file']);text=file.read_text()
        original=originals[descriptor['source_id']] if 'source_id' in descriptor else unit_text[descriptor['unit']]
        if hashlib.sha256(text.encode()).hexdigest()!=view['view_sha256'] or ''.join(text.split())!=''.join(original.split()):
            raise EditorialError('review_readable_view_changed','Readable input differs from the complete bound original','review')
        intervals=seen.get(str(file),[])
        covered=set(n for start,end in intervals for n in range(start,end+1))
        actual_lines=len(text.splitlines())
        if view['lines']!=actual_lines or not set(range(1,actual_lines+1))<=covered:
            raise EditorialError('review_source_unread','Actual native reads did not cover every line of a required original or final manuscript','review')
    return raw,{'provider':'antigravity','role':role,'conversation_id':conversation,'parent_conversation_id':row[1],
                'step_index':step,'response_kind':kind,'batch_file':str(Path(batch_file).resolve()),
                'batch_sha256':hashlib.sha256(Path(batch_file).read_bytes()).hexdigest(),'receipt_sha256':digest(raw),
                'verified_read_files':sorted(seen)}


def import_receipt(conversation,role,batch_file,out):
    from review_comparison import save
    raw,origin=native_result(conversation,role,batch_file,Path(out).with_suffix('.unverified-native.json'))
    save(out,raw);origin_file=Path(out).with_suffix('.origin.json');save(origin_file,origin)
    return {'receipt_file':str(Path(out).resolve()),'origin_file':str(origin_file.resolve()),'native_conversation':conversation,'role':role}


def require_origins(review,reference):
    from review_comparison import EditorialError, read, bounded_path, run_root
    root=run_root(reference);authors={'facts':set(),'listening':set()}
    for item in review.get('receipts',[]):
        if not item.get('origin_file'):
            raise EditorialError('review_origin_missing','Production must import actual reviewer outputs/read evidence; editor-written JSON is not an independent receipt','review')
        origin=read(bounded_path(root,item['origin_file']))
        batch_file=bounded_path(root,origin['batch_file'])
        if hashlib.sha256(batch_file.read_bytes()).hexdigest()!=origin['batch_sha256']:
            raise EditorialError('stale_review_receipt','Native review batch changed after receipt import','review')
        actual,verified=native_result(origin['conversation_id'],item['role'],batch_file)
        if actual!=read(bounded_path(root,item['file'])) or verified!=origin:
            raise EditorialError('review_origin_changed','Saved receipt differs from its actual native author/output/read evidence','review')
        authors[item['role']].add(origin['conversation_id'])
    if not authors['facts'] or not authors['listening'] or authors['facts']&authors['listening']:
        raise EditorialError('review_origin_role_mismatch','Facts and listening must come from distinct actual reviewer conversations','review')
