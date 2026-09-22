# Nexus v2 artifacts

Submit JSON with the helper. The helper supplies the lease token; never place secrets in artifacts. Nexus records the full artifact including editorial evidence. All dates below are Unix seconds when a numeric timestamp is required.

## Shared editorial object (required for external content jobs)

```json
{"editorial":{"language":"zh-CN","ready":true,"dedup_checked":true,"claims":[{"claim":"稿件中的具体事实","source_url":"https://source.example/article","evidence":"已核实的短原文引文或时间戳"}],"media":[{"url":"https://source.example/chart.png","kind":"image","status":"inspected","observation":"实际读图结论，包含坐标与单位"}],"quality":{"fidelity":2,"selection":2,"listening":2,"structure":2,"media":2}}}
```

`media` may be empty when there is no meaningful media. Values above illustrate the shape, not facts to copy. Ready/dedup flags represent completed checks. Never submit example URLs or placeholder evidence.

## Radio complete program (current daily publishing format)

Submit `job_type=radio_program` once per date/edition with slot `morning-program`
or `evening-program`. Required: `title`, `opening` (<=400 characters), `closing`
(<=200), `sections` (ordered nonempty array, distinct configured categories).
Each section is a complete Radio draft under the contract below, including
sources, stories, pronunciation variants, quality/evidence and coverage review.
Section drafts remain local: do not submit them separately as radio_episode.

Design transitions as one newsroom broadcast: the current host briefly closes or hands off to the next host; the next host picks up the topic and enters the news. Within one host’s adjacent categories, use a short natural topic transition. Write content-specific bridges without implying unsupported causality; avoid repeating “接下来”、greetings, ritual thanks or a forced interview. Include handoff lines in the appropriate host’s section, so the outgoing host actually speaks the handoff.

Opening announces date/edition and introduces the program once. Closing signs
off once. Sections introduce their topic and transition naturally; no repeated
welcome, date announcement or final farewell. Cortex maps categories to the
existing configured hosts (the first host anchors opening/closing); never pass
voice file paths in agent artifacts. Preserve category ownership and group
related sections coherently. One program creates one item and one multi-host
voice job. Total displayed script must fit the existing 80KB text contract;
per-section spoken manuscripts retain their own 6500-character bound. Do not
silently omit coverage to fit; report unresolved overflow.

The displayed manuscript preserves section headings and all source links;
Cortex caches completed segments across retries, assembles PCM in order and
encodes one MP3. A queued program is not playable until the whole audio succeeds.

## Radio

Required: `title`, `category` (one configured exact category name), `script` (10–6500 characters of substantive Chinese speech), `sources` (array of `{url,title,summary}`), `stories` (all qualifying independent events after semantic deduplication, at least one). Optional: `original_url`, `publish_time`, `tags`.

Each story requires `{event_key,title,tier,script,source_urls}`. `event_key` identifies the real event (entities/action/date), not a random counter. `tier` is `major` (developed news, at least 60 non-whitespace characters) or `brief` (one factual sentence, at least 10 non-whitespace characters). Each story script must satisfy its tier minimum and appear verbatim (whitespace ignored) in the complete `script`. When pronunciation edits change a story, also provide its `audio_script`, and include that exact spoken block in the top-level `audio_script`. Compose each full manuscript from a short intro + the story blocks + optional short closing; never supply multi-story metadata with a single-story audio manuscript. Each story cites declared sources and corresponding `editorial.claims` evidence. Every event needs source evidence, including brief news. Mechanical validation cannot establish that event keys describe genuinely different events: you must compare the underlying evidence.

Keep one section per category inside the morning/evening program; the legacy radio_episode endpoint remains for old integrations. Cover all qualifying news: major stories first, followed by a clearly introduced brief-news section. No target count or preferred total length; do not skip a category just because only 1–2 events qualify. Titles name the main topics without listing every brief. No stale filler.

Required `editorial.radio_coverage`: `{"reviewed":true,"eligible_event_keys":["actual-event-key", "another-event-key"]}`. Populate this from the complete candidate ledger described in editorial.md, not by copying an arbitrarily selected story list. Keys must be unique and match `stories` exactly; ledger exclusions/pending items and reasons remain in the private run report. This is an auditable declaration, not automated proof of semantic completeness. The 6500-character transport limit still applies: compress depth first, explicitly report unresolved overflow without silently omitting it or inventing extra slot IDs.

Each source must actually support a story. Use one lead original URL for the digest; do not reuse that URL for a different episode or invent query parameters to bypass duplicate protection. Radio persists to existing items and queues `radio_episode` audio.

Radio additionally accepts optional `audio_script` for pronunciation-safe spoken wording; `script` remains the displayed manuscript. Preserve factual equivalence. See [pronunciation preflight](pronunciation.md).

## Reading

Required by this skill: `title`, `original_url`, `reader_markdown`, `compressed_markdown`, `audio_script`, shared `editorial`. Optional: `subtitle`, `source_name`, `source_url`, `canonical_url`, `plain_text`, `publish_time`, `key_points` (specific insights), `reading_time_min`, `quality_score` (0–10), `tags`.

Keep the canonical original URL stable. Preserve source structure and media in the reader version. `audio_script` is a Chinese spoken adaptation, not raw Markdown or a copy of an English original. Retain configured subscription group in tags when appropriate. Nexus publishes to the existing curated feed and queues `curated_article` audio.

## Weekly

Required: `title`, `week_start`, `week_end`, `digest_markdown`, `audio_script`, `included_item_ids` (at least three distinct existing published articles in that time window), shared `editorial`. Optional: `themes`.

Use concrete exact dates and stable window slots; inspect existing weeklies before publishing. Nexus creates the weekly record and linked feed item together and queues `curated_weekly` audio.

## Loop

Required: `post_id`, `user_id`, `status` (`processed` or `skipped`), `signals` (may be empty for skipped). Each signal: `content`, `signal_type`, `polarity` (`positive` or `negative`), `confidence` (0–1), `strength` (0.1–5), `evidence` from the actual post. No content `editorial` object or TTS.

Do not guess post ownership or write a permanent category ban. A statement without durable preference should produce skipped, not an invented signal.
