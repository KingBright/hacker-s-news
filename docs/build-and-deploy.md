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
