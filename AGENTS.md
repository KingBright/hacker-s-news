# FreshLoop Agent Runbook

This file is the first stop for future coding agents and contributors. Read it
before changing build, deploy, client UI, or product-line behavior.

## Canonical Commands

- Context recovery: `./scripts/context_snapshot.sh`
- Backend tests: `cd backend && cargo test -p cortex && cargo test -p nexus`
- Frontend checks: `cd frontend && npm run lint && npm run build`
- Android analysis: `cd android_client && flutter analyze`
- Feed API smoke: `NEXUS_KEY="$(python3 -c 'import tomllib; print(tomllib.load(open("config.toml","rb"))["nexus"]["auth_key"])')" ./scripts/verify_feed_api.sh`
- Android package and publish into the web bundle: `./scripts/deploy.sh --android`
- Full server/frontend deploy: `./scripts/deploy.sh`

Do not treat a bare `flutter build apk` failure as authoritative for this
project. The deploy script configures the Android packaging environment first:
it resolves Homebrew `openjdk@17`, exports `JAVA_HOME`, then runs Flutter and
copies `app-release.apk` to `frontend/public/android-app.apk`.

## Context Recovery

When returning to this project after context loss, start with:

```bash
./scripts/context_snapshot.sh
```

Then read the docs it lists before changing code. This is the durable memory
path for this repo: `AGENTS.md` for working rules, `task.md` for the active
implementation checklist, `docs/build-and-deploy.md` for deploy behavior, and
`docs/freshloop-product-style.md` for product/UI constraints.

## Product Lines

FreshLoop now has two product lines:

- Radio: audio-first news briefings, queue playback, notification controls, and
  background audio.
- Reading: curated subscription articles, original/compressed reading modes,
  optional article audio, and weekly audio digests.

Keep shared content ingestion and publishing behavior in Cortex/Nexus shared
modules. Do not fork a second backend stack for Reading.

## Visual System

FreshLoop's client identity is dark, quiet, audio-native, and green-accented:

- Background: near-black/dark green.
- Surfaces: dark cards with subtle borders and restrained shadows.
- Accent: `#19E66B`.
- Cards: compact rounded rectangles, close to existing feed/player cards.
- Icons: use familiar media/reading icons. Prefer icon buttons over text-only
  controls where the action is obvious.

Avoid introducing a bright document-app look for the Reading product line. It
should feel like FreshLoop learned to read, not like a separate web reader was
embedded.

## Risk Notes

- Android playback is sensitive: audio queue, notification, bookmark, and
  background playback are coupled through `audio_handler.dart`.
- Do not patch playback failures with broad `try/catch`. Identify the risk
  boundary: missing audio URL, invalid notification resource, player source
  interruption, queue mutation, bookmark state, or network failure.
- When adding Android UI, prefer Dart-side models/API wrappers unless changing
  Rust Bridge types is truly necessary.
- Preserve unrelated dirty worktree changes. This repo often has in-progress
  deployment and client edits.

## Verification Expectations

Before handing work back after substantial changes:

- Run the relevant backend/frontend/mobile checks above.
- Run `./scripts/verify_feed_api.sh` after Feed/Nexus/Cortex changes. Provide
  `NEXUS_KEY` when testing internal write interfaces.
- For frontend UI, do a browser smoke check on desktop and mobile widths.
- For Android APK, use `./scripts/deploy.sh --android` or explain why that
  canonical path could not run.
