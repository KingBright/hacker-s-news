"""Bounded metadata pagination; no editorial generation or source fetching."""
import time
import urllib.parse


def collect_sources(api, product_line=None, offset=0, since=0, max_pages=400, deadline_seconds=120):
    if offset < 0 or since < 0 or not 1 <= max_pages <= 400:
        raise ValueError('invalid source pagination bounds')
    deadline = time.monotonic() + deadline_seconds
    items, seen, totals = [], set(), []
    categories = None
    start = offset
    for page in range(max_pages):
        if time.monotonic() >= deadline:
            raise RuntimeError('source pagination deadline exceeded; coverage remains incomplete')
        query = {k: v for k, v in dict(product_line=product_line, offset=offset, since=since, limit=100).items() if v is not None}
        response = api.call('/api/agent/sources?' + urllib.parse.urlencode(query))
        batch = response.get('sources', {})
        rows, total = batch.get('items'), batch.get('total')
        if not isinstance(rows, list) or type(total) is not int or batch.get('offset') != offset:
            raise RuntimeError('invalid source pagination response')
        if categories is None:
            categories = response.get('categories')
        elif response.get('categories') != categories:
            raise RuntimeError('source categories changed during pagination; reconcile before selection')
        totals.append(total)
        if len(set(totals)) > 1:
            raise RuntimeError('source inventory changed during pagination; coverage remains incomplete')
        ids = [row.get('id') for row in rows if isinstance(row, dict)]
        if len(ids) != len(rows) or any(not isinstance(i, str) or not i for i in ids) or len(set(ids)) != len(ids) or seen.intersection(ids):
            raise RuntimeError('source page repeats or contains invalid IDs; coverage remains incomplete')
        seen.update(ids); items.extend(rows); offset += len(rows)
        if offset >= total:
            if offset != total:
                raise RuntimeError('source count exceeds reported total')
            return {'categories': categories, 'sources': {'offset': start, 'total': total, 'items': items},
                    'pagination': {'complete': start == 0, 'pages': page + 1, 'since': since, 'next_offset': offset,
                                   'scope': 'cache first-seen time; verify original publication date separately'}}
        if not rows:
            raise RuntimeError('source pagination ended before reported total')
    raise RuntimeError('source page budget exceeded; coverage remains incomplete')
