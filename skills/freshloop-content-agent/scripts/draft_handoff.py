"""Deterministic per-manuscript handoff. No model, publication or TTS calls."""
import hashlib
from pathlib import Path
from article_view import article_view
from review_comparison import EditorialError,digest,read,save,source_input,passage_match
from review_batches import write_readable

BUDGETS={'major':(180,480),'brief':(35,140),'compressed':(1800,3600),'audio':(1800,4000)}
DIMENSIONS={'actor','event','time','numbers','limitations'}
def compact(text):return ''.join(text.split()) if isinstance(text,str) else ''


def paragraph_map(unit):
    refs={}
    for n,source in enumerate(unit['sources'],1):
        body=source_input(source['source_file'])['body']
        for p,text in enumerate(body.splitlines(),1):
            if text.strip():refs['S'+str(n)+'P'+str(p).zfill(4)]={'source_id':source['source_id'],'source_excerpt':text}
    return refs

def prepare(product_line,source_files,out,ledger=None,event_key=None):
    root=Path(out);root.mkdir(parents=True,exist_ok=True);inputs={}
    for file in source_files:
        record=read(file);view=article_view(record)
        report=root/'sources'/(record['id']+'-cleaning.json');report.parent.mkdir(exist_ok=True)
        # Preserve the original even when cleaning blocks model dispatch.
        raw=root/'sources'/(record['id']+'-original.json')
        if raw.exists() and raw.read_bytes()!=Path(file).read_bytes():raise ValueError('Existing preserved original differs; retain it and use a new prepared version')
        if not raw.exists():raw.write_bytes(Path(file).read_bytes());raw.chmod(0o600)
        trace=view.get('cleaning',{'unavailable':True,'reason':view.get('reason')})
        save(report,{**trace,'source_file':str(raw.resolve()),'source_record_sha256':hashlib.sha256(raw.read_bytes()).hexdigest()})
        if not view.get('available') or view['cleaning']['requires_inspection']:
            raise EditorialError('clean_input_requires_inspection','Source cleaning flagged encoding, changed numbers, suspicious deletion or an empty/short body; inspect retained original before model dispatch','cleaning',[record['id']])
        source=source_input(file);source.pop('body')
        source['source_file']=str(raw.resolve());source['source_sha256']=hashlib.sha256(raw.read_bytes()).hexdigest()
        source.update(title=record['entry'].get('title'),source_date=record['entry'].get('pub_date'),
                      links=view['links'],cleaning_file=str(report.resolve()),
                      readable=write_readable(view['text'],report.parent,'cleaned-'+record['id']))
        inputs[source['source_id']]=source
    units=[]
    if product_line=='radio':
        from editorial_selection import selection
        if not ledger:raise ValueError('Radio drafting requires the original reconciled candidate ledger')
        plan=selection(None,ledger,ledger);keys=[event_key] if event_key else list(plan['events'])
        for key in keys:
            event=plan['events'][key]
            missing=set(event['source_ids'])-set(inputs)
            if missing:raise ValueError('Complete original body missing for selected event '+key)
            units.append({'unit_id':key,'tier':event['tier'],'field':'script','category':event['category'],
                          'sources':[inputs[i] for i in event['source_ids']],
                          'selection_file':plan['file'],'selection_sha256':plan['sha256']})
    else:
        if len(inputs)!=1:raise ValueError('Reading handoff is one complete original article')
        source=next(iter(inputs.values()))
        for stage in ('compressed','audio'):
            units.append({'unit_id':source['source_id']+'-'+stage,'tier':stage,'field':'compressed_markdown' if stage=='compressed' else 'audio_script',
                          'original_url':source['url'],'sources':[source]})
    index={'product_line':product_line,'scope':'selected-event-only' if event_key else 'all-selected-units',
           'semantic_truth_verified_by_program':False,'units':[]}
    for number,unit in enumerate(units,1):
        minimum,maximum=BUDGETS[unit['tier']]
        unit.update(schema='freshloop-draft-unit-v1',product_line=product_line,
                    budget={'min_nonwhitespace_characters':minimum,'max_nonwhitespace_characters':maximum},
                    required_fact_dimensions=sorted(DIMENSIONS),citation_format='paragraph_ref',
                    instruction='Read all complete cleaned originals using every read-plan range. Ledger identifiers/tier are binding, but its old factual descriptions and titles are not evidence. Produce exactly one field for this unit, not a whole program. Keep subject, concrete action, dates, numeric/comparison scope and limitations. No padding or downgraded tier. If sources cannot support the required depth, return status=insufficient_evidence. Cleaning is not factual verification; media not observed cannot be claimed as seen.',
                    output_schema={'unit_id':unit['unit_id'],'tier':unit['tier'],'status':'draft|insufficient_evidence','title':'source-grounded title',unit['field']:'complete manuscript','actor':'exact actor wording used in manuscript','event':'exact event phrase used in manuscript','evidence':[{'dimension':'actor|event|time|numbers|limitations','paragraph_ref':'model-selected S1P0001 reference to the actual supporting complete paragraph','draft_excerpt':'complete exact manuscript sentence/paragraph','explanation':'actual correspondence; never a pass template'}]})
        path=root/('unit-'+str(number).zfill(3)+'.json');save(path,unit)
        binding={'input_file':str(path.resolve()),'input_sha256':digest(unit),'unit_id':unit['unit_id'],'tier':unit['tier'],'budget':unit['budget']}
        save(path.with_suffix('.request.json'),binding);save(path.with_suffix('.paragraphs.json'),paragraph_map(unit));index['units'].append(binding)
    save(root/'index.json',index);return index

def materialize(path):
    """Local test adapters get each complete body ONCE; no source excerpts."""
    unit=read(path);binding=read(Path(path).with_suffix('.request.json'));values=[]
    if digest(unit)!=binding['input_sha256']:raise ValueError('Draft unit/budget changed after preparation')
    refs=paragraph_map(unit)
    for number,source in enumerate(unit['sources'],1):
        file=Path(source['source_file'])
        if hashlib.sha256(file.read_bytes()).hexdigest()!=source['source_sha256']:
            raise ValueError('Original source record changed')
        current=source_input(file)
        if current['body_sha256']!=source['body_sha256']:raise ValueError('Cleaned complete source changed')
        readable=Path(source['readable']['file']).read_text()
        if hashlib.sha256(readable.encode()).hexdigest()!=source['readable']['view_sha256'] or compact(readable)!=compact(current['body']):
            raise ValueError('Readable source differs from complete cleaned body')
        value={k:source[k] for k in ('source_id','url','title','source_date')}
        # href and resolved URL often repeat the same long address; keep the
        # complete link relationship once in model input, raw href in audit.
        value['links']=[{'text':x['text'],'url':x.get('url') or x['href'],'footnote_reference':x.get('footnote_reference',False)} for x in source['links']]
        if unit.get('citation_format')=='paragraph_ref':
            value['complete_paragraphs']=[{'ref':key,'text':row['source_excerpt']} for key,row in refs.items() if row['source_id']==source['source_id']]
        else:value['complete_body']=current['body']
        values.append(value)
    return {k:v for k,v in unit.items() if k!='sources'}|{'complete_originals':values}

def check(input_file,response_file,out):
    unit=read(input_file);binding=read(Path(input_file).with_suffix('.request.json'))
    if binding['input_sha256']!=digest(unit):raise ValueError('Draft input/budget changed after dispatch')
    raw=Path(response_file).read_bytes();errors=[];data=None
    try:data=read(response_file)
    except (ValueError,UnicodeError):errors.append('actual response is not complete JSON; retain raw result')
    if isinstance(data,dict):
        if data.get('unit_id')!=unit['unit_id'] or data.get('tier')!=unit['tier']:errors.append('unit/tier differs from original bound selection; never downgrade')
        if data.get('status')!='draft':errors.append('source evidence is insufficient or draft status missing')
        field=unit['field'];text=data.get(field,'');budget=unit['budget']
        if not isinstance(text,str):errors.append('manuscript field must be text');text=''
        count=len(compact(text))
        if not budget['min_nonwhitespace_characters']<=count<=budget['max_nonwhitespace_characters']:
            errors.append('manuscript length '+str(count)+' outside bound '+str(budget)+'; use real details or report insufficient evidence, never pad')
        for key in ('actor','event'):
            if not compact(data.get(key)) or compact(data[key]) not in compact(text):errors.append('concrete '+key+' missing from final manuscript')
        from freshloop_agent import validate_narration
        errors.extend(validate_narration(text))
        for source in unit['sources']:
            current=source_input(source['source_file'])
            if current['source_sha256']!=source['source_sha256'] or current['body_sha256']!=source['body_sha256']:raise ValueError('Bound original/cleaned source changed before draft check')
        originals={s['source_id']:source_input(s['source_file'])['body'] for s in unit['sources']}
        refs=paragraph_map(unit);resolved=[]
        evidence=data.get('evidence',[]);dimensions=set()
        if not isinstance(evidence,list):evidence=[]
        for number,anchor in enumerate(evidence):
            if not isinstance(anchor,dict):errors.append('evidence anchor must be an object');continue
            citation=refs.get(anchor.get('paragraph_ref')) if unit.get('citation_format')=='paragraph_ref' else {'source_id':anchor.get('source_id'),'source_excerpt':anchor.get('source_excerpt','')}
            if not citation:errors.append('evidence['+str(number)+'] selects an unknown source paragraph');continue
            resolved.append({**anchor,**citation});ident=citation['source_id'];dimension=anchor.get('dimension');dimensions.add(dimension)
            if ident not in originals or not passage_match(originals[ident],citation['source_excerpt']):errors.append('evidence['+str(number)+'] original paragraph binding changed')
            # Generation anchors may be exact phrases, unlike final reviewer receipts,
            # which still require full source/draft sentence comparisons.
            if not compact(anchor.get('draft_excerpt')) or compact(anchor['draft_excerpt']) not in compact(text):errors.append('evidence['+str(number)+'] draft_excerpt is not an exact phrase in this manuscript')
            if not anchor.get('explanation'):errors.append('evidence['+str(number)+'] lacks actual correspondence explanation')
        if not DIMENSIONS<=dimensions:errors.append('actor/event/time/numbers/limitations evidence is incomplete')
        if {e.get('source_id') for e in resolved}!={s['source_id'] for s in unit['sources']}:errors.append('quote each contributing source; do not omit its actual contribution')
    elif not errors:errors.append('actual response must be one manuscript object')
    result={'resolved_evidence':resolved if isinstance(data,dict) else [],'citation_resolution':'Only model-selected refs mapped to immutable original paragraphs; no judgments generated','unit_id':unit['unit_id'],'input_sha256':binding['input_sha256'],'response_sha256':hashlib.sha256(raw).hexdigest(),
            'mechanical_valid':not errors,'errors':errors,'semantic_truth_verified_by_program':False,'requires_actual_independent_review':True}
    save(out,result);return result
