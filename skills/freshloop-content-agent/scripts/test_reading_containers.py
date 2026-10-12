import unittest
from article_view import article_view

class ReadingContainerTests(unittest.TestCase):
    def test_blogger_post_body_excludes_archive_and_keeps_complete_body(self):
        record={'entry':{'link':'https://oldvcr.blogspot.com/2026/10/article.html'},
                'original_html':"""<body><div>Archive and menu</div><div class="post-body entry-content" itemprop="description articleBody"><p>Alpha Micro restoration diagnosis.</p><h2>Repair limitations</h2><p>Power supply and serial output remain unverified.</p><pre>diagnostic code sample</pre><table><tr><td>voltage</td><td>5V</td></tr></table></div><div>Comment unrelated speculation</div></body>"""}
        view=article_view(record)
        self.assertEqual(view['root'],'post-body')
        self.assertNotIn('Archive',view['text']);self.assertNotIn('unrelated',view['text'])
        self.assertIn('remain unverified',view['text']);self.assertIn('diagnostic code',view['text'])
        self.assertIn('voltage | 5V',view['text']);self.assertFalse(view['truncated'])

    def test_verified_gatsby_body_keeps_footnotes_excludes_related_opinion(self):
        raw="""<body><main><article><section><p>Engineers automated other industries.</p><h2>Limits of the argument</h2><p>The author does not know the net balance.</p><div class="footnotes"><p>Some industries still exist.</p></div></section><p>Subscribe to updates.</p><p>Related post preview.</p><blockquote><p>Unrelated claim about datacenter violence.</p></blockquote></article></main></body>"""
        view=article_view({'entry':{'link':'https://seangoedecke.com/software-engineers-dont-get-to-be-luddites-about-ai/'},'original_html':raw})
        self.assertEqual(view['root'],'article-section')
        self.assertIn('does not know',view['text']);self.assertIn('still exist',view['text'])
        self.assertNotIn('Subscribe',view['text']);self.assertNotIn('violence',view['text'])
        # Do not turn every site's introductory section into the complete article.
        other=article_view({'entry':{'link':'https://example.test/other'},'original_html':raw})
        self.assertEqual(other['root'],'article')

    def test_multiple_article_sections_preserve_later_limits(self):
        view=article_view({'entry':{'link':'https://seangoedecke.com/test'},
             'original_html':'<body><main><article><section><p>Main argument.</p></section><section><p>Important contrary evidence.</p></section></article></main></body>'})
        self.assertEqual(view['root'],'article')
        self.assertIn('contrary evidence',view['text'])

if __name__ == '__main__': unittest.main()
