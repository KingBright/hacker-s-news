import argparse
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import freshloop_agent as agent


def artifact():
    return {'title':'测试','original_url':'https://example.org/article','reader_markdown':'完整原文',
            'compressed_markdown':'中文摘要','audio_script':'这是一份有来源、有明确事实和限制的中文听稿。'*8,
            'editorial':{'language':'zh-CN','ready':True,'dedup_checked':True,
                         'quality':dict.fromkeys(('fidelity','selection','listening','structure','media'),2),
                         'claims':[{'claim':'已核实','source_url':'https://example.org/article','evidence':'原文片段'}],'media':[]}}


def radio_artifact():
    data=artifact()
    stories=[dict(event_key=f'event-{i}',tier='major',title=f'新闻{i}',script=(f'这是第{i}项独立新闻。已经核对原始报道，保留事实范围和限制。'*4),source_urls=[f'https://example.org/{i}']) for i in range(3)]
    data.update(category='Tech',stories=stories,script='\n'.join(x['script'] for x in stories))
    data['audio_script']=data['script']
    data['sources']=[dict(url=x['source_urls'][0],title=x['title'],summary='来源证据') for x in stories]
    data['editorial']['claims']=[dict(claim=x['title'],source_url=x['source_urls'][0],evidence='原文引文') for x in stories]
    data['editorial']['radio_coverage'] = dict(reviewed=True, eligible_event_keys=[x['event_key'] for x in stories])
    return data


class AgentContractTests(unittest.TestCase):
    def test_page_chrome_and_missing_quality_are_rejected(self):
        for debris in ('.embed-container{position:relative}', 'title document.querySelectorAll("body")', '<img src="/photo.jpg">', '![video](https://www.youtube.com/watch?v=example)'):
            data=artifact(); data['reader_markdown']=debris
            self.assertTrue(agent.validate(data,'reading_article'))
        data=artifact(); del data['editorial']['quality']
        self.assertTrue(agent.validate(data,'reading_article'))
        data=artifact(); data['editorial']['quality']['fidelity']=0
        self.assertTrue(agent.validate(data,'reading_article'))
        data=artifact(); data['reader_markdown']='Article\n```js\ndocument.querySelectorAll("body")\n```'
        self.assertEqual(agent.validate(data,'reading_article'),[])

    def test_quality_gate_rejects_debris_and_unobserved_media_claim(self):
        data=artifact(); self.assertEqual(agent.validate(data,'reading_article'),[])
        data['audio_script'] += ' &mdash;'
        self.assertTrue(agent.validate(data,'reading_article'))
        data=artifact(); data['editorial']['media']=[{'url':'https://example.org/photo.png','status':'inspected'}]
        self.assertTrue(agent.validate(data,'reading_article'))

    def test_radio_spoken_variant_is_validated(self):
        data=radio_artifact()
        self.assertEqual(agent.validate(data,'radio_episode'),[])
        data['audio_script'] += '<phoneme>wrong</phoneme>'
        self.assertTrue(agent.validate(data,'radio_episode'))

    def test_radio_digest_rejects_single_repeated_unbacked_and_missing_spoken_stories(self):
        for mutate in (
            lambda d: d.update(stories=d['stories'][:1]),
            lambda d: d['stories'][1].update(event_key=d['stories'][0]['event_key']),
            lambda d: d['stories'][1].update(source_urls=['https://example.org/missing']),
            lambda d: d['editorial'].update(claims=d['editorial']['claims'][:1]),
            lambda d: d.update(audio_script=d['stories'][0]['script']),
        ):
            data=radio_artifact(); mutate(data)
            self.assertTrue(agent.validate(data,'radio_episode'))

    def test_briefs_and_sparse_days_preserve_eligible_coverage(self):
        data=radio_artifact()
        data['stories'][1].update(tier='brief', script='另一家公司公布新的芯片，具体上市时间尚未确定。')
        data['script']=data['audio_script']='\n'.join(x['script'] for x in data['stories'])
        self.assertEqual(agent.validate(data,'radio_episode'),[])
        data['stories']=data['stories'][:1]
        data['script']=data['audio_script']=data['stories'][0]['script']
        self.assertTrue(agent.validate(data,'radio_episode'))
        data['editorial']['radio_coverage']['eligible_event_keys']=['event-0']
        self.assertEqual(agent.validate(data,'radio_episode'),[])
        data['stories'][0].update(tier='brief', script='另一家公司公布新的芯片，具体上市时间尚未确定。')
        data['script']=data['audio_script']=data['stories'][0]['script']
        self.assertEqual(agent.validate(data,'radio_episode'),[])
        data['stories'][0]['tier']='unknown'
        self.assertTrue(agent.validate(data,'radio_episode'))

    def test_complete_program_validates_every_section(self):
        data=dict(title='早间新闻', opening='这里是早间新闻。', closing='感谢收听，下期再见。', sections=[radio_artifact()])
        self.assertEqual(agent.validate(data, 'radio_program'), [])
        data['sections'][0]['stories']=[]
        self.assertTrue(agent.validate(data, 'radio_program'))

    def test_state_is_private_and_atomic(self):
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'state.json'
            agent.save(path,{'lease_token':'private'})
            self.assertEqual(path.stat().st_mode & 0o777,0o600)
            self.assertEqual(agent.read(path),{'lease_token':'private'})
            self.assertEqual(len(list(Path(directory).iterdir())),1)

    def test_worker_names_do_not_change_logical_job_id_and_lost_lease_recovers(self):
        with tempfile.TemporaryDirectory() as directory:
            args=argparse.Namespace(state=str(Path(directory)/'state.json'),job_type='radio_episode',date='2026-09-21',slot='morning-Tech')
            class Fake:
                def __init__(self): self.job=None; self.leases=0
                def call(self,path,body=None,retry=True):
                    if path.endswith('capabilities'): return {'schema_version':2}
                    if path.endswith('/jobs'): return {}
                    if path.endswith('context'): return {'job':self.job or {'status':'queued'}}
                    if path.endswith('lease'):
                        self.leases+=1
                        self.job={'status':'leased','lease_owner':body['agent_id'],'lease_token':'t','lease_expires_at':9999999999}
                        raise RuntimeError('reply lost after server leased job')
            api=Fake()
            with patch.object(agent,'nexus',return_value=api):
                with self.assertRaises(RuntimeError): agent.begin(args)
                agent.begin(args)
            state=agent.read(args.state)
            self.assertEqual(state['lease_token'],'t')
            self.assertEqual(api.leases,1)
            self.assertEqual(state['job_id'],'freshloop:v2:radio_episode:2026-09-21:morning-Tech')

    def test_unknown_submit_keeps_digest_and_blocks_changed_artifact(self):
        with tempfile.TemporaryDirectory() as directory:
            state_path=Path(directory)/'state.json'; draft=Path(directory)/'draft.json'
            agent.save(state_path,{'job_id':'j','job_type':'reading_article','lease_token':'t'})
            agent.save(draft,artifact())
            args=argparse.Namespace(command='submit',state=str(state_path),artifact=str(draft))
            class Fake:
                def call(self,*args,**kwargs): raise RuntimeError('unknown outcome')
            with patch.object(agent,'nexus',return_value=Fake()):
                with self.assertRaises(RuntimeError): agent.dispatch(args)
                self.assertIn('pending_hash',agent.read(state_path))
                changed=artifact(); changed['title']='Changed'; agent.save(draft,changed)
                with self.assertRaisesRegex(ValueError,'pending submission differs'): agent.dispatch(args)

    def test_400_allows_correcting_the_draft(self):
        with tempfile.TemporaryDirectory() as directory:
            state_path=Path(directory)/'state.json'; draft=Path(directory)/'draft.json'
            agent.save(state_path,{'job_id':'j','job_type':'reading_article','lease_token':'t'})
            agent.save(draft,artifact())
            args=argparse.Namespace(command='submit',state=str(state_path),artifact=str(draft))
            class Fake:
                def call(self,*args,**kwargs): raise agent.ApiError(400,'invalid draft')
            with patch.object(agent,'nexus',return_value=Fake()):
                with self.assertRaises(agent.ApiError): agent.dispatch(args)
            self.assertNotIn('pending_hash',agent.read(state_path))

    def test_transport_rejects_plaintext_remote_credentials(self):
        with self.assertRaises(ValueError): agent.Api('http://remote.example','key','X-NEXUS-KEY')


if __name__=='__main__': unittest.main()
