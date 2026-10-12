import copy
import unittest
from unittest.mock import Mock
from source_pagination import collect_sources
from editorial_evidence import validate_evidence


class EditorialRegressionTests(unittest.TestCase):
    def test_major_needs_development_and_detail_without_word_quota(self):
        a = {'stories':[{'tier':'major','source_urls':['https://source.test/a']}],
             'editorial':{'claims':[{'claim':'Acme 发布补丁','source_url':'https://source.test/a','evidence':'Acme released a patch.'}]}}
        self.assertTrue(validate_evidence(a, 'radio_episode'))
        a['editorial']['claims'].append({'claim':'只影响旧版远程管理接口','source_url':'https://source.test/a','evidence':'Only the legacy remote interface is affected.'})
        self.assertEqual(validate_evidence(a, 'radio_episode'), [])
        a['editorial']['claims'].append(copy.deepcopy(a['editorial']['claims'][0]))
        a['editorial']['claims'].pop(1)
        self.assertTrue(validate_evidence(a, 'radio_episode'))
        a['stories'][0]['tier'] = 'brief'
        self.assertEqual(validate_evidence(a, 'radio_episode'), [])

    def test_bad_scheduled_evidence_and_identical_compression(self):
        for evidence in ['原网页确认事实','根据来源文章内容确认','原文片段']:
            a={'editorial':{'claims':[{'claim':'具体公司发布新产品','evidence':evidence}]}}
            self.assertTrue(validate_evidence(a,'reading_article'))
        a={'reader_markdown':'正文 **内容**','compressed_markdown':'正文内容','editorial':{'claims':[]}}
        self.assertTrue(validate_evidence(a,'reading_article'))
        a={'sources':[{'title':'SQLAlchemy 2.1.0 Released'}],
           'editorial':{'claims':[{'claim':'内容摘要：SQLAlchemy发布','evidence':'SQLAlchemy 2.1.0 Released'}]}}
        self.assertTrue(validate_evidence(a,'radio_episode'))


class SourcePaginationTests(unittest.TestCase):
    def page(self, offset, ids, total):
        return {'categories':[{'name':'Tech'}], 'sources':{'offset':offset,'total':total,'items':[{'id':i} for i in ids]}}

    def test_all_pages_and_cache_time_are_preserved(self):
        api=Mock(); api.call.side_effect=[self.page(0,['a','b'],3),self.page(2,['c'],3)]
        result=collect_sources(api,'radio',since=123)
        self.assertEqual([r['id'] for r in result['sources']['items']], ['a','b','c'])
        self.assertTrue(result['pagination']['complete'])
        self.assertIn('offset=2',api.call.call_args.args[0])
        self.assertIn('since=123',api.call.call_args.args[0])
        self.assertEqual(result['categories'],[{'name':'Tech'}])

    def test_repeat_changed_total_premature_end_and_deadline_stop(self):
        for pages in ([self.page(0,['a'],3),self.page(1,['a'],3)],
                      [self.page(0,['a'],3),self.page(1,['b'],4)],
                      [self.page(0,[],3)]):
            api=Mock();api.call.side_effect=pages
            with self.assertRaises(RuntimeError):collect_sources(api)
        with self.assertRaises(RuntimeError):collect_sources(Mock(),deadline_seconds=0)

    def test_resume_is_not_whole_window_and_page_budget_is_bounded(self):
        api=Mock();api.call.return_value=self.page(2,['c'],3)
        self.assertFalse(collect_sources(api,offset=2)['pagination']['complete'])
        api.call.return_value=self.page(0,['a'],3)
        with self.assertRaises(RuntimeError):collect_sources(api,max_pages=1)
        api.call.return_value=self.page(0,[],0)
        self.assertTrue(collect_sources(api)['pagination']['complete'])


if __name__ == '__main__':
    unittest.main()
