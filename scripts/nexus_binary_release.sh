#!/usr/bin/env bash
# Called only by the canonical verified Nexus deployment; no generation endpoints.
set -euo pipefail
[ "$#" -eq 5 ] || { echo "expected app_dir release_id candidate_sha expected_live_sha candidate" >&2; exit 2; }
app=$1
release_id=$2
candidate_sha=$3
expected_live_sha=$4
candidate=$5
[[ "$app" =~ ^/[a-zA-Z0-9_./-]+$ && "$app" != *"/../"* ]] || exit 2
[[ "$release_id" =~ ^[a-f0-9]{40}-[a-f0-9]{12}$ ]] || exit 2
[[ "$candidate_sha" =~ ^[a-f0-9]{64}$ && "$expected_live_sha" =~ ^[a-f0-9]{64}$ ]] || exit 2
hash() { sha256sum "$1" | awk '{print $1}'; }
running_hash() {
    local pid
    pid=$(systemctl show nexus --property MainPID --value)
    [[ "$pid" =~ ^[1-9][0-9]*$ ]] || return 1
    hash "/proc/$pid/exe"
}
health() {
    [ "$(curl --noproxy '*' --fail --silent --show-error --max-time 3 http://127.0.0.1:8899/api/health)" = ok ]
}
[ "$(hash "$candidate")" = "$candidate_sha" ] || { echo "candidate hash mismatch" >&2; exit 1; }
[ -f "$app/nexus" ] && [ ! -L "$app/nexus" ] || exit 1
lock="$app/.nexus-release.lock"
backup="$app/nexus.rollback.$release_id"
staged="$app/nexus.staged.$release_id"
mkdir "$lock" || { echo "existing release lock; inspect original operation" >&2; exit 1; }
printf '%s\n' "$release_id" > "$lock/release_id"
switched=0
accepted=0
cleanup() {
    local status=$?
    trap - EXIT HUP INT TERM
    if [ "$switched" = 1 ] && [ "$accepted" != 1 ]; then
        if cp -p "$backup" "$staged" && mv -f "$staged" "$app/nexus" &&
           systemctl restart nexus && [ "$(running_hash)" = "$expected_live_sha" ] && health; then
            echo "ROLLBACK_OK $expected_live_sha"
        else
            echo "ROLLBACK_FAILED; backup and release lock retained for recovery" >&2
            exit 1
        fi
    fi
    if [ "$(cat "$lock/release_id")" = "$release_id" ]; then
        rm -f "$lock/release_id"
        rmdir "$lock"
    fi
    exit "$status"
}
trap cleanup EXIT
trap 'exit 130' HUP INT TERM
[ "$(hash "$app/nexus")" = "$expected_live_sha" ] || { echo "installed baseline changed; no replacement" >&2; exit 1; }
[ "$(running_hash)" = "$expected_live_sha" ] || { echo "running baseline differs from installed binary" >&2; exit 1; }
[ ! -e "$backup" ] && [ ! -e "$staged" ] || { echo "existing release artifacts; inspect original operation" >&2; exit 1; }
cp -p "$app/nexus" "$backup"
[ "$(hash "$backup")" = "$expected_live_sha" ] || exit 1
cp "$candidate" "$staged"
chmod 755 "$staged"
[ "$(hash "$staged")" = "$candidate_sha" ] || exit 1
switched=1
mv -f "$staged" "$app/nexus"
systemctl restart nexus
ready=0
for attempt in 1 2 3 4 5 6 7 8 9 10; do
    if health && [ "$(running_hash)" = "$candidate_sha" ]; then ready=1; break; fi
    sleep 1
done
[ "$ready" = 1 ] || { echo "new Nexus health/running hash did not pass" >&2; exit 1; }
code=$(curl --noproxy '*' --silent --output /dev/null --write-out '%{http_code}' --max-time 3 \
    http://127.0.0.1:8899/api/internal/agent/jobs/__release_readonly_probe__/repair-radio)
[ "$code" = 405 ] || { echo "repair route missing after restart" >&2; exit 1; }
accepted=1
printf 'NEXUS_RELEASE_APPLIED source=%s sha256=%s rollback=%s previous_sha256=%s\n' \
    "$release_id" "$candidate_sha" "$backup" "$expected_live_sha"
