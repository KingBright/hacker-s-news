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

if __name__ == '__main__':unittest.main()
