# FreshLoop external agent runbook (v2)

The canonical, portable editorial workflow is
[`skills/freshloop-content-agent/SKILL.md`](../skills/freshloop-content-agent/SKILL.md).
Its helper manages transport and private checkpoints; Codex/Antigravity itself
performs source understanding, semantic deduplication, multimodal inspection and
manuscript writing. Do not run the legacy Python local-LLM content generators.

## Runtime responsibilities

- Cortex: existing RSS/OPML subscriptions, source-group preservation, proxy-aware
  bounded retrieval, persistent source snapshots, media link discovery and TTS.
- Mature agent: actual article extraction, Chinese adaptation, source attribution,
  selection, merge/dedup decisions, honest media observations and final review.
- Nexus: authenticated leases, atomic product/voice publication, duplicate
  constraints and audit artifacts. Clients retain Radio, Reading, Loop and Focus.

## Mode selection

```toml
[content_generation]
enabled = true
mode = "external_agent"

[voice_worker]
enabled = true
```

Default/missing mode is `self_driven`; `enabled=false` with no explicit mode
retains legacy voice-only behavior. External mode overrides local generation,
continues source ingestion at `interval_min` (default 30, minimum 5), and leaves
voice processing available. Manual generation triggers return 409; source
refresh does not call the LLM. Selecting a mode requires restarting Cortex.
Do not enable two editorial schedulers with different logical slot conventions.

## APIs and checkpoints

Cortex (existing X-CORTEX-KEY rules):
- `GET /api/agent/sources?product_line=reading&offset=0&limit=30`
- `POST /api/agent/sources/refresh`
- `GET /api/agent/sources/{id}`
- `POST /api/agent/sources/{id}/fetch`
- `GET /api/agent/sources/{id}/media/{index}` (cached binary, 8 MiB maximum)

Sources are retained for 14 days; pages download on demand and reuse successful
snapshots. Inspect timestamps and errors. Listing is compact and paginated; full
HTML/media live in individual records. Cache data is not already curated text.

Nexus needs schema version 2. Create is idempotent for a stable job ID; lease
accepts exact `job_id`; expired ownership cannot heartbeat, fail or publish.
Content submission commits product rows, voice jobs and job completion together.
Identical completed submit with the same token returns its existing result;
changed content conflicts. Invalid drafts roll back and retain the lease.
Loop preferences live in Redb separately: validate before writes and use stable
post/signal keys to prevent duplicate memories on retry; no distributed
transaction across SQLite and Redb is claimed.

Keep a separate mode-0600 helper state file per logical job. Never echo lease or
auth tokens. On unknown submit outcomes preserve the exact draft and retry it,
not a newly generated draft. Content gate rejects raw English/markup fallbacks
for jobs whose context declares `production_mode=external_agent`.

## Rollout and verification

1. Run backend checks and helper regression tests; deploy the v2 Nexus backend.
2. Install the repo skill into each agent runtime's skill directory and supply
   authenticated Cortex/Nexus environment variables. Use trusted CA configuration;
   never disable TLS verification.
3. Run a small controlled batch; verify readable content, source/media links,
   returned voice IDs, completed audio and client playback separately.
4. Switch Cortex to external mode and replace the legacy editorial schedule with
   skill invocations. Retain the existing self-driven config for rollback.
5. Use the same run-date/slot convention in both runtimes. Schedule failures must
   preserve checkpoints. A queued voice job is not proof of audible completion.

Read the skill for bounded batch sizes, quality rules and scheduled prompt. This
file supersedes the old curl skeleton that ignored create errors and leased an
unrelated job type from the first 20 queue entries.

## TTS capacity planning

See [measured concurrency and pipeline guidance](tts-throughput-2026-09-21.md).
Match voice worker concurrency with isolated TTS process slots and benchmark on
the target machine before increasing them. Multiple Metal processes share the
same GPU; a higher process count is not an assumed linear speedup.

## Installed production schedule (2026-09-21)

Antigravity native Scheduled Tasks, project `hacker-s-news`, Asia/Shanghai:

| Task ID | Schedule | Work |
| --- | --- | --- |
| `freshloop-loop` | Daily around 07:30 | Radio categories, up to 3 Reading articles, pending Loop posts |
| `freshloop-evening-loop` | Daily around 18:00 | New/deduplicated Radio and Reading, pending Loop posts |
| `freshloop-weekly` | Monday around 08:00 | Previous complete Monday–Sunday week |

Antigravity applies launch jitter; these are target times, not exact deadlines.
Its native scheduled tasks use Flash. Background/menu-bar mode and Prevent Sleep
were already enabled and were verified. The application, network and model
account must remain available. Exact installed prompts are recorded in
`antigravity-schedules.json`. Daily prompts include a missing-weekly catch-up
check with the same weekly slot; they never create a second logical weekly job.

Use `python3 scripts/freshloop_agent_runtime.py` for helper commands. For other
read-only API calls, use its `api-get --service nexus|cortex --path /api/...
--out PRIVATE_FILE` command. Credentials are loaded privately; do not print job
lease tokens. This launcher and Cortex honor optional `[nexus].connect_ip` for
a verified local route, preserving the HTTPS hostname/SNI and certificate check.
This is scoped to the process, not a system DNS change. The installed desktop
uses the verified NAS LAN route to avoid intermittent public hairpin connection
failures. A machine on another network must use a reachable verified route or
omit `connect_ip`; never disable TLS verification to work around routing.

For this rollout `voice_worker.repair_missing_audio=false`: new external
submissions atomically create voice jobs, which retain their normal retries.
Unreviewed historical fallback scripts must not be repeatedly auto-enqueued by
the legacy backfill scan. Existing queued jobs and article records are retained;
this does not delete history. Enable blanket backfill only after the historical
manuscripts have passed editorial review. The latest bad weekly was repaired and
its audio accepted separately.

Weekly manuscript repair is explicit: a leased `weekly_digest` artifact may
include `replace_digest_id` and the exact `expected_audio_script` that was
reviewed. Nexus accepts only the same published, still-silent digest/window,
preserves both product IDs, cancels/fences old voice leases and creates the new
voice job in the submit transaction. A changed script, wrong window or already
voiced target is rejected. This is an authorized repair path, not a way to bypass
normal duplicate checks or silently overwrite historical audio.

## Complete Radio programs (2026-09-22)

Daily Radio now uses one `radio_program` per date and `morning-program` /
`evening-program` slot. Draft category sections locally under the coverage
contract; use one opening/closing and submit the ordered sections together.
Cortex applies configured host/category ownership and assembles a single MP3.
Do not submit each section separately. Weekly and Reading remain unchanged.
