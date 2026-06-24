# FreshLoop Build And Deploy

This document records the commands that are considered authoritative for this
repository. If a command here disagrees with memory, trust the command here and
inspect the script before changing behavior.

## Android

Use:

```bash
./scripts/deploy.sh --android
```

What it does:

- Locates Homebrew `openjdk@17`.
- Exports `JAVA_HOME` and prepends `$JAVA_HOME/bin` to `PATH`.
- Runs `flutter build apk --release --target-platform android-arm64` from
  `android_client/`.
- Copies `android_client/build/app/outputs/flutter-apk/app-release.apk` to
  `frontend/public/android-app.apk`.
- Regenerates `frontend/public/version.json` from
  `android_client/pubspec.yaml`.
- Builds and uploads the frontend so the new APK and `version.json` are
  published together.

In-app update detection only triggers when the remote `build_number` in
`version.json` is higher than the installed Android app's build number. If you
expect clients to receive an update notification, make sure the Android version
was bumped before deployment.

Current script note: after the Android block, `deploy.sh` still performs the
remote directory preparation step. That is harmless for a normal deploy path,
but if a future change needs a purely local APK build, split that behavior into
a dedicated local packaging script instead of falling back to bare Flutter.

Optional version bump:

```bash
./scripts/deploy.sh --android --bump patch
```

Do not use bare `flutter build apk` as the final answer for this project unless
you intentionally want to test the ambient shell environment. A bare command can
fail if `JAVA_HOME` is not globally configured even though the repository's
packaging script works.

## Frontend

```bash
cd frontend
npm run lint
npm run build
```

The static export is written to `frontend/out`.

## Backend

```bash
cd backend
cargo test -p cortex
cargo test -p nexus
```

Remote Nexus deployment is handled by:

```bash
./scripts/deploy.sh
```

The deploy script builds the frontend, cross-compiles Nexus for
`x86_64-unknown-linux-musl`, uploads artifacts, writes `config.env`, and restarts
the remote systemd service.

## Cortex Local Service

```bash
./scripts/install_local_service.sh
```

Manual triggers:

```bash
curl -X POST http://localhost:3721/api/trigger
curl -X POST http://localhost:3721/api/trigger/feed
curl -X POST 'http://localhost:3721/api/trigger/feed/weekly?force=true'
curl http://localhost:3721/api/status
```

If `CORTEX_API_KEY` is configured, include it as `X-CORTEX-KEY` or bearer auth.

TTS resource policy for the local Cortex service:

- The 2026-05 memory incident was caused by the old Cortex TTS policy using
  Metal for VoxCPM inside the long-running LaunchAgent. Candle 0.10.2's Metal
  backend pools intermediate `private_buffers`; in VoxCPM's dynamic-shape
  generation path that pool can grow without being returned to the OS during
  process lifetime.
- Keep `[tts].keep_engine_loaded = false` unless the machine is dedicated to
  batch audio generation. Cortex is a long-running LaunchAgent, so loaded TTS
  models must be released after synthesis.
- Keep `[tts].memory_pressure_relief = true` on macOS so the process asks the
  allocator to return dirty pages after unloading the model.
- Keep `[tts].process_isolation = true` for VoxCPM in the local LaunchAgent.
  VoxCPM/Candle can retain large native or Metal buffer pools across repeated
  chunks; running synthesis in a short-lived `cortex tts-worker` process gives
  the OS a hard cleanup boundary after each audio job.
- Keep `[tts].worker_memory_limit_mb` below the amount that would make the Mac
  unusable. The default local config uses `24576` MB; the parent Cortex process
  kills the worker if it crosses that limit.
- `[tts].worker_idle_timeout_secs` is an idle-progress timeout, not a total
  generation timeout. Long audio jobs may run past it as long as each chunk keeps
  updating worker progress.
- Radio production uses VoxCPM with Metal acceleration in the isolated worker:
  `engine = "voxcpm_metal"`, `device = "metal"`,
  `process_isolation = true`. Do not run Metal VoxCPM in the long-running parent
  Cortex process. Experimental non-VoxCPM engines such as Qwen3, Magic-TTS, and
  MOSS require `FRESHLOOP_ALLOW_EXPERIMENTAL_TTS=1`; do not set it in the
  LaunchAgent.
- Metal requests fail fast if the runtime cannot create a Metal device. Do not
  allow a silent CPU fallback for Radio TTS; it hides the real runtime problem
  and can stall audio production for hours.
- The isolated worker must split long LLM output into TTS-safe chunks before
  VoxCPM sees it. It also re-splits incoming worker requests so old draft caches
  cannot reintroduce 1000+ character chunks.
- Worker progress is heartbeat-based. The parent should kill a worker only when
  there is no progress heartbeat for `[tts].worker_idle_timeout_secs`, not when a
  legitimate long synthesis exceeds a wall-clock duration.
- `./scripts/install_local_service.sh` runs a deployment-blocking ASR closed-loop
  check before replacing the installed Cortex binary. The check synthesizes a
  long Chinese news-style script into per-chunk WAV files, transcribes every
  chunk, and fails deployment if any chunk has low pinyin similarity or if the
  later chunks degrade materially versus the first half. It also compares ASR
  text with the configured voice prompt and fails prompt-leakage cases where the
  model starts repeating the reference audio text. Reports are written under
  `/tmp/freshloop-tts-asr-loop/`.
- VoxCPM prompt-cache generation must keep `max_len` bounded by input text
  length. Otherwise short text chunks can run against the full configured audio
  limit and make generation look hung even with hardware acceleration.
- If Activity Monitor shows Cortex footprint in the tens of GB, inspect with:

```bash
PID="$(pgrep -f '^/Users/jinliang/.freshloop/bin/cortex$' | head -n1)"
vmmap -summary "$PID" | sed -n '1,120p'
```

## Feed API Verification

After changing Nexus Feed routes, Cortex curated feed production, Web Reading,
or Android Reading clients, run:

```bash
NEXUS_KEY="$(python3 -c 'import tomllib; print(tomllib.load(open("config.toml","rb"))["nexus"]["auth_key"])')" ./scripts/verify_feed_api.sh
```

The script verifies:

- Cortex `/api/status` reports curated feed enabled.
- Public Nexus feed APIs return valid JSON.
- The first curated article has readable content.
- Reading progress accepts unauthenticated guest no-op updates.
- Internal feed write rejects missing auth.
- Internal feed write accepts local `/audio/...` URLs when `NEXUS_KEY` is set.
