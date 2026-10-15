"""Transport/provenance tests only. Synthetic logs do not prove LLM quality."""
import copy,json,sqlite3,tempfile,unittest,uuid
from pathlib import Path
from unittest.mock import patch
import freshloop_agent as agent
import review_comparison as review
import review_batches as batches
import native_review as native
import delivery
from test_freshloop_agent import radio_artifact,reviewed_file,sync_receipts

class HandoffTests(unittest.TestCase):
    def test_readable_long_unicode_is_complete_and_ranges_bound(self):
        with tempfile.TemporaryDirectory() as tmp:
            text=('原文事实与限制，不能截断。'+('long-source-details '*30)+'\n')*1500
            view=batches.write_readable(text,Path(tmp),'long')
            self.assertEqual(''.join(text.split()),''.join(Path(view['file']).read_text().split()))
            ranges=[r for p in view['read_plan_files'] for r in json.loads(Path(p).read_text())['ranges']]
            self.assertEqual([n for r in ranges for n in range(r['start_line'],r['end_line']+1)],list(range(1,view['lines']+1)))
            self.assertTrue(all(r['bytes']<=24000 and r['end_line']-r['start_line']<80 for r in ranges))

    def test_parts_concatenate_actual_judgments_and_stale_binding_fails(self):
        part=dict(role='facts',artifact_sha256='a',input_file='f',input_sha256='s',comparisons=[dict(unit='1',conclusion='revise')],blocking=['real omission'],warnings=[],summary='actual judgment',checked_artifacts=['a'])
        other=copy.deepcopy(part);other['comparisons']=[dict(unit='2',conclusion='unverifiable')]
        before=copy.deepcopy([part,other]);result=batches.merge_parts([part,other],'facts')
        self.assertEqual(result['comparisons'],part['comparisons']+other['comparisons'])
        self.assertEqual([part,other],before)
        other['input_sha256']='different'
        with self.assertRaises(ValueError):batches.merge_parts([part,other],'facts')

    def test_metadata_batches_small_complete_and_production_needs_real_origin(self):
        with tempfile.TemporaryDirectory() as tmp:
            path=Path(tmp)/'review.json';data=radio_artifact();reviewed_file(path,data)
            request=agent.read(Path(tmp)/'review-inputs.request.json');index=agent.read(request['review_batches'])
            expected=review.units(data);actual=[]
            for row in index['batches']:
                raw=Path(row['file']).read_text();self.assertLessEqual(len(raw.splitlines()),700);self.assertLessEqual(len(raw.encode()),32000)
                batch=json.loads(raw);self.assertLessEqual(len(batch['units']),4);actual.extend(u['unit'] for u in batch['units'])
            self.assertEqual(actual,[u['unit'] for u in expected])
            with self.assertRaisesRegex(review.EditorialError,'actual reviewer outputs'):agent.require_final_review(data,path,production=True)

    def test_editor_template_claiming_native_pass_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            path=Path(tmp)/'review.json';data=radio_artifact();reviewed_file(path,data);r=agent.read(path)
            for role in r['reviews']:
                for c in role['comparisons']:
                    c['assessment']=role['role']+'：已核对'+c['unit']+'，来源主体、事件、日期和限制全部保留。'
                    for e in c['evidence']:e['observation']='原文'+e['source_id']+'与终稿对应，含义一致。'
            sync_receipts(path,r)
            with self.assertRaisesRegex(review.EditorialError,'templates repeated'):agent.require_final_review(data,path)

    def test_synthetic_native_reads_before_actual_reply_and_mutations(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);path=root/'review.json';data=radio_artifact();reviewed_file(path,data)
            req=agent.read(root/'review-inputs.request.json');index=agent.read(req['review_batches']);batch_file=Path(index['batches'][0]['file']);batch=agent.read(batch_file)
            state=root/'native';state.mkdir();ident=str(uuid.uuid4());parent=str(uuid.uuid4())
            with sqlite3.connect(state/'conversation_summaries.db') as conn:
                conn.execute('CREATE TABLE conversation_summaries (conversation_id TEXT,agent_name TEXT,parent_conversation_id TEXT)')
                conn.execute('INSERT INTO conversation_summaries VALUES (?,?,?)',(ident,native.PROFILES['facts'],parent))
            log=state/'brain'/ident/'.system_generated/logs/transcript_full.jsonl';log.parent.mkdir(parents=True)
            rows=[]
            files=[(str(batch_file),1,len(batch_file.read_text().splitlines()))]
            for d in batch['sources']+batch['units']:
                for p in d['readable']['read_plan_files']:
                    for r in agent.read(p)['ranges']:files.append((d['readable']['file'],r['start_line'],r['end_line']))
            for file,start,end in files:
                rows.extend([dict(step_index=len(rows),type='PLANNER_RESPONSE',tool_calls=[dict(name='view_file',args=dict(AbsolutePath=file,StartLine=start,EndLine=end))]),dict(step_index=len(rows)+1,type='GENERIC',status='DONE',content=f'Showing lines {start} to {end}')])
            raw={k:batch[k] for k in ('artifact_sha256','input_file','input_sha256')};raw.update(role='facts',comparisons=[dict(unit=u['unit'],conclusion='revise',assessment='real raw fixture opinion') for u in batch['units']],blocking=['real fixture issue'],warnings=[],summary='test fixture')
            reply=dict(step_index=len(rows),type='PLANNER_RESPONSE',tool_calls=[dict(name='send_message',args=dict(Recipient=parent,Message=json.dumps(raw)))])
            def write(value):log.write_text(''.join(json.dumps(r)+'\n' for r in value))
            with patch.object(native,'state_root',return_value=state):
                write(rows+[reply]);actual,origin=native.native_result(ident,'facts',batch_file)
                self.assertEqual(actual,raw);self.assertEqual(origin['parent_conversation_id'],parent)
                source = Path(batch['sources'][0]['source_file'])
                original_bytes = source.read_bytes()
                changed_record = json.loads(original_bytes)
                changed_record['original_html'] = changed_record['original_html'].replace('<p>', '<p>A later source correction changes the reported comparison. ', 1)
                mutations = [('raw-only', original_bytes + b'\n'),
                             ('body', json.dumps(changed_record).encode())]
                for label, changed_bytes in mutations:
                    with self.subTest(source_change=label):
                        out = root / ('import-' + label + '.json')
                        source.write_bytes(changed_bytes)
                        try:
                            with self.assertRaisesRegex(review.EditorialError, 'Source snapshot changed'):
                                native.import_receipt(ident, 'facts', batch_file, out)
                            self.assertFalse(out.exists())
                            self.assertFalse(out.with_suffix('.origin.json').exists())
                            self.assertEqual(agent.read(out.with_suffix('.unverified-native.json'))['actual_raw_response'], raw)
                        finally:
                            source.write_bytes(original_bytes)
                write([reply]+rows)
                with self.assertRaisesRegex(review.EditorialError,'batch metadata'):native.native_result(ident,'facts',batch_file)
                bad=copy.deepcopy(rows);bad[1]['content']+=' Content truncated';write(bad+[reply])
                with self.assertRaisesRegex(review.EditorialError,'batch metadata'):native.native_result(ident,'facts',batch_file)
                bad=copy.deepcopy(reply);bad['tool_calls'][0]['args']['Recipient']=str(uuid.uuid4());write(rows+[bad])
                with self.assertRaisesRegex(review.EditorialError,'No actual reviewer'):native.native_result(ident,'facts',batch_file)
                write(rows+[reply])
                with self.assertRaisesRegex(review.EditorialError,'actual independent reviewer'):native.native_result(ident,'listening',batch_file)
                changed=copy.deepcopy(batch);changed['sources']=[];agent.save(batch_file,changed)
                with self.assertRaisesRegex(review.EditorialError,'every source'):native.native_result(ident,'facts',batch_file)
                changed=copy.deepcopy(batch);changed['sources'][0]['readable']['lines']=0;agent.save(batch_file,changed)
                with self.assertRaisesRegex(review.EditorialError,'every line'):native.native_result(ident,'facts',batch_file)
                agent.save(batch_file,batch)
                view=Path(batch['sources'][0]['readable']['file']);view.write_text(view.read_text()+'invented fact')
                with self.assertRaisesRegex(review.EditorialError,'differs from'):native.native_result(ident,'facts',batch_file)

    def test_budget_deferral_cannot_be_content_rejection(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp)/'agent-runs';root.mkdir();inventory={'pagination':{'complete':True},'sources':{'items':[{'id':'sixth'}]}}
            agent.save(root/'sources.json',inventory)
            ledger=dict(inventory_file=str(root/'sources.json'),inventory_sha256=review.digest(inventory),candidates=[dict(source_id='sixth',outcome='not-selected',reason='编辑预算不足，保留至后续版次核验')])
            agent.save(root/'reading-ledger.json',ledger);manifest=dict(date='2026-10-13',edition='早间',reading_ledger=str(root/'reading-ledger.json'),outputs=[])
            self.assertIn('must remain pending',''.join(delivery.reading_scope(manifest,root/'manifest.json')))
            ledger['candidates'][0].update(outcome='pending',deferral_reason='editorial_budget_exhausted');agent.save(root/'reading-ledger.json',ledger)
            self.assertIn('未完成候选',''.join(delivery.reading_scope(manifest,root/'manifest.json')))
            ledger['candidates'][0].update(outcome='not-selected',reason='正文是已发布旧文且没有实质更新');ledger['candidates'][0].pop('deferral_reason');agent.save(root/'reading-ledger.json',ledger)
            self.assertEqual(delivery.reading_scope(manifest,root/'manifest.json'),[])
if __name__=='__main__':unittest.main()
