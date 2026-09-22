#!/bin/bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TASK_TEMP="$(mktemp -d)"
trap 'rm -rf "$TASK_TEMP"' EXIT
# Load only the upload helper, without invoking deployment.
sed -n '/^upload_file() {/,/^}/p' "$ROOT/scripts/deploy_core.sh" > "$TASK_TEMP/upload.sh"
source "$TASK_TEMP/upload.sh"
printf 'a complete release artifact\n' > "$TASK_TEMP/source"
printf 'previous artifact\n' > "$TASK_TEMP/destination"
ATTEMPTS=0
sleep() { :; }
ssh_remote() {
    ATTEMPTS=$((ATTEMPTS+1))
    if [ "$ATTEMPTS" -eq 1 ]; then
        head -c 3 > "$TASK_TEMP/partial"
        test "$(cat "$TASK_TEMP/destination")" = 'previous artifact'
        return 255
    fi
    bash -c "$1"
}
upload_file "$TASK_TEMP/source" "$TASK_TEMP/destination" 600
cmp "$TASK_TEMP/source" "$TASK_TEMP/destination"
test "$ATTEMPTS" -eq 2
printf 'PASS: upload retries reopen input and publish only complete bytes\n'
