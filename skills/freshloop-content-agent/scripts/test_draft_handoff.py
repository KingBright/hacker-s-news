"""Mechanical generation/cleaning contracts; synthetic fixtures are not LLM QA."""
import tempfile,unittest,hashlib
from pathlib import Path
from draft_handoff import prepare,materialize,check,DIMENSIONS
from review_comparison import save,read,digest,EditorialError

class DraftHandoffTests(unittest.TestCase):
    def fixture(self,root):
        text='灯塔实验室公布了一批电池样品的循环测试结果，研究对象是实验室原型而不是已经上市的产品。样品在恒定温度下完成一千次充放电，团队同时记录容量保持率和电压变化，以便判断不同材料的表现。测试只涵盖固定温度和特定电流条件，没有检验低温户外或长期储存表现，不能据此推断量产电池寿命。实验室计划扩大样本并验证异常数据，发布说明同时提醒读者，当前结果尚未经过长期实际使用的独立验证。'
        source=root/'source.json';save(source,{'id':'s1','entry':{'title':'Battery trial','link':'https://example.org/trial','pub_date':'2026-10-12'},'original_html':'<article><p>'+text+'</p></article>'})
        inventory={'pagination':{'complete':True},'categories':[{'name':'科技'}],'sources':{'items':[{'id':'s1','url':'https://example.org/trial'}]}}
        save(root/'inventory.json',inventory)
        ledger={'inventory_file':str(root/'inventory.json'),'inventory_sha256':digest(inventory),'categories':[{'name':'科技','outcome':'covered','reason':'actual scope'}],'candidates':[{'source_id':'s1','category':'科技','outcome':'included-major','event_key':'battery','reason':'new study','importance_reason':'needs experimental limits','required_details':{'actor':'unverified old label','event':'unverified old label','limits':'unverified old label'}}]}
        save(root/'ledger.json',ledger);index=prepare('radio',[source],root/'prepared',root/'ledger.json')
        unit=Path(index['units'][0]['input_file'])
        response={'unit_id':'battery','tier':'major','status':'draft','title':'实验室电池试验','script':text,'actor':'灯塔实验室','event':'公布了一批电池样品的循环测试结果','evidence':[{'dimension':d,'paragraph_ref':'S1P0001','draft_excerpt':text,'explanation':'synthetic binding only'} for d in DIMENSIONS]}
        return unit,response
    def test_complete_sources_once_no_answers_and_bound_tier(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);unit,response=self.fixture(root);payload=materialize(unit)
            self.assertEqual(len(payload['complete_originals']),1);self.assertNotIn('unverified old label',str(payload))
            save(root/'response.json',response);result=check(unit,root/'response.json',root/'check.json')
            self.assertTrue(result['mechanical_valid']);self.assertFalse(result['semantic_truth_verified_by_program'])
            self.assertEqual(result['resolved_evidence'][0]['source_excerpt'],response['script'])
            response['evidence'][0]['paragraph_ref']='S1P9999';save(root/'response.json',response)
            self.assertIn('unknown source paragraph',str(check(unit,root/'response.json',root/'check.json')['errors']))
            response['tier']='brief';save(root/'response.json',response)
            self.assertIn('never downgrade',str(check(unit,root/'response.json',root/'check.json')['errors']))
    def test_null_malformed_response_stale_budget_and_original_are_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);unit,response=self.fixture(root);save(root/'response.json',None)
            self.assertFalse(check(unit,root/'response.json',root/'check.json')['mechanical_valid'])
            response['script']=['bad'];response['evidence']=[None];save(root/'response.json',response)
            self.assertFalse(check(unit,root/'response.json',root/'check.json')['mechanical_valid'])
            data=read(unit);data['budget']['min_nonwhitespace_characters']=1;save(unit,data)
            with self.assertRaisesRegex(ValueError,'changed'):materialize(unit)
    def test_suspicious_or_short_source_stops_before_model_handoff(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);source=root/'short.json';save(source,{'id':'s','entry':{'link':'https://example.org/short'},'original_html':'<article>Short</article>'})
            with self.assertRaisesRegex(EditorialError,'before model dispatch'):prepare('reading',[source],root/'prepared')
            report=read(root/'prepared/sources/s-cleaning.json')
            self.assertEqual(Path(report['source_file']).read_bytes(),source.read_bytes())
            self.assertEqual(report['source_record_sha256'],hashlib.sha256(source.read_bytes()).hexdigest())
            self.assertFalse((root/'prepared/index.json').exists())
if __name__=='__main__':unittest.main()
