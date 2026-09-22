# Task: FreshLoop Dual Product Foundation

## Goal

Refactor FreshLoop into a shared content foundation with two product lines:

- Radio: existing audio-first news briefing workflow.
- Curated Feed: daily high-quality subscription feed, reading-first with optional dry-summary audio, plus weekly digest manuscripts with optional audio.

## Phase 1: Foundation

- [x] Define the product split and implementation boundary.
- [x] Add reusable Cortex content primitives for feed sources, fetched entries, text normalization, and RSS fetching.
- [x] Keep the existing Radio workflow behavior-compatible while moving shared fetch/normalize logic out of `news.rs`.
- [x] Add Nexus feed data tables for mixed feed items, article content, and weekly digests.
- [x] Add public and internal Nexus Feed APIs.
- [x] Add Cortex DTOs/client methods for publishing feed items and weekly digests.

## Phase 2: Curated Feed Pipeline

- [x] Add configured curated feed sources, including OPML/RSS support.
- [x] Configure AK's own RSS plus the HN Popular Blogs OPML source list.
- [x] Add proxy-first fetching with domain-level proxy success cache for overseas feeds.
- [x] Skip old Radio subscriptions when the same feed is owned by the Curated Feed product line.
- [x] Implement daily article ingestion and quality scoring.
- [x] Implement original/condensed article content generation.
- [x] Implement optional article audio generation from a dedicated listening script.
- [x] Add recovery backfill for published curated articles that have text but are missing audio.

## Phase 3: Weekly Digest

- [x] Generate weekly summaries from curated feed items.
- [x] Publish weekly digest with a readable manuscript and optional audio.
- [x] Use a rolling local 7-day digest window so Monday/manual triggers do not create empty summaries.
- [x] Add retry/cache boundaries for LLM and TTS work.

## Phase 4: Clients

- [x] Add Web dual product navigation.
- [x] Add reading-focused Feed UI.
- [x] Add Android dual product navigation and reader surface.

## Verification

- [x] `cargo test -p cortex` (15 tests)
- [x] `cargo test -p nexus` (4 tests)
- [x] `npm run lint`
- [x] `npm run build`
- [x] HTTP smoke check for deployed `/feed` page shell and feed APIs
- [x] `flutter analyze`
- [x] Android APK packaging via `./scripts/deploy.sh --android`
- [x] Feed API smoke via `./scripts/verify_feed_api.sh`
- [x] Online version/APK check: `1.3.18+27`

## Memory/Recovery

- [x] Add `AGENTS.md` as the first-stop runbook for future resumed sessions.
- [x] Add build/deploy and product-style docs.
- [x] Add `./scripts/context_snapshot.sh` to reconstruct repo context without chat history.
- [x] Record long-term lessons under `~/.happy_coding/knowledge/`.

## External Agent Mode — 2026-09-21

- [x] Add explicit `external_agent` mode while preserving the self-driven default.
- [x] Keep RSS/OPML source groups, add persistent page/media cache and authenticated source APIs without local LLM drafting.
- [x] Fix exact leasing, expired ownership, atomic content/voice submission and identical-submit replay.
- [x] Create and validate `skills/freshloop-content-agent`; install shared repo skill into local Codex and Antigravity discovery directories.
- [x] Define Chinese editorial, semantic deduplication, provenance and honest multimodal inspection rules.
- [x] Preserve Radio/Reading/Loop/Focus; improve web original-audio display and fix Android reader links and empty quote markers.
- [x] Audit current manuscripts: `docs/content-quality-review-2026-09-21.md`.
- [x] Pass backend/helper/frontend/Android checks, brand contract and desktop/mobile browser smoke.
- [x] Isolated real API acceptance: source/page/media cache → reviewed manuscript → idempotent Nexus publish → Cortex VoxCPM audio → playable Reading metadata. 496-character sample produced 87.312s MP3 (published duration rounded to 88s), with local LLM endpoint deliberately unreachable.
- [x] Production rollout completed with user authorization: Nexus/Cortex v2 deployed, external mode active, three native Antigravity schedules enabled; 25 real audio outputs completed including weekly. See `docs/external-agent-acceptance-2026-09-21.md`.

Canonical runtime instructions: `docs/scheduled-agent-runbook.md` and the skill.

## TTS throughput review — 2026-09-21

- [x] Atomic voice leasing, concurrent lease regression, PID-only memory sampling and cancellation cleanup.
- [x] Offline 1/2/3-worker benchmark: retain two instances; third was slower. See `docs/tts-throughput-2026-09-21.md`.
- [x] Cortex83/Nexus36 plus auxiliary tests pass; live Feed smoke passes (writes skipped).
- [x] Reusable `scripts/benchmark_tts_concurrency.py`; production config/services untouched.

## Production acceptance — 2026-09-21

- [x] Morning/evening/weekly native triggers verified and formal schedules restored.
- [x] Weekly repair reviewed, unique week and linked Feed/audio consistency verified.
- [x] Two TTS workers, retry backoff, authenticated LAN TLS route and final installation verified.
- [x] Existing client playback/media/mobile smoke passed; no frontend or Android release in this rollout.

## Client publication — 2026-09-21

- [x] User-authorized Web + Android release through `./scripts/deploy.sh --android`: Android 1.3.32+41.
- [x] Public APK/version/signature/SHA verification, Web byte comparison, desktop/mobile playback and four-route smoke passed. See `docs/client-release-2026-09-21.md`.

## Radio editions and pronunciation — 2026-09-21

- [x] Web / Android: complete morning/evening cards with date/weekday, audio count/duration, collapsed details, whole-group play, live new-audio hints, no date/refresh hero.
- [x] Logical production-slot grouping, complete-edition pagination, quiet refresh without interrupting playback; stable totals after listening.
- [x] Spoken-only normalization and separate Radio audio_script; context-aware pronunciation skill, ambiguous API/AI/LLM preserved for editorial review.
- [x] Nexus and Cortex deployed; Web / Android 1.3.33+42 released and public signature/SHA verified.
- [x] Backend, client/helper tests, desktop/mobile/live audio checks passed; final 7-chunk ASR average99.46%, technical chunk100%, no late degradation. See `docs/radio-editions-2026-09-21.md`.

## Radio digest regression — 2026-09-21

- [x] Confirmed latest22 episodes were single-source/single-event; external migration omitted old minimum3-topic contract.
- [x] Restored category digest rules in skill, helper, Nexus and native morning/evening prompts; weekly unchanged and verified.
- [x] Helper9/Nexus42 tests, skill validation, canonical backend deployment, live capability and Feed checks passed.
- [ ] Next real scheduled digest needs semantic/listening acceptance; historical22 episodes preserved. See `docs/radio-digest-regression-2026-09-21.md`.


## Radio comprehensive coverage clarification — 2026-09-21

- [x] Replaced fixed minimum3/3–6 selection with all eligible events, major stories and one-sentence briefs; genuine sparse days publish.
- [x] Added candidate ledger/reviewed eligible-key reconciliation, tier-aware display/spoken validation and missing-event regression coverage.
- [x] Helper10/Nexus43 tests passed; native morning/evening prompts saved and reopened exactly.
- [ ] Next scheduled run: verify candidate coverage and listening quality, including briefs and explicit unresolved items.
- [x] Comprehensive-coverage backend deployed; live tier/coverage capabilities and Feed smoke verified (one transient SSH retry).

## Complete multi-host Radio programs and review — 2026-09-22

- [x] One radio_program per edition: unified opening/closing, source-backed category drafts, original host ownership and newsroom handoffs.
- [x] Parallel cached section synthesis, one MP3 encode; complete programs replace old card tracks when ready.
- [x] Reviewed production code and fixed streaming-upload corruption, async bcrypt blocking, repeated historical polling, quadratic grouping and deployment interruption handling.
- [x] Web/Nexus/Android1.3.35+44 published and public APK verified; native daily schedules updated, weekly unchanged.
- [x] Final Cortex pronunciation release; all three recent remasters complete (16:13 / 20:23 / 14:06), with public MP3 integrity/range checks, browser playback and handoff ASR acceptance.
- [x] Repair startup URL deduplication without deleting published history; restore three original items and preserve older audio.
- [x] Commit and push related FreshLoop work after final acceptance; preserve unrelated experiments.
