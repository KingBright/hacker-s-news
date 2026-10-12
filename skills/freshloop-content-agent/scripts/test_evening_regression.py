"""Real failure patterns with independent source text; no network/model/TTS calls."""
import argparse
import copy
from html import escape
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, patch
import freshloop_agent as agent
import review_comparison as review
from editorial_selection import bind_selection, selection
import delivery

FIXTURE = Path(__file__).parent / 'fixtures' / 'evening_quality.json'


def sample(root):
    fixture = json.loads(FIXTURE.read_text())
    rows = fixture['sources']; files = []
    sections = []
    for row in rows:
        source = root / (row['id'] + '.json')
        review.save(source, {'id':row['id'], 'entry':{'link':row['url'],'pub_date':'2026-10-12T05:00:00+00:00'},
                            'original_html':'<article><p>'+escape(row['body'])+'</p></article>', 'fetch_error':None})
        files.append(str(source))
        sentences = row['body'].split('。')
        section = {'title':row['title'],'category':row['category'],'script':row['draft'],'audio_script':row['draft'],
                   'sources':[{'url':row['url'],'title':row['title']}],
                   'stories':[{'event_key':row['key'],'tier':row['tier'],'title':row['title'],'script':row['draft'],
                               'audio_script':row['draft'],'source_urls':[row['url']]}],
                   'editorial':{'language':'zh-CN','ready':True,'dedup_checked':True,'media':[],
                                'quality':dict.fromkeys(('fidelity','selection','listening','structure','media'),2),
                                'claims':[{'claim':s,'evidence':s+'。','source_url':row['url']} for s in sentences[:2]],
                                'radio_coverage':{'reviewed':True,'eligible_event_keys':[row['key']]}}}
        sections.append(section)
    artifact = {'title':'非生产回归样稿','opening':'这是独立回归样稿。','closing':'回归样稿结束。','sections':sections}
    inventory = {'categories':[{'name':r['category']} for r in rows]+[{'name':'AI前沿'}],
                 'sources':{'items':[{'id':r['id'],'url':r['url']} for r in rows]},
                 'pagination':{'complete':True,'since':1791792000}}
    inv = root / 'inventory.json'; review.save(inv,inventory)
    ledger = {'inventory_file':str(inv),'inventory_sha256':review.digest(inventory),
              'categories':[{'name':r['category'],'outcome':'covered','reason':r['title']} for r in rows]+[
                  {'name':'AI前沿','outcome':'empty','reason':'该独立测试输入不包含AI前沿新事件。'}],
              'candidates':[{'source_id':r['id'],'category':r['category'],'outcome':'included-'+r['tier'],
                             'event_key':r['key'],'reason':r['title'],'importance_reason':r['importance'],
                             'required_details':{'actor':r['body'].split('。')[0],'event':r['title'],'limits':r['limits']}} for r in rows]}
    lp = root / 'candidate-ledger.json';review.save(lp,ledger)
    return artifact, files, lp, rows


def receipts(root, artifact, files, ledger, rows, difference=None):
    draft=root/'draft.json';review.save(draft,artifact)
    prepared=review.prepare_inputs(draft,files,root/'inputs.json',ledger)
    packet=review.read(prepared['input_file'])
    for role in ('facts','listening'):
        comparisons=[]
        for unit in packet['units']:
            event=next(r for r in rows if r['url'] in unit['source_urls'])
            reason = (difference or {}).get(event['key'])
            comp={'unit':unit['unit'],'source_ids':[event['id']],'checked_dimensions':sorted(review.DIMENSIONS),
                  'conclusion':'revise' if reason else 'pass',
                  'differences':[{'kind':'important_omissions','reason':reason}] if reason else [],
                  'assessment':role+': '+event['importance']+' 核对的限制为：'+event['limits'],
                  'evidence':[{'source_id':event['id'],'source_excerpt':event['body'],'draft_excerpt':unit['text'],
                               'observation':role+': 原文主体、进展和限定对应本条终稿；'+(reason or event['limits'])}]}
            comparisons.append(comp)
        result={'role':role,'artifact_sha256':prepared['artifact_sha256'],'input_file':prepared['input_file'],
                'input_sha256':prepared['input_sha256'],'blocking':[],
                'checked_artifacts':['draft.json'],'summary':role+'逐条对照完整独立原文及显示/可听稿',
                'comparisons':comparisons}
        review.save(root/(role+'.json'),result)
    return draft, packet


class EveningRegressionTests(unittest.TestCase):
    def test_correct_major_brief_mix_and_complete_sources_pass(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);a,files,ledger,rows=sample(root);draft,_=receipts(root,a,files,ledger,rows)
            self.assertEqual(agent.validate(a,'radio_program'),[])
            review.assemble_review(draft,root/'inputs.json',root/'facts.json',root/'listening.json',root/'review.json')
            agent.require_final_review(a,root/'review.json')
            self.assertEqual([s['stories'][0]['tier'] for s in a['sections']],['major','major','brief'])

    def test_linux_branch_limit_and_third_star_omissions_block_before_api(self):
        cases={'linux':('Linux 创始人借助 Claude Code 优化文件读取，减少多线程引用计数竞争。','省掉个人分支尚未进入主线和特定测试负载限制。'),
               'fast':('国家天文台确认脉冲星三体系统，脉冲星与白矮星组成双星。','声称三体却省掉第三颗类太阳恒星，解释不完整。')}
        for key,(bad,reason) in cases.items():
            with self.subTest(key=key), tempfile.TemporaryDirectory() as tmp:
                root=Path(tmp);a,files,ledger,rows=sample(root)
                sec=next(s for s in a['sections'] if s['stories'][0]['event_key']==key)
                sec['script']=sec['audio_script']=sec['stories'][0]['script']=sec['stories'][0]['audio_script']=bad
                draft,_=receipts(root,a,files,ledger,rows,{key:reason})
                with self.assertRaises(review.EditorialError) as caught:
                    review.assemble_review(draft,root/'inputs.json',root/'facts.json',root/'listening.json',root/'review.json')
                self.assertEqual(caught.exception.code,'semantic_revise')
                state=root/'state.json';review.save(state,{'job_id':'offline-only','job_type':'radio_program','lease_token':'fixture'})
                args=argparse.Namespace(command='submit',state=str(state),artifact=str(draft),review_file=str(root/'review.json'))
                api=Mock()
                with patch.object(agent,'nexus',return_value=api),self.assertRaises(review.EditorialError):agent.dispatch(args)
                api.call.assert_not_called()

    def test_fragment_slice_copied_receipt_and_stale_receipt_are_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);a,files,ledger,rows=sample(root);draft,_=receipts(root,a,files,ledger,rows)
            facts=review.read(root/'facts.json')
            anchor=facts['comparisons'][0]['evidence'][0]
            anchor['source_excerpt']=rows[0]['body'][10:20]
            anchor['draft_excerpt']=a['sections'][0]['script'][10:20]
            review.save(root/'facts.json',facts)
            with self.assertRaises(review.EditorialError) as caught:
                review.assemble_review(draft,root/'inputs.json',root/'facts.json',root/'listening.json',root/'review.json')
            self.assertEqual(caught.exception.code,'comparison_evidence_mismatch')
            receipts(root,a,files,ledger,rows);facts=review.read(root/'facts.json');listening=copy.deepcopy(facts);listening['role']='listening'
            review.save(root/'listening.json',listening)
            with self.assertRaises(review.EditorialError) as caught:
                review.assemble_review(draft,root/'inputs.json',root/'facts.json',root/'listening.json',root/'review.json')
            self.assertEqual(caught.exception.code,'review_receipt_copied')
            receipts(root,a,files,ledger,rows);a['title']='changed final manuscript';review.save(draft,a)
            with self.assertRaises(review.EditorialError):review.assemble_review(draft,root/'inputs.json',root/'facts.json',root/'listening.json',root/'review.json')

    def test_no_synthesized_pass_or_receipt_rewrite(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);a,files,ledger,rows=sample(root);draft,_=receipts(root,a,files,ledger,rows)
            facts=review.read(root/'facts.json');facts['comparisons'][0]['assessment']='通过审核';review.save(root/'facts.json',facts)
            with self.assertRaises(review.EditorialError):review.assemble_review(draft,root/'inputs.json',root/'facts.json',root/'listening.json',root/'review.json')
            receipts(root,a,files,ledger,rows);review.assemble_review(draft,root/'inputs.json',root/'facts.json',root/'listening.json',root/'review.json')
            data=review.read(root/'review.json');data['reviews'][0]['summary']='replacement summary';review.save(root/'review.json',data)
            with self.assertRaises(review.EditorialError) as caught:agent.require_final_review(a,root/'review.json')
            self.assertEqual(caught.exception.code,'review_receipt_changed')

    def test_candidate_missing_category_missing_and_downgrade_are_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);a,files,ledger,rows=sample(root);state={}
            bind_selection(a,state,root/'draft.json',ledger)
            a['sections'][0]['stories'][0]['tier']='brief'
            with self.assertRaises(review.EditorialError) as caught:bind_selection(a,state,root/'draft.json')
            self.assertEqual(caught.exception.code,'selection_depth_changed')
            a,files,ledger,rows=sample(root);data=review.read(ledger);data['candidates'].pop();review.save(ledger,data)
            with self.assertRaises(review.EditorialError):selection(a,ledger,root/'draft.json')
            a,files,ledger,rows=sample(root);data=review.read(ledger);data['categories'].pop();review.save(ledger,data)
            with self.assertRaises(review.EditorialError):selection(a,ledger,root/'draft.json')

    def test_resolved_summary_rewrite_and_history_rewrite_do_not_clear_failure(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);draft=root/'draft.json';review.save(draft,{'title':'bad'})
            args=argparse.Namespace(command='validate',artifact=str(draft))
            review.record_issue(args,review.EditorialError('bad','Linux branch limitation omitted','review'))
            journal=root/'.editorial/issues.events.jsonl';original=journal.read_bytes()
            data=review.read(root/'editorial-issues.json');data['issues'][0]['disposition']='corrected_and_reviewed';review.save(root/'editorial-issues.json',data)
            self.assertTrue(review.unresolved(review.report_issues(root)))
            self.assertEqual(journal.read_bytes(),original)
            entry=json.loads(journal.read_text());entry['data']=data;journal.write_text(json.dumps(entry)+'\n')
            self.assertTrue(review.unresolved(review.report_issues(root)))

    def test_legitimate_review_transition_preserves_original_error_and_binding(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);a,files,ledger,rows=sample(root);draft,_=receipts(root,a,files,ledger,rows)
            args=argparse.Namespace(command='review-check',artifact=str(draft),state=str(root/'state.json'))
            review.save(args.state,{'job_id':'offline-only','job_type':'radio_program'})
            review.record_issue(args,review.EditorialError('bad','old wrong number','review'))
            review.assemble_review(draft,root/'inputs.json',root/'facts.json',root/'listening.json',root/'review.json')
            agent.require_final_review(a,root/'review.json');review.approved(a,root/'review.json',review.read(args.state))
            events=review.issue_events(root)
            self.assertEqual(events[0]['data']['issues'][0]['disposition'],'blocked')
            self.assertEqual(events[-1]['data']['issues'][0]['disposition'],'corrected_and_reviewed')
            self.assertEqual(events[-1]['evidence']['artifact_sha256'],review.digest(a))
            self.assertIn('input_sha256',events[-1]['evidence'])

    def test_reading_budget_skip_cannot_report_delivery_success(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp)/'agent-runs/2026-10-12/evening-program';root.mkdir(parents=True)
            manifest=root/'manifest.json'
            review.save(manifest,{'date':'2026-10-12','edition':'晚间','outputs':[{'title':'Radio','voice_job_id':'v'}],
                                  'warnings':['时间限制，Reading未产出。']})
            args=argparse.Namespace(manifest=manifest,out=root/'delivery.json',report=root/'report.md',wait_seconds=0)
            with patch.object(delivery,'check_output',return_value={'title':'Radio','voice_job_id':'v','state':'ready','audio_url':'https://example.test/a.mp3','duration_sec':117}),patch.object(delivery,'check_weekly',return_value={'state':'ready','audio_url':'https://example.test/w.mp3'}):
                self.assertEqual(delivery.finalize(args,None,None,review.save),2)
            self.assertFalse(review.read(args.out)['delivered'])
            inventory={'sources':{'items':[{'id':'reading-1'}]},'pagination':{'complete':True}}
            review.save(root/'reading-inventory.json',inventory)
            ledger={'inventory_file':str(root/'reading-inventory.json'),'inventory_sha256':review.digest(inventory),
                    'candidates':[{'source_id':'reading-1','outcome':'pending','reason':'全文尚未审核，不能把RSS当正文。'}]}
            review.save(root/'reading-ledger.json',ledger)
            value=review.read(manifest);value['reading_ledger']=str(root/'reading-ledger.json')
            self.assertTrue(delivery.reading_scope(value,manifest))
            ledger['candidates'][0].update(outcome='published',voice_job_id='missing-voice')
            review.save(root/'reading-ledger.json',ledger)
            self.assertTrue(delivery.reading_scope(value,manifest))
            ledger['candidates'][0].update(voice_job_id='v')
            review.save(root/'reading-ledger.json',ledger)
            self.assertEqual(delivery.reading_scope(value,manifest),[])

    def test_changing_the_pinned_depth_plan_does_not_bypass_downgrade_guard(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);a,files,ledger,rows=sample(root);state={}
            bind_selection(a,state,root/'draft.json',ledger)
            changed=review.read(ledger);changed['candidates'][0]['outcome']='included-brief';review.save(ledger,changed)
            a['sections'][0]['stories'][0]['tier']='brief'
            with self.assertRaises(review.EditorialError) as caught:bind_selection(a,state,root/'draft.json',ledger)
            self.assertEqual(caught.exception.code,'selection_plan_changed')

    def test_plan_is_pinned_even_when_the_first_draft_changes_depth(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);a,files,ledger,rows=sample(root);state={};sp=root/'state.json'
            a['sections'][0]['stories'][0]['tier']='brief'
            with self.assertRaises(review.EditorialError):bind_selection(a,state,root/'draft.json',ledger,sp)
            self.assertEqual(review.read(sp)['editorial_selection']['sha256'],review.digest(review.read(ledger)))
            changed=review.read(ledger);changed['candidates'][0]['outcome']='included-brief';review.save(ledger,changed)
            with self.assertRaises(review.EditorialError) as caught:bind_selection(a,state,root/'draft.json',ledger,sp)
            self.assertEqual(caught.exception.code,'selection_plan_changed')

    def test_radio_pending_coverage_blocks_full_edition_and_weekly_is_separate(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp)/'agent-runs/2026-10-12/evening-program';root.mkdir(parents=True)
            a,files,ledger,rows=sample(root);state={}
            bind_selection(a,state,root/'draft.json',ledger);review.save(root/'radio-state.json',state)
            manifest={'edition':'晚间','radio_ledger':str(ledger)}
            self.assertEqual(delivery.radio_scope(manifest,root/'manifest.json'),[])
            data=review.read(ledger);data['categories'][-1].update(outcome='pending',reason='尚未查清AI前沿候选。');review.save(ledger,data)
            self.assertTrue(delivery.radio_scope(manifest,root/'manifest.json'))
            self.assertEqual(delivery.reading_scope({'edition':'周报'},root/'manifest.json'),[])

if __name__ == '__main__': unittest.main()
