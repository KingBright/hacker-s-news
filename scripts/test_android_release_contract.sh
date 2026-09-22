#!/bin/bash

set -u

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
CONTRACT="$ROOT_DIR/scripts/android_release_contract.sh"
TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/freshloop-android-release-test.XXXXXX")"
PASS_COUNT=0
FAIL_COUNT=0

cleanup() {
    rm -rf "$TMP_DIR"
}
trap cleanup EXIT

pass() {
    PASS_COUNT=$((PASS_COUNT + 1))
    echo "PASS: $1"
}

fail() {
    FAIL_COUNT=$((FAIL_COUNT + 1))
    echo "FAIL: $1"
}

expect_success() {
    local name="$1"
    shift
    local output
    local status

    output="$("$@" 2>&1)"
    status=$?
    if [ "$status" -eq 0 ]; then
        pass "$name"
    else
        fail "$name (exit $status)"
        printf '%s\n' "$output"
    fi
}

expect_failure() {
    local name="$1"
    local expected_message="$2"
    shift 2
    local output
    local status

    output="$("$@" 2>&1)"
    status=$?
    if [ "$status" -eq 0 ]; then
        fail "$name (unexpected success)"
        return
    fi
    if ! printf '%s\n' "$output" | grep -F "$expected_message" >/dev/null; then
        fail "$name (missing error: $expected_message)"
        printf '%s\n' "$output"
        return
    fi
    pass "$name"
}

expect_file_contains() {
    local name="$1"
    local path="$2"
    local expected_text="$3"

    if grep -F "$expected_text" "$path" >/dev/null; then
        pass "$name"
    else
        fail "$name (missing: $expected_text)"
    fi
}

expect_text_order() {
    local name="$1"
    local path="$2"
    local first_text="$3"
    local second_text="$4"
    local first_line
    local second_line

    first_line="$(grep -nF "$first_text" "$path" | head -n 1 | cut -d: -f1)"
    second_line="$(grep -nF "$second_text" "$path" | head -n 1 | cut -d: -f1)"
    if [ -n "$first_line" ] && [ -n "$second_line" ] && [ "$first_line" -lt "$second_line" ]; then
        pass "$name"
    else
        fail "$name (expected '$first_text' before '$second_text')"
    fi
}

write_pubspec() {
    local path="$1"
    local version_name="$2"
    local version_code="$3"
    printf 'name: test_app\nversion: %s+%s\n' "$version_name" "$version_code" > "$path"
}

write_version_json() {
    local path="$1"
    local version_name="$2"
    local version_code="$3"
    local download_url="${4:-https://news.hackerlife.fun:8443/android-app.apk}"
    printf '{\n  "version": "%s",\n  "build_number": %s,\n  "download_url": "%s",\n  "release_notes": "test"\n}\n' \
        "$version_name" "$version_code" "$download_url" > "$path"
}

write_fake_apk() {
    local path="$1"
    local payload="$2"
    local version_name="$3"
    local version_code="$4"
    local package_name="${5:-fun.hackerlife.freshloop}"
    local app_label="${6:-FreshLoop}"
    local signer_digest="${7:-TEST-SIGNER-DIGEST}"

    printf '%s\n' "$payload" > "$path"
    printf "package: name='%s' versionCode='%s' versionName='%s'\napplication-label:'%s'\n" \
        "$package_name" "$version_code" "$version_name" "$app_label" > "$path.badging"
    printf 'Signer #1 certificate SHA-256 digest: %s\n' "$signer_digest" > "$path.signer"
}

FAKE_AAPT="$TMP_DIR/fake-aapt"
printf '%s\n' \
    '#!/bin/bash' \
    'if [ "$1" != "dump" ] || [ "$2" != "badging" ]; then' \
    '    echo "unexpected fake aapt arguments" >&2' \
    '    exit 2' \
    'fi' \
    'cat "$3.badging"' > "$FAKE_AAPT"
chmod +x "$FAKE_AAPT"

FAKE_APKSIGNER="$TMP_DIR/fake-apksigner"
printf '%s\n' \
    '#!/bin/bash' \
    'if [ "$1" != "verify" ] || [ "$2" != "--print-certs" ]; then' \
    '    echo "unexpected fake apksigner arguments" >&2' \
    '    exit 2' \
    'fi' \
    'cat "$3.signer"' > "$FAKE_APKSIGNER"
chmod +x "$FAKE_APKSIGNER"

FAKE_CURL="$TMP_DIR/fake-curl"
printf '%s\n' \
    '#!/bin/bash' \
    'if [ "${FAKE_CURL_FAIL:-}" = "1" ]; then' \
    '    echo "simulated network failure" >&2' \
    '    exit 22' \
    'fi' \
    'output_path=""' \
    'url=""' \
    'while [ "$#" -gt 0 ]; do' \
    '    case "$1" in' \
    '        -o)' \
    '            output_path="$2"' \
    '            shift 2' \
    '            ;;' \
    '        https://*)' \
    '            url="$1"' \
    '            shift' \
    '            ;;' \
    '        *)' \
    '            shift' \
    '            ;;' \
    '    esac' \
    'done' \
    'case "$url" in' \
    '    https://news.hackerlife.fun:8443/version.json\?*)' \
    '        cp "$FAKE_PUBLIC_DIR/version.json" "$output_path"' \
    '        ;;' \
    '    https://news.hackerlife.fun:8443/android-app.apk\?*)' \
    '        cp "$FAKE_PUBLIC_DIR/android-app.apk" "$output_path"' \
    '        cp "$FAKE_PUBLIC_DIR/android-app.apk.badging" "$output_path.badging"' \
    '        cp "$FAKE_PUBLIC_DIR/android-app.apk.signer" "$output_path.signer"' \
    '        ;;' \
    '    *)' \
    '        echo "unexpected public URL: $url" >&2' \
    '        exit 3' \
    '        ;;' \
    'esac' > "$FAKE_CURL"
chmod +x "$FAKE_CURL"

PUBSPEC_42="$TMP_DIR/pubspec-42.yaml"
PUBSPEC_41="$TMP_DIR/pubspec-41.yaml"
PUBSPEC_PREPARE_EQUAL="$TMP_DIR/pubspec-prepare-equal.yaml"
PUBSPEC_PREPARE_PENDING="$TMP_DIR/pubspec-prepare-pending.yaml"
PUBSPEC_PREPARE_STALE="$TMP_DIR/pubspec-prepare-stale.yaml"
REMOTE_40_JSON="$TMP_DIR/remote-40.json"
REMOTE_41_JSON="$TMP_DIR/remote-41.json"
REMOTE_42_JSON="$TMP_DIR/remote-42.json"
VERSION_42_JSON="$TMP_DIR/version-42.json"
VERSION_43_JSON="$TMP_DIR/version-43.json"
LOCAL_42_APK="$TMP_DIR/local-42.apk"
SAME_42_APK="$TMP_DIR/same-42.apk"
DIFFERENT_42_APK="$TMP_DIR/different-42.apk"
DIFFERENT_SIGNER_42_APK="$TMP_DIR/different-signer-42.apk"
APK_41="$TMP_DIR/build-41.apk"

write_pubspec "$PUBSPEC_42" "1.3.33" "42"
write_pubspec "$PUBSPEC_41" "1.3.32" "41"
write_pubspec "$PUBSPEC_PREPARE_EQUAL" "1.3.32" "41"
write_pubspec "$PUBSPEC_PREPARE_PENDING" "1.3.33" "42"
write_pubspec "$PUBSPEC_PREPARE_STALE" "1.3.31" "40"
write_version_json "$REMOTE_40_JSON" "1.3.31" "40"
write_version_json "$REMOTE_41_JSON" "1.3.32" "41"
write_version_json "$REMOTE_42_JSON" "1.3.33" "42"
write_version_json "$VERSION_42_JSON" "1.3.33" "42"
write_version_json "$VERSION_43_JSON" "1.3.34" "43"
write_fake_apk "$LOCAL_42_APK" "same apk bytes" "1.3.33" "42"
write_fake_apk "$SAME_42_APK" "same apk bytes" "1.3.33" "42"
write_fake_apk "$DIFFERENT_42_APK" "different apk bytes" "1.3.33" "42"
write_fake_apk \
    "$DIFFERENT_SIGNER_42_APK" \
    "different signer apk bytes" \
    "1.3.33" \
    "42" \
    "fun.hackerlife.freshloop" \
    "FreshLoop" \
    "DIFFERENT-SIGNER-DIGEST"
write_fake_apk "$APK_41" "old apk bytes" "1.3.32" "41"

RUN_CONTRACT=(env \
    "ANDROID_RELEASE_AAPT=$FAKE_AAPT" \
    "ANDROID_RELEASE_APKSIGNER=$FAKE_APKSIGNER" \
    bash "$CONTRACT")

expect_failure \
    "rejects a build number equal to production" \
    "must be greater than published build 41" \
    "${RUN_CONTRACT[@]}" assert-newer "$PUBSPEC_41" "$REMOTE_41_JSON"

expect_failure \
    "rejects a build number lower than production" \
    "must be greater than published build 42" \
    "${RUN_CONTRACT[@]}" assert-newer "$PUBSPEC_41" "$REMOTE_42_JSON"

expect_failure \
    "rejects pubspec and APK manifest mismatch" \
    "APK version 1.3.32+41 does not match pubspec 1.3.33+42" \
    "${RUN_CONTRACT[@]}" verify-local "$PUBSPEC_42" "$APK_41" "$VERSION_42_JSON"

expect_failure \
    "rejects version.json and APK mismatch" \
    "version.json 1.3.34+43 does not match APK 1.3.33+42" \
    "${RUN_CONTRACT[@]}" verify-local "$PUBSPEC_42" "$LOCAL_42_APK" "$VERSION_43_JSON"

expect_failure \
    "rejects a published APK with different bytes" \
    "published APK SHA-256 does not match local APK" \
    "${RUN_CONTRACT[@]}" verify-published \
        "$PUBSPEC_42" "$LOCAL_42_APK" "$REMOTE_42_JSON" "$DIFFERENT_42_APK"

expect_failure \
    "rejects a release signed by a different certificate" \
    "APK signer certificate does not match the published app" \
    "${RUN_CONTRACT[@]}" verify-signer "$LOCAL_42_APK" "$DIFFERENT_SIGNER_42_APK"

expect_failure \
    "rejects an APK copy with different bytes" \
    "copied APK SHA-256 does not match built APK" \
    "${RUN_CONTRACT[@]}" verify-apk-copy "$LOCAL_42_APK" "$DIFFERENT_42_APK"

expect_failure \
    "blocks a frontend-only deploy from changing Android artifacts" \
    "frontend deploy would change the published Android APK" \
    "${RUN_CONTRACT[@]}" verify-static-bundle \
        "$VERSION_42_JSON" "$LOCAL_42_APK" "$REMOTE_42_JSON" "$DIFFERENT_42_APK"

expect_success \
    "accepts a strictly newer local build" \
    "${RUN_CONTRACT[@]}" assert-newer "$PUBSPEC_42" "$REMOTE_40_JSON"

expect_success \
    "bumps an equal version exactly once" \
    "${RUN_CONTRACT[@]}" prepare-version \
        "$PUBSPEC_PREPARE_EQUAL" "$REMOTE_41_JSON" patch
if grep -F "version: 1.3.33+42" "$PUBSPEC_PREPARE_EQUAL" >/dev/null; then
    pass "equal version was bumped to 1.3.33+42"
else
    fail "equal version was not bumped to 1.3.33+42"
fi
expect_success \
    "does not bump the same pending version twice" \
    "${RUN_CONTRACT[@]}" prepare-version \
        "$PUBSPEC_PREPARE_EQUAL" "$REMOTE_41_JSON" patch
if grep -F "version: 1.3.33+42" "$PUBSPEC_PREPARE_EQUAL" >/dev/null; then
    pass "pending version remains 1.3.33+42"
else
    fail "pending version changed during a retry"
fi

expect_success \
    "preserves an existing unpublished version" \
    "${RUN_CONTRACT[@]}" prepare-version \
        "$PUBSPEC_PREPARE_PENDING" "$REMOTE_41_JSON" patch
if grep -F "version: 1.3.33+42" "$PUBSPEC_PREPARE_PENDING" >/dev/null; then
    pass "existing unpublished version remains unchanged"
else
    fail "existing unpublished version changed"
fi

expect_failure \
    "rejects a checkout whose version is behind production" \
    "local Android build 40 is behind published build 41" \
    "${RUN_CONTRACT[@]}" prepare-version \
        "$PUBSPEC_PREPARE_STALE" "$REMOTE_41_JSON" patch

expect_success \
    "accepts matching local artifacts" \
    "${RUN_CONTRACT[@]}" verify-local "$PUBSPEC_42" "$LOCAL_42_APK" "$VERSION_42_JSON"

expect_success \
    "accepts matching published metadata and APK bytes" \
    "${RUN_CONTRACT[@]}" verify-published \
        "$PUBSPEC_42" "$LOCAL_42_APK" "$REMOTE_42_JSON" "$SAME_42_APK"

expect_success \
    "accepts the existing Android signer certificate" \
    "${RUN_CONTRACT[@]}" verify-signer "$LOCAL_42_APK" "$SAME_42_APK"

expect_success \
    "accepts a byte-identical APK copy" \
    "${RUN_CONTRACT[@]}" verify-apk-copy "$LOCAL_42_APK" "$SAME_42_APK"

expect_success \
    "allows frontend-only deploys to preserve Android artifacts" \
    "${RUN_CONTRACT[@]}" verify-static-bundle \
        "$VERSION_42_JSON" "$LOCAL_42_APK" "$REMOTE_42_JSON" "$SAME_42_APK"

PUBLIC_EQUAL_DIR="$TMP_DIR/public-equal"
PUBLIC_MATCH_DIR="$TMP_DIR/public-match"
PUBLIC_DIFFERENT_DIR="$TMP_DIR/public-different"
mkdir -p "$PUBLIC_EQUAL_DIR" "$PUBLIC_MATCH_DIR" "$PUBLIC_DIFFERENT_DIR"
cp "$REMOTE_41_JSON" "$PUBLIC_EQUAL_DIR/version.json"
cp "$REMOTE_42_JSON" "$PUBLIC_MATCH_DIR/version.json"
cp "$REMOTE_42_JSON" "$PUBLIC_DIFFERENT_DIR/version.json"
cp "$SAME_42_APK" "$PUBLIC_MATCH_DIR/android-app.apk"
cp "$SAME_42_APK.badging" "$PUBLIC_MATCH_DIR/android-app.apk.badging"
cp "$SAME_42_APK.signer" "$PUBLIC_MATCH_DIR/android-app.apk.signer"
cp "$DIFFERENT_42_APK" "$PUBLIC_DIFFERENT_DIR/android-app.apk"
cp "$DIFFERENT_42_APK.badging" "$PUBLIC_DIFFERENT_DIR/android-app.apk.badging"
cp "$DIFFERENT_42_APK.signer" "$PUBLIC_DIFFERENT_DIR/android-app.apk.signer"

expect_failure \
    "online preflight propagates a reused build failure" \
    "must be greater than published build 41" \
    env \
        "ANDROID_RELEASE_AAPT=$FAKE_AAPT" \
        "ANDROID_RELEASE_APKSIGNER=$FAKE_APKSIGNER" \
        "ANDROID_RELEASE_CURL=$FAKE_CURL" \
        "FAKE_PUBLIC_DIR=$PUBLIC_EQUAL_DIR" \
        bash "$CONTRACT" assert-newer-online "$PUBSPEC_41"

expect_failure \
    "online preflight propagates a network failure" \
    "simulated network failure" \
    env \
        "ANDROID_RELEASE_AAPT=$FAKE_AAPT" \
        "ANDROID_RELEASE_APKSIGNER=$FAKE_APKSIGNER" \
        "ANDROID_RELEASE_CURL=$FAKE_CURL" \
        "FAKE_CURL_FAIL=1" \
        "FAKE_PUBLIC_DIR=$PUBLIC_EQUAL_DIR" \
        bash "$CONTRACT" assert-newer-online "$PUBSPEC_42"

expect_success \
    "online verification accepts the canonical published artifacts" \
    env \
        "ANDROID_RELEASE_AAPT=$FAKE_AAPT" \
        "ANDROID_RELEASE_APKSIGNER=$FAKE_APKSIGNER" \
        "ANDROID_RELEASE_CURL=$FAKE_CURL" \
        "FAKE_PUBLIC_DIR=$PUBLIC_MATCH_DIR" \
        bash "$CONTRACT" verify-published-online "$PUBSPEC_42" "$LOCAL_42_APK"

expect_failure \
    "online verification propagates a published SHA failure" \
    "published APK SHA-256 does not match local APK" \
    env \
        "ANDROID_RELEASE_AAPT=$FAKE_AAPT" \
        "ANDROID_RELEASE_APKSIGNER=$FAKE_APKSIGNER" \
        "ANDROID_RELEASE_CURL=$FAKE_CURL" \
        "FAKE_PUBLIC_DIR=$PUBLIC_DIFFERENT_DIR" \
        bash "$CONTRACT" verify-published-online "$PUBSPEC_42" "$LOCAL_42_APK"

GENERATED_JSON="$TMP_DIR/generated-version.json"
expect_success \
    "generates canonical update metadata" \
    "${RUN_CONTRACT[@]}" generate-version-json "$PUBSPEC_42" "$GENERATED_JSON" "test release"
expect_success \
    "generated metadata passes the local contract" \
    "${RUN_CONTRACT[@]}" verify-local "$PUBSPEC_42" "$LOCAL_42_APK" "$GENERATED_JSON"

DEPLOY_SCRIPT="$ROOT_DIR/scripts/deploy_core.sh"
expect_file_contains \
    "deploy entrypoint prepares a production-relative version" \
    "$DEPLOY_SCRIPT" \
    '"$ANDROID_RELEASE_CONTRACT" prepare-version'
expect_file_contains \
    "deploy entrypoint verifies local release artifacts" \
    "$DEPLOY_SCRIPT" \
    '"$ANDROID_RELEASE_CONTRACT" verify-local'
expect_file_contains \
    "frontend deploys preserve public Android artifacts" \
    "$DEPLOY_SCRIPT" \
    'cp "$PUBLISHED_ANDROID_DIR/android-app.apk" frontend/out/android-app.apk'
expect_file_contains \
    "Android release entrypoint uses a local process lock" \
    "$DEPLOY_SCRIPT" \
    'freshloop-android-release.lock'
expect_text_order \
    "public verification occurs before the success banner" \
    "$DEPLOY_SCRIPT" \
    '"$ANDROID_RELEASE_CONTRACT" verify-published-online' \
    'Deployment Complete!'
expect_file_contains \
    "deploy template delegates to the tracked release core" \
    "$ROOT_DIR/scripts/deploy.sh.template" \
    'exec "$SCRIPT_DIR/deploy_core.sh" "$@"'

echo ""
echo "Android release contract tests: $PASS_COUNT passed, $FAIL_COUNT failed."
if [ "$FAIL_COUNT" -ne 0 ]; then
    exit 1
fi
