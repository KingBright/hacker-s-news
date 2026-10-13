"""Deterministic article extraction/cleaning; retains raw evidence, never summarizes."""
import hashlib
from html.parser import HTMLParser
import re
import unicodedata
from urllib.parse import urljoin, urlparse

BLOCKED={'script','style','nav','noscript','template','svg','form','button'}
VOID={'area','base','br','col','embed','hr','img','input','link','meta','param','source','track','wbr'}
BREAKS={'p','div','section','li','h1','h2','h3','h4','h5','h6','tr','pre','blockquote','br','hr'}
NOISE_CLASSES={'advertisement','ad-container','ad-slot','adsbygoogle','cookie-banner','cookie-consent','social-share-buttons','newsletter-signup'}
PROTECTED={'pre','code','blockquote','sup'}
NUMBERS=r'[-+−]?\d+(?:[.,:/-]\d+)*(?:\s*(?:%|µm|μm|nm|mm|GHz|MHz|kHz|GB|MB|°C|毫秒|纳米|人|周|个月|磅|倍))?'
def sha(text):return hashlib.sha256(text.encode('utf-8','surrogatepass')).hexdigest()
def numbers(text):return {''.join(x.split()) for x in re.findall(NUMBERS,text)}

def clean_text(text,protected_ranges=()):
    """Conservative exact adjacent-paragraph dedup; code/quotes/footnotes stay."""
    changes=[];warnings=[];lines=[];offset=0;previous=None
    for raw in text.splitlines(keepends=True):
        protected=any(a<offset+len(raw) and b>offset for a,b in protected_ranges)
        value=raw.rstrip('\r\n')
        normalized=unicodedata.normalize('NFC',value)
        if normalized!=value:changes.append({'rule':'NFC','start_char':offset,'original_sha256':sha(value)})
        value=normalized
        invalid=[(i,ord(c)) for i,c in enumerate(value) if (ord(c)<32 and c not in '\t\r\n') or 127<=ord(c)<=159 and c!='\x85']
        if invalid:
            changes.append({'rule':'invalid_control','start_char':offset,'codepoints':sorted({n for _,n in invalid})})
            value=''.join(c for c in value if not ((ord(c)<32 and c not in '\t\r\n') or 127<=ord(c)<=159 and c!='\x85'))
        value=value.replace('\x85','\n')
        # Preserve indentation and unit/code spacing across repeated cleaning.
        value=value.rstrip()
        if value.strip():
            if not protected and len(value)>=40 and previous==value:
                changes.append({'rule':'exact_adjacent_duplicate','start_char':offset,'end_char':offset+len(raw),'removed_sha256':sha(raw),'retained_paragraph_sha256':sha(value)})
            else:
                lines.append(value);previous=None if protected else value
        offset+=len(raw)
    result='\n'.join(lines)
    if '\ufffd' in result or any(0xD800<=ord(c)<=0xDFFF for c in result):warnings.append('invalid_encoding_requires_original_inspection')
    if numbers(text)!=numbers(result):warnings.append('numeric_tokens_changed_requires_original_inspection')
    if len(''.join(result.split()))<40:warnings.append('empty_or_short_body_requires_original_inspection')
    return result,{'version':1,'input_sha256':sha(text),'cleaned_sha256':sha(result),'changes':changes,'warnings':warnings,
                   'truncated':False,'semantic_truth_verified':False}

class ArticleParser(HTMLParser):
    def __init__(self,prefer_article_sections=False):
        super().__init__(convert_charrefs=True);self.prefer_article_sections=prefer_article_sections
        self.stack=[];self.blocked=[];self.protected=[];self.active=[];self.candidates=[];self.anchors=[];self.excluded=[]
    def append(self,text):
        if any(self.blocked):
            for row in self.excluded:
                if row.get('active') and row['depth']<=len(self.stack):row['characters']+=len(text)
            return
        for capture in self.active:
            start=capture['length'];capture['parts'].append(text);capture['length']+=len(text)
            if any(self.protected):capture['protected_ranges'].append((start,start+len(text)))
        for anchor in self.anchors:anchor['parts'].append(text)
    def handle_starttag(self,tag,attrs):
        attrs=dict(attrs);classes=attrs.get('class','').split();in_article=any(c['rank']>=3 for c in self.active)
        blocked=tag in BLOCKED or (tag in {'header','footer','aside'} and not in_article) or bool(set(classes)&NOISE_CLASSES)
        if blocked and not any(self.blocked):
            self.excluded.append({'tag':tag,'classes':classes,'line':self.getpos()[0],'column':self.getpos()[1],'depth':len(self.stack)+1,'characters':0,'contains_structural_content':False,'active':True})
        if any(self.blocked) and tag in {'h1','h2','h3','table','sup'}:
            for row in self.excluded:
                if row.get('active') and row['depth']<=len(self.stack):row['contains_structural_content']=True
        post_body='post-body' in classes and 'entry-content' in classes
        section=self.prefer_article_sections and tag=='section' and self.stack and self.stack[-1]=='article'
        rank=4 if 'blog_post' in classes or post_body or section else 3 if tag in {'article','dt-article'} else 2 if tag=='main' else 1 if tag=='body' else 0
        root='post-body' if post_body else 'article-section' if section else 'blog_post' if rank==4 else tag
        if rank and not blocked and not any(self.blocked):
            self.active.append({'rank':rank,'root':root,'depth':len(self.stack)+1,'parts':[],'length':0,'protected_ranges':[],'links':[]})
        if tag in BREAKS:self.append('\n')
        elif tag in {'td','th'}:self.append(' | ')
        if tag=='a' and attrs.get('href') and not blocked and not any(self.blocked):
            self.anchors.append({'href':attrs['href'],'parts':[],'captures':list(self.active),'depth':len(self.stack)+1})
        if tag not in VOID:
            self.stack.append(tag);self.blocked.append(blocked)
            self.protected.append(tag in PROTECTED or 'footnotes' in classes or attrs.get('role') in {'doc-footnote','doc-endnotes','doc-noteref'} or attrs.get('id','').startswith(('fn-','fnref-')))
    def handle_endtag(self,tag):
        if tag not in self.stack:return
        if tag in BREAKS:self.append('\n')
        index=len(self.stack)-1-self.stack[::-1].index(tag)
        self.stack=self.stack[:index];self.blocked=self.blocked[:index];self.protected=self.protected[:index]
        for row in self.excluded:
            if row['depth']>len(self.stack):row['active']=False
        completed=[a for a in self.anchors if a['depth']>len(self.stack)]
        for a in completed:
            row={'href':a['href'],'text':''.join(a['parts']).strip()}
            for capture in a['captures']:capture['links'].append(row)
        self.anchors=[a for a in self.anchors if a not in completed]
        finished=[c for c in self.active if c['depth']>len(self.stack)]
        self.candidates.extend(finished);self.active=[c for c in self.active if c not in finished]
    def handle_data(self,data):self.append(data)

def article_view(record):
    raw=record.get('original_html')
    if not isinstance(raw,str) or not raw.strip():return {'available':False,'reason':'no cached HTML; verify feed excerpt separately'}
    entry=record.get('entry') or {};link=entry.get('link','') if isinstance(entry,dict) else ''
    parser=ArticleParser(prefer_article_sections=isinstance(link,str) and urlparse(link).hostname=='seangoedecke.com')
    parser.feed(raw);parser.close()
    candidates=[c for c in parser.candidates+parser.active if ''.join(c['parts']).strip()]
    if not candidates:return {'available':False,'reason':'no article/main/body container; inspect original source','excluded_dom':parser.excluded}
    rank=max(c['rank'] for c in candidates);selected=[c for c in candidates if c['rank']==rank]
    if len(selected)>1:
        articles=[c for c in candidates if c['root']=='article']
        roots={'article'} if all(c['root']=='article-section' for c in selected) and len(articles)==1 else {'main','body'}
        enclosing=[c for c in candidates if c['root'] in roots]
        if enclosing:
            outer=max(c['rank'] for c in enclosing);selected=[c for c in enclosing if c['rank']==outer]
    texts=[];audit=[];links=[];warnings=[]
    for c in selected:
        text,report=clean_text(''.join(c['parts']),c['protected_ranges']);texts.append(text);audit.append(report);warnings.extend(report['warnings'])
        for row in c['links']:
            absolute=urljoin(link,row['href']);scheme=urlparse(absolute).scheme
            if scheme not in {'http','https','ftp','mailto','tel'}:
                warnings.append('unsupported_link_scheme_not_followed')
                links.append({**row,'url':None,'safe_to_follow':False,'footnote_reference':False});continue
            links.append({**row,'url':absolute,'footnote_reference':bool(re.search(r'#(?:fn(?:ref)?[-_]|footnote)',row['href'],re.I))})
    # Explicit ads are removable, but a large/structural ad-labelled subtree is
    # suspicious and must be checked against raw HTML before model dispatch.
    if any(set(r['classes'])&NOISE_CLASSES and (r['characters']>400 or r['contains_structural_content']) for r in parser.excluded):
        warnings.append('excluded_noise_subtree_may_contain_article_content')
    text='\n\n'.join(texts)
    metadata={k:entry[k] for k in ('title','link','pub_date','source_name') if k in entry}
    return {'available':bool(text.strip()),'text':text,'root':selected[0]['root'],'containers':len(selected),'characters':len(text),'truncated':False,'mechanical':True,
            'requires_editorial_verification':True,'metadata':metadata,'links':links,
            'cleaning':{'version':1,'original_html_sha256':sha(raw),'cleaned_sha256':sha(text),'containers':audit,'excluded_dom':parser.excluded,'warnings':sorted(set(warnings)),'requires_inspection':any(w!='unsupported_link_scheme_not_followed' for w in warnings),'semantic_truth_verified':False},
            'warning':'Deterministic extraction/cleaning is not factual verification. Inspect flagged deletion/encoding/short-body cases against retained original_html.'}
