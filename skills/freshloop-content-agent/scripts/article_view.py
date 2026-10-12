"""Mechanical article view for cached HTML; never invents or summarizes content."""
from html.parser import HTMLParser
import re
from urllib.parse import urlparse

BLOCKED = {'script', 'style', 'nav', 'header', 'footer', 'aside', 'noscript', 'template', 'svg', 'form', 'button'}
VOID = {'area', 'base', 'br', 'col', 'embed', 'hr', 'img', 'input', 'link', 'meta', 'param', 'source', 'track', 'wbr'}
BREAKS = {'p', 'div', 'section', 'li', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'tr', 'pre', 'blockquote', 'br', 'hr'}


class ArticleParser(HTMLParser):
    def __init__(self, prefer_article_sections=False):
        super().__init__(convert_charrefs=True)
        self.prefer_article_sections = prefer_article_sections
        self.stack, self.active, self.candidates = [], [], []

    def append(self, text):
        if not any(tag in BLOCKED for tag in self.stack):
            for capture in self.active:
                capture['parts'].append(text)

    def handle_starttag(self, tag, attrs):
        attributes = dict(attrs)
        classes = attributes.get('class', '').split()
        post_body = 'post-body' in classes and 'entry-content' in classes
        article_section = self.prefer_article_sections and tag == 'section' and self.stack and self.stack[-1] == 'article'
        rank = 4 if 'blog_post' in classes or post_body or article_section else 3 if tag in {'article', 'dt-article'} else 2 if tag == 'main' else 1 if tag == 'body' else 0
        root = 'post-body' if post_body else 'article-section' if article_section else 'blog_post' if rank == 4 else tag
        if rank and not any(t in BLOCKED for t in self.stack):
            self.active.append({'rank': rank, 'root': root, 'depth': len(self.stack) + 1, 'parts': []})
        if tag in BREAKS:
            self.append('\n')
        elif tag in {'td', 'th'}:
            self.append(' | ')
        if tag not in VOID:
            self.stack.append(tag)

    def handle_endtag(self, tag):
        if tag not in self.stack:
            return
        if tag in BREAKS:
            self.append('\n')
        index = len(self.stack) - 1 - self.stack[::-1].index(tag)
        self.stack = self.stack[:index]
        finished = [c for c in self.active if c['depth'] > len(self.stack)]
        self.candidates.extend(finished)
        self.active = [c for c in self.active if c not in finished]

    def handle_data(self, data):
        self.append(data)


def article_view(record):
    raw = record.get('original_html')
    if not isinstance(raw, str) or not raw.strip():
        return {'available': False, 'reason': 'no cached HTML; verify feed excerpt separately'}
    entry = record.get('entry') or {}
    link = entry.get('link', '') if isinstance(entry, dict) else ''
    # Verified Gatsby feed template: the article's direct section contains the
    # complete post and footnotes; recommendations are later article siblings.
    prefer_sections = isinstance(link, str) and urlparse(link).hostname == 'seangoedecke.com'
    parser = ArticleParser(prefer_article_sections=prefer_sections)
    parser.feed(raw); parser.close()
    candidates = parser.candidates + parser.active
    candidates = [c for c in candidates if ''.join(c['parts']).strip()]
    if not candidates:
        return {'available': False, 'reason': 'no article/main/body container; inspect original source'}
    rank = max(c['rank'] for c in candidates)
    selected = [c for c in candidates if c['rank'] == rank]
    # Some landing pages use article for individual feature cards. Preserve
    # the whole main/body rather than dropping installation limits and dates.
    if len(selected) > 1:
        articles = [c for c in candidates if c['root'] == 'article']
        roots = {'article'} if all(c['root'] == 'article-section' for c in selected) and len(articles) == 1 else {'main', 'body'}
        enclosing = [c for c in candidates if c['root'] in roots]
        if enclosing:
            outer_rank = max(c['rank'] for c in enclosing)
            selected = [c for c in enclosing if c['rank'] == outer_rank]
    texts = []
    for candidate in selected:
        lines = [re.sub(r'[\t \r\f\v]+', ' ', line).strip() for line in ''.join(candidate['parts']).split('\n')]
        texts.append('\n'.join(line for line in lines if line))
    text = '\n\n'.join(texts)
    return {'available': True, 'text': text, 'root': selected[0]['root'], 'containers': len(selected),
            'characters': len(text), 'truncated': False, 'mechanical': True,
            'requires_editorial_verification': True,
            'warning': 'Check article boundaries, publisher date, tables and media against original_html; this is extraction, not editorial verification.'}
