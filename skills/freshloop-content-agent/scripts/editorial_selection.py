"""Reconcile the existing candidate ledger; no ranking/model/publication calls."""
from pathlib import Path
from review_comparison import EditorialError, bounded_path, digest, read, run_root, save

OUTCOMES = {'included-major', 'included-brief', 'merged-duplicate',
            'already-covered-without-update', 'out-of-scope', 'unverifiable', 'pending'}


def selection(artifact, ledger_path, reference):
    root = run_root(reference)
    path = bounded_path(root, ledger_path)
    ledger = read(path)
    inventory_path = bounded_path(root, ledger.get('inventory_file', ''))
    inventory = read(inventory_path)
    if digest(inventory) != ledger.get('inventory_sha256') or inventory.get('pagination', {}).get('complete') is not True:
        raise EditorialError('selection_inventory_incomplete', 'Candidate ledger must bind the complete metadata inventory', 'selection')
    items = inventory.get('sources', {}).get('items', [])
    expected = {x['id']: x for x in items}
    rows = ledger.get('candidates', [])
    if not isinstance(rows, list) or len(rows) != len(expected) or {x.get('source_id') for x in rows} != set(expected):
        raise EditorialError('selection_candidate_missing', 'Account for every fetched candidate; do not replace the inventory with selected stories', 'selection')
    categories = {x['name'] for x in inventory.get('categories', [])}
    category_rows = ledger.get('categories', [])
    if {x.get('name') for x in category_rows} != categories or any(not x.get('reason') or x.get('outcome') not in ('covered', 'empty', 'pending', 'filtered') for x in category_rows):
        raise EditorialError('selection_category_missing', 'Reconcile every configured category with a concrete outcome, including pending coverage', 'selection')
    events = {}
    for row in rows:
        if row.get('outcome') not in OUTCOMES or not row.get('reason') or row.get('category') not in categories:
            raise EditorialError('selection_reason_missing', 'Every candidate needs its category and concrete inclusion/exclusion reason', 'selection')
        if row['outcome'].startswith('included-'):
            key, tier = row.get('event_key'), row['outcome'].split('-')[1]
            if not key or not row.get('importance_reason'):
                raise EditorialError('selection_importance_missing', 'Choose depth from source-backed importance before drafting, not to satisfy a validator', 'selection')
            detail = row.get('required_details')
            if not isinstance(detail, dict) or not detail.get('actor') or not detail.get('event') or not detail.get('limits'):
                raise EditorialError('selection_detail_missing', 'Record actor, event and essential limitations (or why none apply)', 'selection')
            event = events.setdefault(key, {'tier': tier, 'category': row['category'], 'source_ids': [], 'required_details': detail})
            if event['tier'] != tier or event['category'] != row['category']:
                raise EditorialError('selection_event_conflict', 'Merged event has conflicting depth/category decisions', 'selection')
            event['source_ids'].append(row['source_id'])
    for row in rows:
        if row['outcome'] == 'merged-duplicate':
            if row.get('event_key') not in events:
                raise EditorialError('selection_merge_missing', 'Merged report must point to an included independent event', 'selection')
            events[row['event_key']]['source_ids'].append(row['source_id'])
    result = {'file': str(path), 'sha256': digest(ledger), 'inventory_file': str(inventory_path),
              'inventory_sha256': digest(inventory), 'events': events,
              'pending': [r['source_id'] for r in rows if r['outcome'] in ('pending', 'unverifiable')],
              'categories': category_rows}
    if artifact is None:
        return result
    stories = {}
    for section in artifact.get('sections', [artifact]):
        for story in section.get('stories', []):
            key = story.get('event_key')
            if key in stories:
                raise EditorialError('selection_event_duplicate', 'An event appears more than once', 'selection')
            stories[key] = (story, section.get('category'))
    if set(stories) != set(events):
        raise EditorialError('selection_event_missing', 'Final stories must cover every selected independent event', 'selection')
    for key, event in events.items():
        story, category = stories[key]
        if story.get('tier') != event['tier'] or category != event['category']:
            raise EditorialError('selection_depth_changed', 'Do not downgrade major stories to brief after validation; restore the planned depth and facts', 'selection')
        if set(story.get('source_urls', [])) != {expected[i]['url'] for i in event['source_ids']}:
            raise EditorialError('selection_source_changed', 'Story sources differ from the reconciled candidate event', 'selection')
    return result


def bind_selection(artifact, state, reference, ledger_path=None, state_path=None):
    is_radio = 'sections' in artifact or 'stories' in artifact
    if not is_radio:
        return None
    old = state.get('editorial_selection')
    path = ledger_path or (old or {}).get('file')
    if not path:
        if 'agent-runs' in Path(reference).resolve().parts:
            raise EditorialError('selection_ledger_missing', 'Production Radio requires --ledger with the reconciled original candidate inventory', 'selection')
        return None
    result = selection(None, path, reference)
    if old and result['sha256'] != old['sha256']:
        raise EditorialError('selection_plan_changed', 'The initial candidate/depth plan changed; keep the original plan and report unresolved evidence', 'selection')
    archive = run_root(reference) / '.editorial' / 'selections' / (result['sha256'] + '.json')
    if not archive.exists():
        save(archive, read(result['file']))
    state['editorial_selection'] = result
    if state_path: save(state_path, state)
    selection(artifact, path, reference)
    return result
