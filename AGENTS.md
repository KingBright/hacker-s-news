# FreshLoop Agent Runbook

This file is the first stop for future coding agents and contributors. Read it
before changing build, deploy, client UI, or product-line behavior.

## Canonical Commands

- Context recovery: `./scripts/context_snapshot.sh`
- Backend tests: `cd backend && cargo test -p cortex && cargo test -p nexus`
- Frontend checks: `cd frontend && npm run lint && npm run build`
- Android analysis: `cd android_client && flutter analyze`
- Brand contract: `./scripts/verify_brand_contract.sh`
- Android release contract tests: `./scripts/test_android_release_contract.sh`
- Feed API smoke: `NEXUS_KEY="$(python3 -c 'import tomllib; print(tomllib.load(open("config.toml","rb"))["nexus"]["auth_key"])')" ./scripts/verify_feed_api.sh`
- Android release: `./scripts/deploy.sh --android`
- Full server/frontend deploy: `./scripts/deploy.sh`

`./scripts/deploy.sh --android` is the only authorized Android release entrypoint.
It reads the public state from `https://news.hackerlife.fun:8443`, automatically
bumps a matching local/public version once, preserves an already-unpublished
newer local build, validates APK package/label/version/signing certificate and
SHA-256, uploads APK plus `version.json` together, then downloads both from the
public 8443 endpoint and verifies them before reporting success. Do not publish
with bare `flutter build apk`, manual file copies, `--frontend`, or
direct calls to `scripts/deploy_core.sh`.

`scripts/deploy.sh` is intentionally gitignored because it contains the target
server coordinates. It must be a thin wrapper that exports configuration and
executes the tracked `scripts/deploy_core.sh`; all release behavior belongs in
the tracked core. `scripts/deploy.sh.template` is the canonical wrapper shape.

Normal frontend/full deployments intentionally preserve the exact Android APK
and `version.json` already online. They do not publish an Android build even if
`frontend/public/android-app.apk` contains newer local work.

## Context Recovery

When returning to this project after context loss, start with:

```bash
./scripts/context_snapshot.sh
```

Then read the docs it lists before changing code. This is the durable memory
path for this repo: `AGENTS.md` for working rules, `task.md` for the active
implementation checklist, `docs/build-and-deploy.md` for deploy behavior, and
`docs/freshloop-product-style.md` for product/UI constraints. For scheduled
Codex/Antigravity content generation, also read
`docs/agent-content-workflow.md` and `docs/scheduled-agent-runbook.md`.

## Product Lines

FreshLoop now has two product lines:

- Radio: audio-first news briefings, queue playback, notification controls, and
  background audio.
- Reading: curated subscription articles, original/compressed reading modes,
  optional article audio, and weekly audio digests.

Keep shared content ingestion and publishing behavior in Cortex/Nexus shared
modules. Do not fork a second backend stack for Reading.

Scheduled agents may generate manuscripts and structured artifacts through the
Nexus internal agent API. Nexus remains the only writer for product tables and
voice job state; Cortex consumes `voice_jobs` and uploads generated audio.

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
- For brand, icon, metadata, launch, or navigation changes, run
  `./scripts/verify_brand_contract.sh`.
- Before changing Android release logic, run
  `./scripts/test_android_release_contract.sh`. For an actual Android release,
  use only `./scripts/deploy.sh --android`; success requires the final public
  8443 version/APK/signature/SHA verification.
