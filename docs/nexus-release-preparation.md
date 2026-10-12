# Nexus release from pinned source

The source commit for this repair is
7d1c58eb84ccf7b785fcf5e2344db36cd895e348. It includes the Reading source-storage fix
and the already-deployed Radio repair/repetition/major rules. Do not release bare
b2a8814: it omits the existing repair endpoint and restores the old major quota.

Run through the existing Remote Hosts Mac Studio workspace. Preparation alone
does not run Cargo, deploy, change shared build storage, or touch schedules:

    python3 -B scripts/prepare_nexus_release.py prepare \
      --source-ref 7d1c58eb84ccf7b785fcf5e2344db36cd895e348 \
      --snapshot .task-work/nexus-release-7d1c58e \
      --reading-fixture .task-work/editorial-quality-20261012/reading-technical-preview-v3/reading_article.json

The snapshot exports committed backend/tts files plus the pinned aha and
qwen3-tts-rs commits and applies only path-manifests.json's compatibility
replacements. It copies no local runtime config, credentials, uncommitted TTS
source, frontend or APK. Path manifests are needed for full workspace resolution;
these TTS packages are outside Nexus's compiled dependency closure. Their
compatibility must still pass locked offline Cargo resolution.

After the shared RH v6 / Pixel build queue explicitly releases the slot:

    python3 -B scripts/prepare_nexus_release.py test --snapshot .task-work/nexus-release-7d1c58e
    python3 -B scripts/prepare_nexus_release.py build --snapshot .task-work/nexus-release-7d1c58e

Each stage reuses /Users/jinliang/rust-target, uses locked offline dependencies,
two jobs and a 5400-second budget. Test runs the complete Nexus suite with the
real >80k Reading source fixture; build requires the exact snapshot's passed
test evidence (at least54 passed tests; the previous8-test subset is insufficient).
Every check compares snapshot inputs against the pinned Git objects and
manifest patches; source/fixture changes invalidate the evidence. Build creates an
ELF64 x86_64 candidate and verified-release.json, never an upload.
Linux build explicitly loads native-target.toml, the existing ignored local
musl linker/strip contract made reviewable; host test configuration is unchanged.
No Docker or server-side compilation is used.

Before publication, obtain and review the current running/installed Nexus
SHA256 through the existing authorized server route. The current health and
capability APIs do not expose that identity. The October10 cached release SHA
f8864929e035b0851f152cdf32b23bf966c01add9775f5eaae21ef90b98377ca
is historical evidence, not proof of today's running hash. Keep the old
binary's rollback path and original hash in the deployment receipt.

Only after final test/build evidence and publication authorization:

    ./scripts/deploy.sh --nexus \
      --verified-release .task-work/nexus-release-7d1c58e/verified-release.json \
      --expected-live-sha REVIEWED_CURRENT_RUNNING_SHA256

This opt-in path exits before legacy key generation, configuration/service-file
upload, frontend build or Cargo. One remote operation validates the installed
and running baseline, saves nexus.rollback.<release-id>, atomically replaces
only nexus, restarts only nexus and checks GET health, running hash and the
existing repair route. Failed health/restart/route validation restores the old
binary and restarts it once. Rollback failure retains the lock and backup.
An uncertain transport result must be inspected; do not replay deployment.

No publishing, repair POST, voice-job creation, TTS, Cortex restart or schedule
trigger belongs to this release. The original October13 07:30 Asia/Shanghai
task remains unchanged; its quality requires observing the real native run.

Local mechanical checks, with mocked services and no Cargo/network:

    python3 -B -m unittest discover -s scripts -p test_nexus_release.py
    bash scripts/test_deploy_upload.sh
    bash -n scripts/deploy_core.sh scripts/nexus_binary_release.sh
