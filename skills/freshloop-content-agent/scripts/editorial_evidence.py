"""Mechanical evidence checks. Factual truth still requires source review."""
def compact(text):
    return ''.join(c.casefold() for c in text if c.isalnum()) if isinstance(text, str) else ''


def usable_claim(claim, source_titles=()):
    if not isinstance(claim, dict):
        return False
    text, evidence = compact(claim.get('claim')), compact(claim.get('evidence'))
    if not text or not evidence:
        return False
    if any(text.startswith(compact(s)) for s in ('内容摘要', '摘要内容', '已核实')):
        return False
    if evidence in {compact(s) for s in ('根据来源文章内容确认', '原网页确认事实', '原文片段', '原文引文', '根据以上精选文章汇总')}:
        return False
    # A source title alone establishes neither the development nor supporting facts.
    if any(evidence == compact(title) or text == compact(title) for title in source_titles if title):
        return False
    return True


def validate_evidence(artifact, job_type):
    review = artifact.get('editorial', {})
    if not isinstance(review, dict):
        return []
    claims = review.get('claims', [])
    if not isinstance(claims, list):
        return []
    sources = artifact.get('sources', [])
    titles = [s.get('title', '') for s in sources if isinstance(s, dict)] if isinstance(sources, list) else []
    errors = []
    if any(not usable_claim(c, titles) for c in claims):
        errors.append('claims need specific facts and actual supporting passages; title-only or confirmation placeholders are not evidence')
    if job_type == 'radio_episode':
        for story in artifact.get('stories', []) if isinstance(artifact.get('stories'), list) else []:
            if not isinstance(story, dict):
                continue
            urls = story.get('source_urls', [])
            if not isinstance(urls, list):
                continue
            supported = [c for c in claims if usable_claim(c, titles) and c.get('source_url') in urls]
            if story.get('tier') == 'major' and (len({compact(c.get('claim')) for c in supported}) < 2 or len({compact(c.get('evidence')) for c in supported}) < 2):
                errors.append('major story needs distinct source-backed development and supporting detail; return to the source, never pad or relabel importance to pass')
    if job_type in ('reading_article', 'weekly_digest'):
        needed = 2 if job_type == 'reading_article' else 3
        supported = [c for c in claims if usable_claim(c, titles)]
        if len({compact(c.get('claim')) for c in supported}) < needed or len({compact(c.get('evidence')) for c in supported}) < needed:
            errors.append('Reading/weekly needs distinct substantive facts and supporting passages, not title-only summaries; verify article arguments and examples')
    if job_type == 'reading_article':
        if compact(artifact.get('reader_markdown')) == compact(artifact.get('compressed_markdown')):
            errors.append('reader and compressed versions are identical; preserve the original structure and synthesize separately')
    return errors
