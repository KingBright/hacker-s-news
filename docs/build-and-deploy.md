# FreshLoop Build And Deploy

This document records the commands that are considered authoritative for this
repository. If a command here disagrees with memory, trust the command here and
inspect the script before changing behavior.

## Android

There is one release command:

```bash
./scripts/deploy.sh --android
```

No manual version edit is required for the normal patch-release path:

- If the local build equals the build currently published on port 8443, the
  command increments the patch version and build number exactly once.
- If the local build is already newer than production, the command keeps that
  unpublished version. Retrying a failed release therefore does not bump again.
- If the local build is behind production, the command stops. Update the
  checkout instead of overwriting a newer release.

Use `--bump minor` or `--bump major` only when local and production builds still
match and the release intentionally changes that semantic version component.

The release contract then:

- Fetches the current production `version.json` and APK only from
  `https://news.hackerlife.fun:8443`, with cache busting.
- Preflights Java, Flutter, `aapt`, `apksigner`, `curl`, Python, and SHA-256
  tooling before changing the version.
- Builds the arm64 release APK and verifies the copied bytes are identical.
- Verifies package `fun.hackerlife.freshloop`, label `FreshLoop`, manifest
  `versionName`/`versionCode`, canonical download URL, and signing-certificate
  continuity with the currently installed production line.
- Checks that production did not change while the local build was running.
- Uploads the APK and `version.json` together through the frontend bundle.
- Downloads both files back through public port 8443 and checks version,
  manifest, signature, and APK SHA-256. Only then does the script print
  `Deployment Complete`.

In-app update detection only triggers when the remote `build_number` in
`version.json` is higher than the installed Android app's build number. The
release entrypoint enforces that invariant; replacing APK bytes under an old
build number is rejected.

Before editing this mechanism, run its deterministic contract tests:

```bash
./scripts/test_android_release_contract.sh
```

Do not publish Android through bare `flutter build apk`, manual copies,
`./scripts/deploy.sh --frontend`, or direct calls to `scripts/deploy_core.sh`.
Frontend-only and full frontend/backend deployments copy the exact Android
files already online into the generated frontend output, so local unpublished
APK work cannot leak into production.

`scripts/deploy.sh` is intentionally gitignored because it stores target server
coordinates. It must remain a thin configuration wrapper matching
`scripts/deploy.sh.template` and execute the tracked `scripts/deploy_core.sh`.
Never put release logic only in the ignored wrapper; otherwise the mechanism
cannot be reviewed, committed, or restored on another machine.

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
These generation trigger endpoints are available only when
`[content_generation].enabled = true`. App-internal production uses this mode so
Cortex runs RSS fetch, LLM drafting, curated Reading generation, weekly digest
checks, and Loop preference extraction against the local OpenAI-compatible LLM.
For the external scheduled-agent mode, keep `enabled = true` and set
`[content_generation].mode = "external_agent"`. Cortex then returns HTTP `409`
for local generation triggers, retains source ingestion/cache and consumes voice
jobs. `enabled=false` without an explicit mode is the legacy voice-only path;
it does not provide the new source-cache workflow.

Cortex also starts the `voice_worker` loop by default. It polls Nexus
`/api/internal/agent/voice-jobs/lease`, synthesizes queued agent-created audio
jobs, uploads MP3 files through Nexus, and completes the corresponding target
record. See `docs/agent-content-workflow.md` before changing this contract.

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
- `[tts].worker_max_processes` controls how many isolated `cortex tts-worker`
  child processes may run concurrently. Pair it with
  `[voice_worker].concurrency`; otherwise extra voice workers will just queue
  behind the TTS semaphore. Start at `2` for the current 64 GB Mac while the
  single-worker memory limit remains `24576` MB.
- `[voice_worker].repair_missing_audio = true` keeps Reading audio reliable in
  agent-driven mode. Cortex periodically asks Nexus to create missing
  `voice_jobs` for published Reading items or weekly digests that already have
  an `audio_script` but no audio URL.
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


## External editorial mode (v2)

See [scheduled-agent-runbook.md](scheduled-agent-runbook.md) and the canonical
[content agent skill](../skills/freshloop-content-agent/SKILL.md). Use explicit
`[content_generation] mode = "external_agent"` for ingestion/cache plus external
editorial work. The old `enabled=false` switch alone remains voice-only.
Deploy Nexus schema 2 before enabling the new skill. Default self-driven behavior
and the Android release entrypoint are unchanged.

Additional contract check:

```bash
python3 -m unittest discover -s skills/freshloop-content-agent/scripts -p 'test_*.py'
```

For isolated acceptance runs, set `CORTEX_DATA_DIR` to a temporary data directory
and `CORTEX_BIND_ADDR` to a separate loopback port. Keep the config in that run
directory and point Nexus to isolated SQLite, memory and audio paths.
