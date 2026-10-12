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
            'compressed_markdown':'中文摘要','audio_script':'科研团队公布了新的电池循环测试数据，样品在实验室完成一千次充放电。测试温度保持恒定，尚不能据此推断低温户外条件下的使用寿命，量产进度仍待确认。',
            'editorial':{'language':'zh-CN','ready':True,'dedup_checked':True,
                         'quality':dict.fromkeys(('fidelity','selection','listening','structure','media'),2),
                         'claims':[{'claim':'样品完成一千次循环','source_url':'https://example.org/article','evidence':'The sample completed 1000 cycles in controlled laboratory conditions.'},{'claim':'测试不涵盖低温户外条件','source_url':'https://example.org/article','evidence':'The tests did not include low-temperature outdoor conditions.'}],'media':[]}}


def radio_artifact():
    data=artifact()
    stories=[dict(event_key=f'event-{i}',tier='major',title=f'新闻{i}',script=['科研团队公布了新的电池循环测试数据，样品在实验室完成一千次充放电。测试温度保持恒定，尚不能据此推断低温户外条件下的使用寿命，量产进度仍待确认。', '软件维护者修复了一处权限校验漏洞，并发布升级说明。受影响的是开启远程管理的旧版本，管理员需要检查配置，官方尚未披露利用规模。', '当地交通部门宣布下周试行公交新线路，连接火车站与科技园。试运营持续一个月，票价与现有线路相同，后续将根据乘客反馈调整班次。'][i],source_urls=[f'https://example.org/{i}']) for i in range(3)]
    data.update(category='Tech',stories=stories,script='\n'.join(x['script'] for x in stories))
    data['audio_script']=data['script']
    data['sources']=[dict(url=x['source_urls'][0],title=x['title'],summary='来源证据') for x in stories]
    data['editorial']['claims']=[dict(claim=f'{x["event_key"]}事实{j}',source_url=x['source_urls'][0],evidence=f'Test source passage supports event {i}, fact {j}.') for i,x in enumerate(stories) for j in range(2)]
    data['editorial']['radio_coverage'] = dict(reviewed=True, eligible_event_keys=[x['event_key'] for x in stories])
    return data


def sync_receipts(path, result):
    """Modify only synthetic test receipts, preserving the production assembler contract."""
    from review_comparison import digest
    result['receipts'] = []
    for row in result['reviews']:
        for key in ('artifact_sha256', 'input_file', 'input_sha256'):
            row[key] = result[key]
        receipt = Path(path).parent / (row['role'] + '-receipt.json')
        agent.save(receipt, row)
        result['receipts'].append(dict(role=row['role'], file=str(receipt.resolve()), sha256=digest(row)))
    agent.save(path, result)


def reviewed_file(path, data):
    # Synthetic binding fixture only; semantic regressions use independent source fixtures.
    import hashlib
    from html import escape
    from review_comparison import prepare_inputs, read, units, DIMENSIONS
    root=Path(path).parent
    draft=root/'review-artifact.json'; agent.save(draft,data)
    wanted={url for unit in units(data) for url in unit['source_urls']}
    files=[]
    for url in sorted(wanted):
        ident=hashlib.sha256(url.encode()).hexdigest()
        body='\n'.join(unit['text'] for unit in units(data) if url in unit['source_urls'])
        source=root/(ident+'.json')
        agent.save(source,dict(id=ident,entry=dict(link=url),original_html='<article><p>'+escape(body)+'</p></article>',fetch_error=None))
        files.append(str(source))
    packet_path=root/'review-inputs.json'
    prepared=prepare_inputs(draft,files,packet_path); packet=read(packet_path)
    rows=[]
    for role in ('facts','listening'):
        comparisons=[]
        for unit in packet['units']:
            sources=[s for s in packet['sources'] if s['url'] in unit['source_urls']]
            comparisons.append(dict(unit=unit['unit'],source_ids=[s['source_id'] for s in sources],
                checked_dimensions=sorted(DIMENSIONS),conclusion='pass',differences=[],
                assessment=role + ': synthetic source and complete final unit are bound',
                evidence=[dict(source_id=s['source_id'],source_excerpt=unit['text'],draft_excerpt=unit['text'],
                               observation=role + ': complete fixture passage corresponds to this unit') for s in sources]))
        rows.append(dict(role=role,blocking=[],checked_artifacts=['draft.json'],summary='Synthetic '+role+' binding fixture',comparisons=comparisons))
    result=dict(artifact_sha256=agent.artifact_digest(data),input_file=prepared['input_file'],input_sha256=prepared['input_sha256'],
                mode='native_subagents',reviews=rows)
    sync_receipts(path, result)
    return str(path)


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

    def test_repeated_passages_and_exact_story_occurrences(self):
        data=radio_artifact()
        # Today's bug: retain original section, then append all stories again.
        data['script'] += '\n' + data['stories'][0]['script']
        self.assertTrue(agent.validate(data, 'radio_episode'))
        text='数字部门宣布下周试行公交新线路，连接火车站与科技园。'
        self.assertTrue(agent.validate_narration(text * 3))
        self.assertEqual(agent.validate_narration(text), [])
        self.assertTrue(agent.validate_narration('这是重要新闻，影响深远且意义重大。'))

    def test_review_invalidated_by_post_review_edit(self):
        with tempfile.TemporaryDirectory() as directory:
            data=radio_artifact(); path=reviewed_file(Path(directory)/'review.json', data)
            agent.require_final_review(data, path)
            data['script'] += '新增了一条消息。'
            with self.assertRaisesRegex(ValueError, 'stale'): agent.require_final_review(data, path)
            with self.assertRaisesRegex(ValueError, 'requires'): agent.require_final_review(data, None)
            path=reviewed_file(Path(directory)/'review.json', data)
            review=agent.read(path); review['reviews'][0]['blocking']=['未核实']; sync_receipts(path, review)
            with self.assertRaisesRegex(ValueError, 'blocking'): agent.require_final_review(data, path)

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
            args=argparse.Namespace(command='submit',state=str(state_path),artifact=str(draft),review_file=reviewed_file(Path(directory)/'review.json', artifact()))
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
            args=argparse.Namespace(command='submit',state=str(state_path),artifact=str(draft),review_file=reviewed_file(Path(directory)/'review.json', artifact()))
            class Fake:
                def call(self,*args,**kwargs): raise agent.ApiError(400,'invalid draft')
            with patch.object(agent,'nexus',return_value=Fake()):
                with self.assertRaises(agent.ApiError): agent.dispatch(args)
            self.assertNotIn('pending_hash',agent.read(state_path))

    def test_transport_rejects_plaintext_remote_credentials(self):
        with self.assertRaises(ValueError): agent.Api('http://remote.example','key','X-NEXUS-KEY')





if __name__=='__main__': unittest.main()
