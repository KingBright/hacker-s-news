#!/bin/bash
set -euo pipefail

BASE_URL="${BASE_URL:-https://news.hackerlife.fun:8443}"
CORTEX_URL="${CORTEX_URL:-http://localhost:3721}"
EXPECT_ITEMS="${EXPECT_ITEMS:-1}"
NEXUS_KEY="${NEXUS_KEY:-}"

tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

say() {
    printf '>>> %s\n' "$1"
}

request() {
    local method="$1"
    local url="$2"
    local body_file="$3"
    shift 3
    curl -sS -X "$method" "$url" "$@" -o "$body_file" -w '%{http_code}'
}

assert_status() {
    local actual="$1"
    local expected="$2"
    local label="$3"
    if [ "$actual" != "$expected" ]; then
        printf 'FAIL %s: expected HTTP %s, got %s\n' "$label" "$expected" "$actual" >&2
        sed -n '1,80p' "$tmpdir/body" >&2 || true
        exit 1
    fi
    printf 'PASS %s: HTTP %s\n' "$label" "$actual"
}

json_len() {
    python3 - "$1" <<'PY'
import json, sys
with open(sys.argv[1], encoding="utf-8") as f:
    data = json.load(f)
print(len(data) if isinstance(data, list) else -1)
PY
}

json_get() {
    python3 - "$1" "$2" <<'PY'
import json, sys
with open(sys.argv[1], encoding="utf-8") as f:
    data = json.load(f)
path = sys.argv[2].split(".")
value = data
for part in path:
    if part == "0":
        value = value[0]
    else:
        value = value.get(part)
print("" if value is None else value)
PY
}

say "Cortex status"
status="$(request GET "$CORTEX_URL/api/status" "$tmpdir/body")"
assert_status "$status" "200" "GET /api/status"
python3 - "$tmpdir/body" <<'PY'
import json, sys
data = json.load(open(sys.argv[1], encoding="utf-8"))
assert data.get("curated_feed_enabled") is True, "curated_feed_enabled must be true"
print("PASS /api/status curated_feed_enabled=true")
PY

say "Nexus health"
status="$(request GET "$BASE_URL/api/health" "$tmpdir/body")"
assert_status "$status" "200" "GET /api/health"

say "Feed list"
status="$(request GET "$BASE_URL/api/feed/items?product_line=curated_feed&item_type=article&limit=20" "$tmpdir/items")"
cp "$tmpdir/items" "$tmpdir/body"
assert_status "$status" "200" "GET /api/feed/items"
item_count="$(json_len "$tmpdir/items")"
printf 'Feed item count: %s\n' "$item_count"
if [ "$EXPECT_ITEMS" = "1" ] && [ "$item_count" -lt 1 ]; then
    printf 'FAIL feed list: expected at least one curated article\n' >&2
    exit 1
fi

if [ "$item_count" -gt 0 ]; then
    first_id="$(json_get "$tmpdir/items" "0.id")"
    say "Feed item detail: $first_id"
    status="$(request GET "$BASE_URL/api/feed/items/$first_id" "$tmpdir/body")"
    assert_status "$status" "200" "GET /api/feed/items/{id}"

    status="$(request GET "$BASE_URL/api/feed/items/$first_id/content" "$tmpdir/body")"
    assert_status "$status" "200" "GET /api/feed/items/{id}/content"
    python3 - "$tmpdir/body" "$first_id" <<'PY'
import json, sys
data = json.load(open(sys.argv[1], encoding="utf-8"))
assert data.get("item_id") == sys.argv[2], "content item_id mismatch"
body = (data.get("plain_text") or data.get("reader_markdown") or "").strip()
assert len(body) > 80, "reader content is unexpectedly short"
audio_script = (data.get("audio_script") or "").strip()
if data.get("audio_script") is not None:
    assert len(audio_script) > 40, "audio_script is present but too short"
print("PASS content payload has matching item_id and readable body")
PY

    status="$(request POST "$BASE_URL/api/feed/items/$first_id/progress" "$tmpdir/body" \
        -H 'Content-Type: application/json' \
        --data '{"mode":"original","scroll_ratio":0.42}')"
    assert_status "$status" "200" "POST /api/feed/items/{id}/progress without user"
fi

say "Weekly digest list"
status="$(request GET "$BASE_URL/api/feed/weeklies" "$tmpdir/body")"
assert_status "$status" "200" "GET /api/feed/weeklies"
python3 - "$tmpdir/body" <<'PY'
import json, sys
data = json.load(open(sys.argv[1], encoding="utf-8"))
if data:
    first = data[0]
    manuscript = (first.get("digest_markdown") or "").strip()
    assert manuscript, "weekly digest must include a manuscript"
    if first.get("audio_url"):
        assert first.get("duration_sec") is None or first.get("duration_sec") >= 0
    print("PASS weekly digest payload includes manuscript")
else:
    print("PASS weekly digest list is empty")
PY

say "Internal feed auth"
status="$(request POST "$BASE_URL/api/internal/feed/items" "$tmpdir/body" \
    -H 'Content-Type: application/json' \
    --data '{"item_type":"article","primary_mode":"read","title":"unauthorized smoke"}')"
assert_status "$status" "401" "POST /api/internal/feed/items without key"

if [ -n "$NEXUS_KEY" ]; then
    say "Internal feed smoke write"
    smoke_id="smoke-feed-api"
    payload="$tmpdir/smoke.json"
    cat > "$payload" <<JSON
{
  "id": "$smoke_id",
  "product_line": "curated_feed",
  "item_type": "article",
  "primary_mode": "read",
  "title": "FreshLoop smoke test",
  "source_url": "https://example.com/feed",
  "original_url": "https://example.com/$smoke_id",
  "canonical_url": "https://example.com/$smoke_id",
  "has_audio": true,
  "audio_url": "/audio/$smoke_id.mp3",
  "duration_sec": 12,
  "reading_time_min": 1,
  "quality_score": 1,
  "status": "smoke",
  "content": {
    "plain_text": "This is a non-published smoke test item used to verify the internal feed write interface accepts local audio URLs.",
    "audio_script": "This is the listening script for the non-published smoke test item."
  }
}
JSON
    status="$(request POST "$BASE_URL/api/internal/feed/items" "$tmpdir/body" \
        -H 'Content-Type: application/json' \
        -H "X-NEXUS-KEY: $NEXUS_KEY" \
        --data-binary "@$payload")"
    assert_status "$status" "200" "POST /api/internal/feed/items with key"
    python3 - "$tmpdir/body" "$smoke_id" <<'PY'
import json, sys
data = json.load(open(sys.argv[1], encoding="utf-8"))
assert data.get("id") == sys.argv[2], data
assert data.get("status") in {"created", "updated", "skipped_duplicate"}, data
print("PASS internal smoke write returned expected id/status")
PY
else
    printf 'SKIP internal authorized write: NEXUS_KEY not provided\n'
fi

say "Feed API verification complete"
