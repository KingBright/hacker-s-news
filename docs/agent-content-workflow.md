# FreshLoop Scheduled Agent Content Workflow

FreshLoop supports external scheduled agents such as Codex or Antigravity for
content generation. Nexus remains the authority for job state, validation,
deduplication, product tables, user memory, and publishing. Cortex consumes only
voice jobs and writes generated audio back through Nexus.

For the exact step-by-step procedure that scheduled agents must follow at run
time, use `docs/scheduled-agent-runbook.md`. This file explains architecture;
the runbook is the executable operating contract.

## Architecture

```text
Scheduled Agent
  -> Nexus internal agent API
  -> agent_jobs / product tables / voice_jobs
  -> Cortex voice_worker
  -> Nexus upload + voice completion API
  -> Web / Android clients
```

External agents must not write SQLite or Redb files directly. The MCP-like
surface is the internal HTTP contract under `/api/internal/agent/*`; a future
MCP server should be a thin wrapper around these endpoints.

## Internal Auth

Every endpoint below requires:

```http
X-NEXUS-KEY: <NEXUS_KEY>
```

## Agent Job Lifecycle

1. Create or pre-seed a job:

```http
POST /api/internal/agent/jobs
```

```json
{
  "job_type": "reading_article",
  "priority": 0,
  "run_after": 1782403200,
  "context": {
    "product_line": "reading",
    "date": "2026-06-26"
  },
  "input": {
    "sources": []
  }
}
```

2. Scheduled agents lease work:

```http
POST /api/internal/agent/jobs/lease
```

```json
{
  "agent_id": "codex-daily-reading",
  "job_types": ["reading_article", "weekly_digest"],
  "lease_seconds": 1800
}
```

3. Agents fetch context if needed:

```http
GET /api/internal/agent/jobs/{id}/context
```

4. Agents submit one validated artifact:

```http
POST /api/internal/agent/jobs/{id}/submit
```

5. Agents report recoverable failure:

```http
POST /api/internal/agent/jobs/{id}/fail
```

Nexus records lifecycle events; expired leases can be reclaimed until `max_attempts`.
Create by ID and identical completed submission are idempotent. Leases may target
`job_id` exactly. Invalid drafts roll back and retain the live lease for correction.
Content rows, voice jobs and completion commit in one SQLite transaction.

## Supported Artifacts

### `radio_episode`

Publishes to the existing Radio `items` table and creates a `radio_episode`
voice job.

```json
{
  "title": "今日科技简报",
  "category": "热点科技",
  "script": "完整可听稿...",
  "publish_time": 1782403200,
  "original_url": "https://example.com/story",
  "tags": ["ai"],
  "sources": [
    {
      "url": "https://example.com/story",
      "title": "Original title",
      "summary": "Source summary"
    }
  ]
}
```

### `reading_article`

Publishes to `feed_items` and `feed_item_contents`. If `audio_script` is
present, Nexus creates a `curated_article` voice job. `original_url` is required
so Web and Android can preserve the original-content link.

```json
{
  "title": "文章标题",
  "subtitle": "可选副标题",
  "source_name": "Source",
  "source_url": "https://example.com",
  "original_url": "https://example.com/article",
  "canonical_url": "https://example.com/article",
  "publish_time": 1782403200,
  "reader_markdown": "原文阅读版 Markdown",
  "plain_text": "纯文本",
  "compressed_markdown": "干货压缩版 Markdown",
  "audio_script": "可听稿，不要直接塞超长全文",
  "key_points": ["要点一", "要点二"],
  "reading_time_min": 6,
  "quality_score": 8,
  "tags": ["engineering"]
}
```

### `weekly_digest`

Publishes a weekly digest row and a corresponding listen-first feed item. If
`audio_script` is present, Nexus creates a `curated_weekly` voice job.

```json
{
  "title": "本周阅读汇总",
  "week_start": 1782144000,
  "week_end": 1782748799,
  "digest_markdown": "周报正文",
  "audio_script": "周报可听稿",
  "included_item_ids": ["feed-item-id"],
  "themes": ["AI", "Engineering"]
}
```

### `loop_preference_extraction`

Writes preference signals into the loop memory store and updates the source
Loop post status. These signals should express weighted tendencies, not hard
filters. User preferences change the emphasis mix; they must not permanently
blacklist content categories.

```json
{
  "post_id": "loop-post-id",
  "user_id": "default",
  "status": "processed",
  "signals": [
    {
      "content": "用户最近更关注 AI Agent 产品化案例",
      "signal_type": "topic_interest",
      "polarity": "positive",
      "confidence": 0.82,
      "strength": 1.3,
      "evidence": "用户在表达区提到..."
    }
  ]
}
```

### `holiday_calendar_update`

Accepted as a stored artifact for the holiday sync path. The existing Cortex
holiday agent still owns official-notice extraction and local calendar file
updates.

## Voice Job Lifecycle

Cortex `voice_worker` polls:

```http
POST /api/internal/agent/voice-jobs/lease
```

It renews its lease while synthesis is running:

```http
POST /api/internal/agent/voice-jobs/{id}/heartbeat
```

It synthesizes `text`, uploads the MP3 through `/api/internal/upload`, then
completes:

```http
POST /api/internal/agent/voice-jobs/{id}/complete
```

Nexus updates the correct target:

- `radio_item`: `items.audio_url`, `items.duration_sec`
- `feed_item`: `feed_items.has_audio`, `feed_items.audio_url`, `feed_items.duration_sec`
- `weekly_digest`: `weekly_digests.audio_url` plus the linked feed item audio fields

If synthesis or upload fails, Cortex calls:

```http
POST /api/internal/agent/voice-jobs/{id}/fail
```

The task is requeued until `max_attempts`.

For diagnostics, query a single voice job directly:

```http
GET /api/internal/agent/voice-jobs/{id}
```

## Cortex Configuration

App-internal production lets Cortex own RSS fetch, local LLM drafting, Reading
generation, weekly digest checks, and Loop preference extraction. This is the
default local production mode when scheduled agents are not stable enough to own
content quality:

```toml
[content_generation]
enabled = true

[llm]
model = "google/gemma-4-26b-a4b-qat"
api_url = "http://127.0.0.1:1234/v1"
json_mode = "none"
```

External production keeps Cortex source ingestion/cache and TTS active while
disabling its local-LLM editorial pipeline:

```toml
[content_generation]
enabled = true
mode = "external_agent"
```

In external mode, Cortex does not register scheduled local-LLM news, curated feed,
weekly digest, Loop preference extraction, or regeneration jobs. Manual trigger
endpoints such as `/api/trigger` and `/api/trigger/feed` return HTTP `409`.
Health, status, memory diagnostics, source ingestion/cache APIs, and `voice_worker` remain available.
The missing/default mode is `self_driven`. Legacy `enabled=false` without the
explicit external mode remains voice-only; it does not ingest sources.

`voice_worker` is optional and defaults to enabled. Add this section to make the
runtime explicit:

```toml
[voice_worker]
enabled = true
poll_interval_secs = 30
concurrency = 2
max_jobs_per_tick = 1
lease_seconds = 1800
repair_missing_audio = true
repair_interval_secs = 600
repair_limit = 20
# voice_kinds = ["radio_episode", "curated_article", "curated_weekly"]
```

`concurrency` starts multiple independent Nexus lease loops with distinct worker
ids. Keep `max_jobs_per_tick = 1` so each loop handles one audio job at a time.
`repair_missing_audio` lets Cortex ask Nexus to reconcile published Reading
articles and weekly digests that have an `audio_script` but no completed audio.
Nexus creates missing `voice_jobs` or backfills audio fields from completed jobs;
this does not run the old RSS/LLM content generation pipeline.

TTS process parallelism is controlled separately:

```toml
[tts]
process_isolation = true
worker_max_processes = 2
```

`voice_worker.concurrency` above `1` only improves throughput when
`tts.worker_max_processes` is also above `1`. Start with `2` on the 64 GB local
Mac because the current VoxCPM worker memory limit is `24576` MB.

## Scheduled Agent Design

Recommended cron jobs:

- `radio_episode`: twice per day at `07:30` and `18:00` Asia/Shanghai.
- `reading_article`: twice per day at `07:30` and `18:00` Asia/Shanghai.
- `weekly_digest`: once per day with the agent deciding whether enough material
  exists; schedule the check at `18:00` Asia/Shanghai.
- `loop_preference_extraction`: lightweight periodic scan of pending Loop
  expression posts at `07:30` and `18:00` Asia/Shanghai.
- `holiday_calendar_update`: low-frequency official-notice monitor.

Agents should submit complete artifacts, not partial drafts. If an agent needs
tooling, it can call public feed/memory APIs for context, but final writes must
go through the internal agent submit endpoint.

## Product Consistency Rules

- Radio and Reading both become playable through the same voice job mechanism.
- Reading keeps both original and compressed modes; generation should improve
  emphasis, structure, and readability, not discard source value.
- Original links are required for Reading articles and should be displayed by
  Web and Android clients.
- Personalization is dynamic balance. It changes ordering, framing, examples,
  and emphasis, but does not turn preferences into absolute topic removal.
