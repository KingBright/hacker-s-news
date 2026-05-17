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
