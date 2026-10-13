#!/usr/bin/env python3
"""Reproduce source-bound manuscript corrections offline. No API/model/TTS calls."""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import sys
import shutil

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'skills/freshloop-content-agent/scripts'))
from article_view import article_view
from freshloop_agent import validate, save
from review_comparison import digest, passage_match, prepare_inputs


def read(path):
    return json.loads(Path(path).read_text())


def reconcile_reading(path):
    path=Path(path);ledger=read(path);changed=[]
    for row in ledger['candidates']:
        if row['outcome']=='not-selected' and ('预算' in row['reason'] and '后续版次核验' in row['reason']):
            row['prior_outcome']=row['outcome'];row['prior_reason']=row['reason']
            row['outcome']='pending';row['deferral_reason']='editorial_budget_exhausted'
            row['reason']='原早间任务将编辑预算用于Radio和审查；本篇正文未抓取、未核验、未发布，保留待处理。'
            changed.append(row['source_id'])
    if changed:
        backup=path.with_name(path.name+'.before-budget-reconciliation')
        if not backup.exists():shutil.copy2(path,backup)
        save(path,ledger)
    return changed


def replay(source_run, case_file, out):
    source_run=Path(source_run);out=Path(out)
    if out.resolve()==source_run.resolve() or out.resolve().is_relative_to(source_run.resolve()):
        raise ValueError('Offline replay output must be independent of production history')
    out.mkdir(parents=True,exist_ok=True)
    original=read(source_run/'radio_program.json');artifact=copy.deepcopy(original)
    ledger=copy.deepcopy(read(source_run/'candidate-ledger.json'))
    sources={}
    source_dir=out/'sources';source_dir.mkdir(exist_ok=True)
    for path in (source_run/'sources_content').glob('*.json'):
        record=read(path)
        if record.get('original_html'):
            assert hashlib.sha256(record['original_html'].encode()).hexdigest()==record['content_hash'],str(path)
        body=article_view(record)['text']
        target=source_dir/path.name;shutil.copy2(path,target)
        sources[record['entry']['link']]=(record,body,target)
    cases=read(case_file)['cases'];differences=[]
    for case in cases:
        matching=[(section,story) for section in artifact['sections'] for story in section['stories'] if story['event_key']==case['event_key']]
        assert len(matching)==1,case['event_key']
        section,story=matching[0];before=copy.deepcopy(story);claims=[];anchors=[]
        for anchor in case['anchors']:
            record,body,path=sources[anchor['source_url']]
            prefix=''.join(anchor['starts'].split())
            paragraphs=[line.strip() for line in body.splitlines() if ''.join(line.split()).startswith(prefix)]
            assert len(paragraphs)==1,(anchor['source_url'],anchor['starts'],len(paragraphs))
            excerpt=paragraphs[0]
            assert passage_match(body,excerpt),anchor['source_url']
            claims.append(dict(claim=anchor['claim'],source_url=anchor['source_url'],evidence=excerpt))
            anchors.append(dict(source_id=record['id'],url=anchor['source_url'],published_at=record['entry'].get('pub_date'),
                                source_html_sha256=record['content_hash'],source_excerpt=excerpt,claim=anchor['claim']))
        assert set(story['source_urls']) <= {c['source_url'] for c in claims},case['event_key']
        for field in ('script','audio_script'):
            old=story.get(field,story['script'])
            assert section[field].count(old)==1,(case['event_key'],field)
            section[field]=section[field].replace(old,case[field])
            story[field]=case[field]
        story['title']=case['title']
        section['editorial']['claims']=[c for c in section['editorial']['claims'] if c['source_url'] not in story['source_urls']]+claims
        for row in ledger['candidates']:
            if row.get('event_key')==case['event_key'] and 'required_details' in row:
                row['required_details']={k:case[k] for k in ('actor','event','limits')}
                row['reason']=case['event']+'；'+case['limits']
        differences.append(dict(event_key=case['event_key'],before=before,after=copy.deepcopy(story),source_facts=anchors))
    artifact['opening']=artifact['opening'].replace('IonQ巨资收购Elisity布局量子网络防御','IonQ拟通过全股票交易收购网络安全公司Elisity')
    artifact['closing']='感谢您的收听。本期带来了AI、技术产业、健康、硬件、游戏、文娱、财经、时政、科学与生活板块的消息。祝您今天工作顺利、生活愉快，晚间新闻联播再见。'
    reasons={'生命健康':'Rubedo公布皮肤外用药早期人体试验；保留小样本和衰老时钟指标限制。','影音文娱':'星际迷航纪录片放映与Beats宝可梦联名预告；区分作品媒介及未公布的产品细节。','科学探索':'AVIDrone传统布局电动直升机赢得DARPA重载挑战；保留载荷和机体干重比较。'}
    for category in ledger['categories']:
        if category['name'] in reasons:category['reason']=reasons[category['name']]
    shutil.copy2(source_run/'radio-sources.json',out/'radio-sources.json')
    ledger['inventory_file']=str((out/'radio-sources.json').resolve())
    artifact_path=out/'radio-program.corrected.json';ledger_path=out/'candidate-ledger.corrected.json'
    save(artifact_path,artifact);save(ledger_path,ledger)
    errors=validate(artifact,'radio_program')
    assert not errors,errors
    all_urls={u for s in artifact['sections'] for story in s['stories'] for u in story['source_urls']}
    prepared=prepare_inputs(artifact_path,[str(sources[url][2]) for url in sorted(all_urls)],out/'review-inputs.json',ledger_path)
    result={'mode':'offline_candidate','source_quotes_bound':True,'semantic_truth_verified_by_program':False,
            'requires_actual_independent_review':True,'source_run':str(source_run.resolve()),'original_artifact_sha256':digest(original),
            'corrected_artifact_sha256':digest(artifact),'original_unchanged':digest(read(source_run/'radio_program.json'))==digest(original),
            'corrected_cases':len(differences),'differences':differences,'mechanical_errors':errors,'review_inputs':prepared,
            'external_api_calls':0,'tts_calls':0,'publications':0}
    save(out/'replay-result.json',result)
    paragraphs=[artifact['opening']]
    for section in artifact['sections']:paragraphs.extend(['【'+section['category']+'】',section['script']])
    paragraphs.append(artifact['closing'])
    (out/'replacement-manuscript.txt').write_text('\n\n'.join(paragraphs)+'\n')
    return result


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--source-run',required=True);parser.add_argument('--case-file',required=True);parser.add_argument('--out',required=True)
    parser.add_argument('--reconcile-reading',action='store_true')
    args=parser.parse_args();result=replay(args.source_run,args.case_file,args.out)
    if args.reconcile_reading:
        reading_before=hashlib.sha256((Path(args.source_run)/'reading-ledger.json').read_bytes()).hexdigest()
        target=Path(args.out)/'reading-ledger.reconciled.json'
        shutil.copy2(Path(args.source_run)/'reading-ledger.json',target)
        shutil.copy2(Path(args.source_run)/'reading-sources.json',Path(args.out)/'reading-sources.json')
        ledger=read(target);ledger['inventory_file']=str((Path(args.out)/'reading-sources.json').resolve());save(target,ledger)
        result['restored_reading_pending_ids']=reconcile_reading(target)
        result['reading_original_preserved']=hashlib.sha256((Path(args.source_run)/'reading-ledger.json').read_bytes()).hexdigest()==reading_before
        assert result['reading_original_preserved']
        save(Path(args.out)/'replay-result.json',result)
    print(json.dumps({k:v for k,v in result.items() if k not in ('differences',)},ensure_ascii=False))


if __name__=='__main__':main()
