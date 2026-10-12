"""Read-only delivery gate and deterministic human report. Never publishes or requeues."""
import json
import time
from datetime import datetime, timedelta
from pathlib import Path
from urllib.parse import urljoin, urlparse, quote
from urllib.request import Request, urlopen
from zoneinfo import ZoneInfo
from review_comparison import report_issues, run_root, unresolved, bounded_path, digest, read


def audio_probe(base, path, context):
    url = urljoin(base + '/', path)
    if urlparse(url).netloc != urlparse(base).netloc or urlparse(url).scheme != urlparse(base).scheme:
        raise ValueError('Audio URL is outside the configured public origin')
    # Public request, deliberately no internal authentication header.
    with urlopen(Request(url, headers={'Range': 'bytes=0-65535'}), timeout=30, context=context) as response:
        if urlparse(response.geturl()).netloc != urlparse(base).netloc:
            raise ValueError('Unexpected audio redirect')
        data = response.read(65536)
        if response.status not in (200, 206) or len(data) < 128:
            raise ValueError('Audio is unavailable or empty')
        if not (data.startswith(b'ID3') or any(data[i] == 255 and data[i+1] & 224 == 224 for i in range(min(len(data)-1, 4096)))):
            raise ValueError('Public URL did not return MP3 bytes')
    return url


def published_target(public, job):
    ident = job.get('target_id')
    kind = job.get('target_type')
    if kind == 'feed_item':
        return public.call('/api/feed/items/' + quote(ident, safe=''))
    endpoint = '/api/items' if kind in ('item', 'radio_item') else '/api/feed/weeklies' if kind == 'weekly_digest' else None
    if endpoint is None:
        raise ValueError('Unsupported voice target')
    seen = set()
    for page in range(1, 6):
        rows = public.call(f'{endpoint}?page={page}&limit=100')
        for row in rows:
            if row.get('id') == ident:
                return row
        ids = tuple(row.get('id') for row in rows)
        if not rows or ids in seen:
            break
        seen.add(ids)
    raise ValueError('Completed audio target is not visible in the public product')


def check_output(api, public, entry, probe=audio_probe):
    result = {'title': entry['title'], 'voice_job_id': entry['voice_job_id'], 'state': 'waiting'}
    job = api.call('/api/internal/agent/voice-jobs/' + quote(entry['voice_job_id'], safe=''))
    if job.get('status') in ('failed', 'cancelled'):
        result.update(state='failed', reason='语音生成失败，需要处理')
        return result
    if job.get('status') != 'completed':
        return result
    target = published_target(public, job)
    if not job.get('audio_url') or target.get('audio_url') != job['audio_url'] or (target.get('duration_sec') or 0) <= 0:
        raise ValueError('Voice completion and public content do not match')
    if target.get('is_deleted') or target.get('status') not in ('published', 'ready', 'completed', None):
        raise ValueError('Content is not published')
    url = probe(public.base, target['audio_url'], public.context)
    result.update(state='ready', audio_url=url, duration_sec=target['duration_sec'], title=target.get('title') or entry['title'])
    return result


def check_weekly(public, probe=audio_probe, now=None):
    now = now or datetime.now(ZoneInfo('Asia/Shanghai'))
    if now.weekday() == 0 and now.hour < 8:
        return {'state': 'not_due', 'description': '本周周报尚未到周一08:00执行时间'}
    monday = (now - timedelta(days=now.weekday())).replace(hour=0, minute=0, second=0, microsecond=0)
    start = int((monday - timedelta(days=7)).timestamp())
    for row in public.call('/api/feed/weeklies?limit=20'):
        if row.get('week_start') == start:
            if row.get('audio_url') and (row.get('duration_sec') or 0) > 0 and row.get('status') == 'published':
                # Also verify Reading's linked entry: the listener must see the same audio.
                linked = public.call('/api/feed/items/' + quote(row['feed_item_id'], safe=''))
                if linked.get('audio_url') != row['audio_url']:
                    raise ValueError('Weekly and Reading audio differ')
                return {'state': 'ready', 'description': row['title'], 'audio_url': probe(public.base, row['audio_url'], public.context)}
            return {'state': 'waiting', 'description': '本周周报音频尚未上线'}
    return {'state': 'missing', 'description': '最近完整自然周的周报缺失，需按原周槽位补做'}


def render_report(manifest, outputs, weekly, ready):
    clean = lambda s: str(s).replace('\n', ' ').replace('|', '／').strip()
    lines = [f"# FreshLoop {clean(manifest['date'])} {clean(manifest['edition'])}", '',
             ('**内容已上线，可收听。**' if outputs else '**本次无新增内容，周报已核对。**') if ready else '**本次交付未完成，请勿视为已上线。**', '',
             '| 内容 | 结果 | 时长 |', '| --- | --- | --- |']
    for row in outputs:
        state = f"[收听]({row['audio_url']})" if row['state'] == 'ready' else row.get('reason', '仍在制作，尚未交付')
        seconds = int(row.get('duration_sec') or 0)
        lines.append(f"| {clean(row['title'])} | {state} | {seconds//60}:{seconds%60:02d} |")
    lines.extend(['', '周报检查：' + (f"[已上线]({weekly['audio_url']})" if weekly['state'] == 'ready' else clean(weekly['description'])), '', '异常与未完成项：'])
    warnings = list(manifest.get('warnings', []))
    warnings.extend(f"编辑异常／{r['stage']}：{r['reason']}；处置：{r['disposition']}；稿件：{r.get('artifact') or '来源核验'}" for r in manifest.get('editorial_issues', []))
    lines.extend(['- ' + clean(w) for w in warnings] or ['- 无额外异常。'])
    if not ready:
        lines.append('- 未达到音频交付标准；保留现有任务并处理故障，不重新创建内容绕过状态。')
    return '\n'.join(lines) + '\n'


def daily_run(manifest, reference):
    return 'agent-runs' in Path(reference).resolve().parts and any(
        x in str(manifest.get('edition', '')).lower() for x in ('早间', '晚间', 'morning', 'evening'))


def radio_scope(manifest, reference):
    if not daily_run(manifest, reference):
        return []
    try:
        from editorial_selection import selection
        path = bounded_path(run_root(reference), manifest.get('radio_ledger', ''))
        bound = selection(None, path, reference)
        if bound['pending'] or any(c['outcome'] == 'pending' for c in bound['categories']):
            return ['Radio候选或类别仍有未完成核验，不得声称完整覆盖。']
        if bound['events']:
            state = read(bounded_path(run_root(reference), manifest.get('radio_state', 'radio-state.json')))
            if state.get('editorial_selection', {}).get('sha256') != bound['sha256']:
                raise ValueError('Radio delivery ledger differs from the original checkpoint plan')
        return []
    except (ValueError, OSError, KeyError, TypeError) as exc:
        return ['Radio范围未完成核对：' + str(exc)]


def reading_scope(manifest, reference):
    """Daily production must account for Reading; audio readiness alone cannot hide it."""
    if not daily_run(manifest, reference):
        return []
    try:
        path = bounded_path(run_root(reference), manifest.get('reading_ledger', ''))
        ledger = read(path)
        inventory = read(bounded_path(run_root(reference), ledger['inventory_file']))
        if digest(inventory) != ledger.get('inventory_sha256') or inventory.get('pagination', {}).get('complete') is not True:
            raise ValueError('Reading inventory is incomplete or changed')
        expected = {r['id'] for r in inventory.get('sources', {}).get('items', [])}
        rows = ledger.get('candidates', [])
        if not isinstance(rows, list) or len(rows) != len(expected) or {r.get('source_id') for r in rows} != expected:
            raise ValueError('Reading candidates were not reconciled')
        output_ids = {r['voice_job_id'] for r in manifest['outputs']}
        pending = []
        for row in rows:
            outcome = row.get('outcome')
            if not row.get('reason') or outcome not in ('published', 'already-published', 'not-selected', 'pending', 'unverifiable'):
                raise ValueError('Reading decision lacks a concrete reason')
            if outcome == 'published' and row.get('voice_job_id') not in output_ids:
                raise ValueError('Selected Reading audio is missing from required outputs')
            if outcome in ('pending', 'unverifiable'):
                pending.append(row['source_id'])
        return ['Reading仍有未完成候选：' + ','.join(pending)] if pending else []
    except (ValueError, OSError, KeyError, TypeError) as exc:
        return ['Reading范围未完成核对：' + str(exc)]


def finalize(args, api, public, save):
    manifest = json.loads(Path(args.manifest).read_text())
    if not manifest.get('date') or not manifest.get('edition') or not isinstance(manifest.get('outputs'), list):
        raise ValueError('Manifest requires date, edition and outputs')
    entries = manifest['outputs']
    if len(entries) > 20 or (not entries and not str(manifest.get('no_new_content_reason', '')).strip()):
        raise ValueError('Empty outputs require an explicit no_new_content_reason; at most20 outputs')
    ids = [entry['voice_job_id'] for entry in entries]
    if len(set(ids)) != len(ids) or any(not entry.get('title') for entry in entries):
        raise ValueError('Duplicate voice jobs or missing output titles')
    if not 0 <= args.wait_seconds <= 7200:
        raise ValueError('Wait must be between 0 and7200seconds')
    deadline = time.monotonic() + args.wait_seconds
    scope_errors = radio_scope(manifest, args.manifest) + reading_scope(manifest, args.manifest)
    manifest['warnings'] = list(manifest.get('warnings', [])) + scope_errors
    while True:
        outputs = []
        for entry in entries:
            try:
                outputs.append(check_output(api, public, entry))
            except Exception as exc:
                outputs.append({'title': entry['title'], 'voice_job_id': entry['voice_job_id'], 'state': 'unverified', 'reason': '上线验证失败，需检查服务或内容记录', 'diagnostic_type': type(exc).__name__, 'http_status': getattr(exc, 'status', getattr(exc, 'code', None))})
        try:
            weekly = check_weekly(public)
        except Exception:
            weekly = {'state': 'unverified', 'description': '周报上线验证失败'}
        manifest['editorial_issues'] = report_issues(run_root(args.manifest))
        editorial_blocked = bool(scope_errors) or unresolved(manifest['editorial_issues'])
        ready = not editorial_blocked and all(row['state'] == 'ready' for row in outputs) and weekly['state'] in ('ready', 'not_due')
        state = {'editorial_issues': manifest['editorial_issues'], 'editorial_blocked': editorial_blocked, 'scope_errors': scope_errors, 'delivered': ready, 'checked_at': datetime.now(ZoneInfo('Asia/Shanghai')).isoformat(), 'outputs': outputs, 'weekly': weekly}
        save(args.out, state)
        print(json.dumps({'delivered': ready, 'ready': sum(row['state'] == 'ready' for row in outputs), 'total': len(outputs), 'weekly': weekly['state']}, ensure_ascii=False), flush=True)
        if ready or editorial_blocked or time.monotonic() >= deadline or any(row['state'] == 'failed' for row in outputs) or weekly['state'] == 'missing':
            report = Path(args.report)
            report.parent.mkdir(parents=True, exist_ok=True)
            report.write_text(render_report(manifest, outputs, weekly, ready))
            report.chmod(0o600)
            return 0 if ready else 2
        time.sleep(min(30, max(0, deadline-time.monotonic())))
