import unittest
from article_view import article_view

class ArticleViewTests(unittest.TestCase):
    def test_article_excludes_scripts_and_navigation_and_keeps_late_facts(self):
        html='<html><head><style>'+('noise ' * 1000)+'</style></head><body><nav>Menu</nav><article><h1>Company release</h1><p>September 24, 2026</p><p>First fact.</p><script>credential noise</script><p>Late limitation.</p><table><tr><td>Result</td><td>69.9%</td></tr></table></article><aside>Other news</aside></body></html>'
        view=article_view({'original_html':html})
        self.assertEqual(view['root'],'article')
        self.assertNotIn('noise',view['text']);self.assertNotIn('Menu',view['text'])
        self.assertIn('Late limitation.',view['text']);self.assertIn('69.9%',view['text'])
        self.assertFalse(view['truncated']);self.assertTrue(view['requires_editorial_verification'])

    def test_known_blog_body_beats_main_sidebar_and_preserves_authorship(self):
        view=article_view({'original_html':'<body><main><div class="blog_nav">Previous</div><div class="blog_post card"><h2>Title</h2><span>by Mike</span><p>Full body</p></div><section>Latest unrelated news</section></main></body>'})
        self.assertEqual(view['root'],'blog_post')
        self.assertIn('by Mike',view['text']);self.assertNotIn('unrelated',view['text'])

    def test_feature_cards_do_not_hide_installation_limits(self):
        html='<body><main><h1>Project</h1><article>Feature one</article><article>Feature two</article><p>Installation erases archived files; back up first.</p></main></body>'
        view=article_view({'original_html':html})
        self.assertEqual(view['root'],'main')
        self.assertIn('back up first',view['text'])

    def test_custom_article_and_explicit_unavailable(self):
        self.assertEqual(article_view({'original_html':'<body><dt-article><p>Research body</p></dt-article></body>'})['root'],'dt-article')
        self.assertFalse(article_view({})['available'])
        self.assertFalse(article_view({'original_html':'<script>ignore</script>'})['available'])


    def test_cleaning_idempotency_encoding_and_duplicate_fact_protection(self):
        from article_view import clean_text,numbers
        paragraph='实验包含11人，4周后比较40%，尚未证明功能恢复。' * 2
        text='Cafe\u0301\x00 — N2P, −2.5 µm, 3.84倍。\n'+paragraph+'\n'+paragraph+'\n'
        cleaned,report=clean_text(text)
        self.assertEqual(clean_text(cleaned)[0],cleaned)
        self.assertIn('Café',cleaned);self.assertNotIn('\x00',cleaned)
        self.assertEqual(numbers(text),numbers(cleaned));self.assertEqual(cleaned.count(paragraph),1)
        self.assertIn('尚未证明功能恢复',cleaned)
        self.assertTrue(any(x['rule']=='exact_adjacent_duplicate' for x in report['changes']))
        self.assertIn('invalid_encoding',str(clean_text('corrupt \ufffd text')[1]['warnings']))
        self.assertIn('numeric_tokens_changed',str(clean_text('1\x002%')[1]['warnings']))
    def test_header_footnote_links_code_and_limits_are_preserved(self):
        html='<body><nav>Menu</nav><article><header><h1>Title</h1>by Author 2026-10-12</header><p>Specific fact is tentative, 40% vs leading systems, not the old chip.</p><p>Footnote<sup><a href="#fn-1">1</a></sup></p><pre>if ready:\n    return 2\n    return 2</pre><footer class="footnotes"><p id="fn-1">1: only 11 people, four weeks.</p><a href="paper.pdf">Study</a></footer><div class="advertisement">Buy now</div></article></body>'
        record={'original_html':html,'entry':{'title':'Title','link':'https://example.org/post','pub_date':'2026-10-12','source_name':'Author'}}
        view=article_view(record)
        self.assertIn('by Author 2026-10-12',view['text']);self.assertIn('    return 2',view['text'])
        self.assertEqual(view['text'].count('return 2'),2);self.assertIn('only 11 people',view['text'])
        self.assertNotIn('Buy now',view['text']);self.assertNotIn('Menu',view['text'])
        self.assertEqual(view['metadata'],record['entry'])
        self.assertIn({'href':'#fn-1','text':'1','url':'https://example.org/post#fn-1','footnote_reference':True},view['links'])
        self.assertTrue(any(x['url']=='https://example.org/paper.pdf' for x in view['links']))
        self.assertFalse(view['cleaning']['requires_inspection'])
    def test_suspicious_cleanup_and_short_body_are_explicit(self):
        view=article_view({'original_html':'<article><p>Actual article still has a complete and sufficiently long factual sentence.</p><div class="advertisement"><h2>Potential actual limitation</h2><p>Only laboratory results, not deployed.</p></div></article>'})
        self.assertTrue(view['cleaning']['requires_inspection'])
        self.assertIn('may_contain_article_content',str(view['cleaning']['warnings']))
        short=article_view({'original_html':'<article>Short</article>'})
        self.assertTrue(short['cleaning']['requires_inspection'])

if __name__ == '__main__':unittest.main()
