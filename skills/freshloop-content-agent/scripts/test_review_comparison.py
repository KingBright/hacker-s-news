"""Mechanical tests for source binding and review outcomes, not an LLM benchmark."""
import argparse
import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, patch
import freshloop_agent as agent
import review_comparison as review
import delivery
from test_freshloop_agent import radio_artifact, artifact, reviewed_file, sync_receipts


class ComparisonTests(unittest.TestCase):
    def test_normal_major_and_one_sentence_brief_are_not_length_gated(self):
        with tempfile.TemporaryDirectory() as root:
            p=Path(root)/'review.json';data=radio_artifact();reviewed_file(p,data)
            agent.require_final_review(data,p)
            data=radio_artifact();data['stories'][0].update(tier='brief',script='Acme 发布补丁，旧版远程管理接口用户需要更新。')
            data['script']=data['audio_script']='\n'.join(s['script'] for s in data['stories'])
            reviewed_file(p,data);agent.require_final_review(data,p)

    def test_actor_number_limit_fabrication_and_overcompression_review_blocks_submit(self):
        # Explicit review differences represent semantic judgments supplied by a reviewer.
        differences=[
            ('actor','删掉Sakana AI和伯克利主体，听众无法识别研究归属'),
            ('numbers','把三项测试任务的69.9%胜率写成96.9%，数值与比较口径不符'),
            ('limits','遗漏Terminal-Bench和SWE-bench指标未改善及共享盲点限制'),
            ('unsupported_additions','来源没有商业部署或安全保证，文稿却声称已商用且安全'),
            ('compression','压成“AI实现突破”，丢失方法、比较范围与重要限制')]
        with tempfile.TemporaryDirectory() as root:
            root=Path(root)
            for kind,reason in differences:
                data=radio_artifact();p=root/'review.json';reviewed_file(p,data)
                r=agent.read(p);row=r['reviews'][0]['comparisons'][0]
                row.update(conclusion='revise',differences=[dict(kind=kind,reason=reason)])
                sync_receipts(p,r);draft=root/'draft.json';agent.save(draft,data)
                state=root/'state.json';agent.save(state,dict(job_id='same-job',job_type='radio_episode',lease_token='private'))
                args=argparse.Namespace(command='submit',state=str(state),artifact=str(draft),review_file=str(p))
                api=Mock()
                with patch.object(agent,'nexus',return_value=api):
                    with self.assertRaises(review.EditorialError):agent.dispatch(args)
                api.call.assert_not_called()
                journal=review.issues(root)
                self.assertIn(reason,journal['issues'][-1]['reason'])
                self.assertEqual(journal['issues'][-1]['disposition'],'blocked')

    def test_changed_source_forged_excerpt_and_missing_audio_comparison_rejected(self):
        with tempfile.TemporaryDirectory() as root:
            p=Path(root)/'review.json';data=radio_artifact();reviewed_file(p,data)
            r=agent.read(p);packet=agent.read(r['input_file']);source=Path(packet['sources'][0]['source_file'])
            old=source.read_text();source.write_text(old+' ')
            with self.assertRaisesRegex(review.EditorialError,'changed'):agent.require_final_review(data,p)
            source.write_text(old)
            r['reviews'][0]['comparisons'][0]['evidence'][0]['source_excerpt']='invented evidence'
            sync_receipts(p,r)
            with self.assertRaisesRegex(review.EditorialError,'excerpts'):agent.require_final_review(data,p)
            reviewed_file(p,data);r=agent.read(p);r['reviews'][1]['comparisons'].pop();sync_receipts(p,r)
            with self.assertRaisesRegex(review.EditorialError,'every'):agent.require_final_review(data,p)

    def test_source_packet_tampering_and_unknown_submit_reconciliation(self):
        with tempfile.TemporaryDirectory() as root:
            root=Path(root);p=root/'review.json';data=radio_artifact();reviewed_file(p,data)
            r=agent.read(p);packet=agent.read(r['input_file']);snapshot=Path(packet['sources'][0]['snapshot']);s=agent.read(snapshot);s['body']='replaced body';agent.save(snapshot,s)
            with self.assertRaisesRegex(review.EditorialError,'input given'):agent.require_final_review(data,p)
            state=root/'state.json';draft=root/'draft.json';agent.save(draft,data);agent.save(state,dict(job_id='j',job_type='radio_episode',lease_token='secret'))
            args=argparse.Namespace(command='submit',artifact=str(draft),state=str(state))
            review.record_issue(args,RuntimeError('Request outcome unknown'))
            self.assertTrue(review.unresolved(review.report_issues(root)))
            review.submission_reconciled(dict(job_id='j',status='completed',result_ref='item:1'),state)
            self.assertFalse(review.unresolved(review.report_issues(root)))
            self.assertEqual(review.issues(root)['issues'][0]['disposition'],'submission_confirmed')

    def test_mechanical_validation_uses_same_revision_budget(self):
        with tempfile.TemporaryDirectory() as root:
            root=Path(root);state=root/'state.json';draft=root/'draft.json'
            agent.save(state,dict(job_id='j',job_type='radio_episode',lease_token='secret'))
            args=argparse.Namespace(command='validate',job_type='radio_episode',artifact=str(draft),state=str(state))
            for i in range(3):
                agent.save(draft,dict(title=str(i)))
                self.assertTrue(agent.dispatch(args))
            self.assertEqual(agent.read(state)['editorial_status'],'failed')
            with self.assertRaisesRegex(review.EditorialError,'exhausted'):agent.dispatch(args)
            self.assertTrue(agent.validate([], 'reading_article'))

    def test_malformed_draft_is_retained_and_reported_without_success(self):
        with tempfile.TemporaryDirectory() as root:
            root=Path(root);draft=root/'bad.json';draft.write_text('{invalid json')
            state=root/'state.json';agent.save(state,dict(job_id='j',job_type='radio_episode',lease_token='secret'))
            args=argparse.Namespace(command='review-check',artifact=str(draft),state=str(state),review_file=str(root/'review.json'))
            with self.assertRaises(json.JSONDecodeError):agent.dispatch(args)
            issue=review.issues(root)['issues'][0]
            self.assertEqual(issue['stage'],'validation')
            archived=agent.read(root/'.editorial/artifacts'/(issue['artifact_sha256']+'.json'))
            self.assertEqual(archived['raw_text'],'{invalid json')
            self.assertIn('JSON',(root/'editorial-issues.json').read_text())
            self.assertNotIn('secret',(root/'run-report.md').read_text())

    def test_preflight_reviews_use_same_revision_budget_before_submit(self):
        with tempfile.TemporaryDirectory() as root:
            root=Path(root);state=root/'state.json';draft=root/'draft.json';p=root/'review.json'
            agent.save(state,dict(job_id='j',job_type='radio_episode',lease_token='secret'))
            args=argparse.Namespace(command='review-check',state=str(state),artifact=str(draft),review_file=str(p))
            for i in range(3):
                data=radio_artifact();data['title']=str(i);agent.save(draft,data);reviewed_file(p,data)
                r=agent.read(p);r['reviews'][0]['comparisons'][0].update(conclusion='reject',differences=[dict(kind='numbers',reason='数字失真')]);sync_receipts(p,r)
                with self.assertRaises(review.EditorialError):agent.dispatch(args)
            self.assertEqual(agent.read(state)['editorial_status'],'failed')
            data=radio_artifact();agent.save(draft,data);reviewed_file(p,data)
            with self.assertRaisesRegex(review.EditorialError,'exhausted'):agent.dispatch(args)

    def test_concurrent_errors_are_not_lost(self):
        from concurrent.futures import ThreadPoolExecutor
        with tempfile.TemporaryDirectory() as root:
            args=argparse.Namespace(command='source',id='source',out=str(Path(root)/'source.json'))
            with ThreadPoolExecutor(max_workers=4) as pool:
                list(pool.map(lambda i:review.record_issue(args,review.EditorialError('failure-'+str(i),'source failure '+str(i),'source',['source'])),range(8)))
            self.assertEqual(len(review.issues(root)['issues']),8)

    def test_reading_requires_original_compressed_and_audio_comparisons(self):
        with tempfile.TemporaryDirectory() as root:
            p=Path(root)/'review.json';data=artifact();reviewed_file(p,data)
            agent.require_final_review(data,p)
            r=agent.read(p)
            self.assertEqual({x['unit'] for x in r['reviews'][0]['comparisons']},{'/reader_markdown','/compressed_markdown','/audio_script'})

    def test_missing_original_never_uses_rss_excerpt_and_path_escape_is_rejected(self):
        with tempfile.TemporaryDirectory() as root:
            root=Path(root);p=root/'source.json'
            agent.save(p,dict(id='source',entry=dict(link='https://example.org/story',description='RSS teaser'),reader_markdown='short teaser'))
            with self.assertRaisesRegex(review.EditorialError,'无法完成来源核验'):review.source_input(p)
            with self.assertRaises(review.EditorialError):review.bounded_path(root,'../config.toml')

    def test_source_fetch_failure_is_saved_reported_and_exits_failure(self):
        with tempfile.TemporaryDirectory() as root:
            root=Path(root);out=root/'source.json'
            args=argparse.Namespace(command='source',id='s1',out=str(out))
            bad=dict(id='s1',entry=dict(link='https://example.org/story'),fetch_error='HTTP 403',original_html='')
            with patch.object(agent.Api,'call',return_value=bad):
                with self.assertRaises(review.EditorialError):agent.dispatch(args)
            self.assertEqual(agent.read(out)['fetch_error'],'HTTP 403')
            self.assertIn('无法完成来源核验',(root/'run-report.md').read_text())
            self.assertEqual(review.issues(root)['issues'][0]['source_ids'],['s1'])

    def test_revision_budget_counts_distinct_bad_drafts_and_retains_versions(self):
        with tempfile.TemporaryDirectory() as root:
            root=Path(root);state=root/'state.json';draft=root/'draft.json'
            agent.save(state,dict(job_id='j',job_type='radio_episode',lease_token='secret'))
            args=argparse.Namespace(command='submit',state=str(state),artifact=str(draft),review_file=None)
            for i in range(3):
                data=radio_artifact();data['title']=str(i);data['script']='';agent.save(draft,data)
                with patch.object(agent,'nexus',return_value=Mock()):
                    with self.assertRaises(review.EditorialError):agent.dispatch(args)
            self.assertEqual(agent.read(state)['editorial_status'],'failed')
            self.assertEqual(len(list((root/'.editorial/artifacts').glob('*.json'))),3)
            data=radio_artifact();agent.save(draft,data);p=root/'review.json';reviewed_file(p,data)
            args.review_file=str(p);api=Mock()
            with patch.object(agent,'nexus',return_value=api):
                with self.assertRaisesRegex(review.EditorialError,'exhausted'):agent.dispatch(args)
            api.call.assert_not_called()
            self.assertNotIn('secret',(root/'editorial-issues.json').read_text())

    def test_corrected_review_remains_visible_and_unresolved_issue_blocks_delivery(self):
        with tempfile.TemporaryDirectory() as root:
            root=Path(root);manifest=root/'manifest.json';state=root/'state.json';draft=root/'draft.json'
            data=artifact();agent.save(draft,data);agent.save(state,dict(job_id='j',job_type='reading_article'))
            args=argparse.Namespace(command='submit',artifact=str(draft),state=str(state))
            review.record_issue(args,review.EditorialError('bad','数字失真','review'))
            agent.save(manifest,dict(date='2026-10-12',edition='晚间',outputs=[dict(title='新闻',voice_job_id='voice')]))
            delivery_args=argparse.Namespace(manifest=manifest,out=root/'delivery.json',report=root/'run-report.md',wait_seconds=0)
            ready=dict(title='新闻',voice_job_id='voice',state='ready',audio_url='https://example.org/a.mp3',duration_sec=60)
            with patch.object(delivery,'check_output',return_value=ready),patch.object(delivery,'check_weekly',return_value=dict(state='ready',audio_url='https://example.org/w.mp3')):
                self.assertEqual(delivery.finalize(delivery_args,None,None,agent.save),2)
                self.assertFalse(agent.read(delivery_args.out)['delivered'])
                p=root/'review.json';reviewed_file(p,data);agent.require_final_review(data,p);review.approved(data,p,agent.read(state))
                self.assertEqual(delivery.finalize(delivery_args,None,None,agent.save),0)
            self.assertIn('数字失真',(root/'run-report.md').read_text())
            self.assertIn('corrected_and_reviewed',(root/'run-report.md').read_text())

if __name__ == '__main__':unittest.main()
