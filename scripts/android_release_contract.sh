#!/bin/bash

set -euo pipefail

CANONICAL_BASE_URL="https://news.hackerlife.fun:8443"
CANONICAL_DOWNLOAD_URL="$CANONICAL_BASE_URL/android-app.apk"
EXPECTED_PACKAGE_NAME="fun.hackerlife.freshloop"
EXPECTED_APP_LABEL="FreshLoop"

die() {
    echo "Android release contract failed: $*" >&2
    exit 1
}

require_file() {
    local path="$1"
    local description="$2"
    [ -f "$path" ] || die "$description not found: $path"
}

require_command() {
    local command_name="$1"
    command -v "$command_name" >/dev/null 2>&1 ||
        die "required command is unavailable: $command_name"
}

load_pubspec_version() {
    local pubspec_path="$1"
    local version_line

    require_file "$pubspec_path" "pubspec"
    version_line="$(
        awk '/^version:[[:space:]]*/ {
            sub(/^version:[[:space:]]*/, "")
            print
            exit
        }' "$pubspec_path"
    )"

    case "$version_line" in
        *+*)
            PUBSPEC_VERSION_NAME="${version_line%+*}"
            PUBSPEC_VERSION_CODE="${version_line##*+}"
            ;;
        *)
            die "pubspec version must use versionName+versionCode: $version_line"
            ;;
    esac

    if ! [[ "$PUBSPEC_VERSION_NAME" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
        die "invalid Android version name in pubspec: $PUBSPEC_VERSION_NAME"
    fi
    if ! [[ "$PUBSPEC_VERSION_CODE" =~ ^[0-9]+$ ]]; then
        die "invalid Android build number in pubspec: $PUBSPEC_VERSION_CODE"
    fi
}

load_version_json() {
    local json_path="$1"
    local json_values

    require_file "$json_path" "version.json"
    require_command python3
    if ! json_values="$(
        python3 - "$json_path" <<'PY'
import json
import sys

path = sys.argv[1]
with open(path, "r", encoding="utf-8") as handle:
    payload = json.load(handle)

version = payload.get("version")
build_number = payload.get("build_number")
download_url = payload.get("download_url")

if not isinstance(version, str) or not version:
    raise SystemExit("version.json field 'version' must be a non-empty string")
if (
    isinstance(build_number, bool)
    or not isinstance(build_number, int)
    or build_number < 1
):
    raise SystemExit("version.json field 'build_number' must be a positive integer")
if not isinstance(download_url, str) or not download_url:
    raise SystemExit("version.json field 'download_url' must be a non-empty string")

print(version)
print(build_number)
print(download_url)
PY
    )"; then
        die "invalid version metadata: $json_path"
    fi

    JSON_VERSION_NAME="$(printf '%s\n' "$json_values" | sed -n '1p')"
    JSON_VERSION_CODE="$(printf '%s\n' "$json_values" | sed -n '2p')"
    JSON_DOWNLOAD_URL="$(printf '%s\n' "$json_values" | sed -n '3p')"
}

resolve_aapt() {
    local discovered_aapt

    if [ -n "${ANDROID_RELEASE_AAPT:-}" ]; then
        [ -x "$ANDROID_RELEASE_AAPT" ] ||
            die "ANDROID_RELEASE_AAPT is not executable: $ANDROID_RELEASE_AAPT"
        AAPT_BIN="$ANDROID_RELEASE_AAPT"
        return
    fi

    if command -v aapt >/dev/null 2>&1; then
        AAPT_BIN="$(command -v aapt)"
        return
    fi

    require_command python3
    discovered_aapt="$(
        python3 - <<'PY'
import glob
import os
import re

roots = []
for variable in ("ANDROID_SDK_ROOT", "ANDROID_HOME"):
    value = os.environ.get(variable)
    if value:
        roots.append(value)
roots.append(os.path.expanduser("~/Library/Android/sdk"))

def version_key(path):
    build_tools_version = os.path.basename(os.path.dirname(path))
    return tuple(
        int(part) if part.isdigit() else part
        for part in re.split(r"([0-9]+)", build_tools_version)
    )

candidates = []
for root in roots:
    candidates.extend(glob.glob(os.path.join(root, "build-tools", "*", "aapt")))
candidates = [path for path in candidates if os.access(path, os.X_OK)]
if candidates:
    print(max(candidates, key=version_key))
PY
    )"

    [ -n "$discovered_aapt" ] ||
        die "aapt was not found; install Android SDK build-tools or set ANDROID_RELEASE_AAPT"
    AAPT_BIN="$discovered_aapt"
}

resolve_apksigner() {
    local discovered_apksigner

    if [ -n "${ANDROID_RELEASE_APKSIGNER:-}" ]; then
        [ -x "$ANDROID_RELEASE_APKSIGNER" ] ||
            die "ANDROID_RELEASE_APKSIGNER is not executable: $ANDROID_RELEASE_APKSIGNER"
        configure_java_for_apksigner
        APKSIGNER_BIN="$ANDROID_RELEASE_APKSIGNER"
        return
    fi

    if command -v apksigner >/dev/null 2>&1; then
        configure_java_for_apksigner
        APKSIGNER_BIN="$(command -v apksigner)"
        return
    fi

    require_command python3
    discovered_apksigner="$(
        python3 - <<'PY'
import glob
import os
import re

roots = []
for variable in ("ANDROID_SDK_ROOT", "ANDROID_HOME"):
    value = os.environ.get(variable)
    if value:
        roots.append(value)
roots.append(os.path.expanduser("~/Library/Android/sdk"))

def version_key(path):
    build_tools_version = os.path.basename(os.path.dirname(path))
    return tuple(
        int(part) if part.isdigit() else part
        for part in re.split(r"([0-9]+)", build_tools_version)
    )

candidates = []
for root in roots:
    candidates.extend(glob.glob(os.path.join(root, "build-tools", "*", "apksigner")))
candidates = [path for path in candidates if os.access(path, os.X_OK)]
if candidates:
    print(max(candidates, key=version_key))
PY
    )"

    [ -n "$discovered_apksigner" ] ||
        die "apksigner was not found; install Android SDK build-tools or set ANDROID_RELEASE_APKSIGNER"
    configure_java_for_apksigner
    APKSIGNER_BIN="$discovered_apksigner"
}

configure_java_for_apksigner() {
    local openjdk_prefix=""

    if java -version >/dev/null 2>&1; then
        return
    fi
    if command -v brew >/dev/null 2>&1; then
        openjdk_prefix="$(brew --prefix openjdk@17 2>/dev/null || true)"
    fi
    if [ -n "$openjdk_prefix" ]; then
        if [ -d "$openjdk_prefix/libexec/openjdk.jdk/Contents/Home" ]; then
            export JAVA_HOME="$openjdk_prefix/libexec/openjdk.jdk/Contents/Home"
        else
            export JAVA_HOME="$openjdk_prefix"
        fi
        export PATH="$JAVA_HOME/bin:$PATH"
    fi
    java -version >/dev/null 2>&1 ||
        die "Java is unavailable; apksigner requires Homebrew openjdk@17"
}

load_apk_manifest() {
    local apk_path="$1"
    local badging
    local package_line

    require_file "$apk_path" "APK"
    resolve_aapt
    if ! badging="$("$AAPT_BIN" dump badging "$apk_path" 2>&1)"; then
        die "aapt could not inspect APK $apk_path: $badging"
    fi

    package_line="$(printf '%s\n' "$badging" | sed -n '/^package:/p' | head -n 1)"
    APK_PACKAGE_NAME="$(
        printf '%s\n' "$package_line" |
            sed -n "s/^package: name='\([^']*\)'.*/\1/p"
    )"
    APK_VERSION_CODE="$(
        printf '%s\n' "$package_line" |
            sed -n "s/.*versionCode='\([^']*\)'.*/\1/p"
    )"
    APK_VERSION_NAME="$(
        printf '%s\n' "$package_line" |
            sed -n "s/.*versionName='\([^']*\)'.*/\1/p"
    )"
    APK_APP_LABEL="$(
        printf '%s\n' "$badging" |
            sed -n "s/^application-label:'\([^']*\)'/\1/p" |
            head -n 1
    )"

    [ -n "$APK_PACKAGE_NAME" ] || die "APK package name is missing: $apk_path"
    [ -n "$APK_VERSION_NAME" ] || die "APK versionName is missing: $apk_path"
    [ -n "$APK_VERSION_CODE" ] || die "APK versionCode is missing: $apk_path"
    [ -n "$APK_APP_LABEL" ] || die "APK application label is missing: $apk_path"
}

sha256_file() {
    local path="$1"

    require_file "$path" "file for SHA-256"
    if command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$path" | awk '{print $1}'
        return
    fi
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$path" | awk '{print $1}'
        return
    fi
    die "neither shasum nor sha256sum is available"
}

verify_apk_against_pubspec() {
    local pubspec_path="$1"
    local apk_path="$2"
    local expected_version_name
    local expected_version_code

    load_pubspec_version "$pubspec_path"
    expected_version_name="$PUBSPEC_VERSION_NAME"
    expected_version_code="$PUBSPEC_VERSION_CODE"
    load_apk_manifest "$apk_path"

    [ "$APK_PACKAGE_NAME" = "$EXPECTED_PACKAGE_NAME" ] ||
        die "APK package $APK_PACKAGE_NAME does not match $EXPECTED_PACKAGE_NAME"
    [ "$APK_APP_LABEL" = "$EXPECTED_APP_LABEL" ] ||
        die "APK label $APK_APP_LABEL does not match $EXPECTED_APP_LABEL"
    if [ "$APK_VERSION_NAME" != "$expected_version_name" ] ||
        [ "$APK_VERSION_CODE" != "$expected_version_code" ]; then
        die "APK version $APK_VERSION_NAME+$APK_VERSION_CODE does not match pubspec $expected_version_name+$expected_version_code"
    fi
}

verify_json_against_apk() {
    local json_path="$1"
    local apk_path="$2"
    local apk_version_name
    local apk_version_code

    load_apk_manifest "$apk_path"
    apk_version_name="$APK_VERSION_NAME"
    apk_version_code="$APK_VERSION_CODE"
    load_version_json "$json_path"

    if [ "$JSON_VERSION_NAME" != "$apk_version_name" ] ||
        [ "$JSON_VERSION_CODE" != "$apk_version_code" ]; then
        die "version.json $JSON_VERSION_NAME+$JSON_VERSION_CODE does not match APK $apk_version_name+$apk_version_code"
    fi
    [ "$JSON_DOWNLOAD_URL" = "$CANONICAL_DOWNLOAD_URL" ] ||
        die "version.json download URL must be $CANONICAL_DOWNLOAD_URL"
}

assert_newer() {
    local pubspec_path="$1"
    local remote_version_json="$2"
    local local_version_name
    local local_version_code
    local published_version_name
    local published_version_code

    load_pubspec_version "$pubspec_path"
    local_version_name="$PUBSPEC_VERSION_NAME"
    local_version_code="$PUBSPEC_VERSION_CODE"
    load_version_json "$remote_version_json"
    published_version_name="$JSON_VERSION_NAME"
    published_version_code="$JSON_VERSION_CODE"

    if [ "$local_version_code" -le "$published_version_code" ]; then
        die "local Android build $local_version_name+$local_version_code must be greater than published build $published_version_code ($published_version_name+$published_version_code)"
    fi

    echo "Android release precondition verified: $local_version_name+$local_version_code > published build $published_version_code."
}

bump_pubspec_version() {
    local pubspec_path="$1"
    local bump_type="$2"
    local old_version_name="$PUBSPEC_VERSION_NAME"
    local old_version_code="$PUBSPEC_VERSION_CODE"
    local major
    local minor
    local patch
    local new_version_name

    IFS=. read -r major minor patch <<EOF
$old_version_name
EOF
    case "$bump_type" in
        major)
            major=$((major + 1))
            minor=0
            patch=0
            ;;
        minor)
            minor=$((minor + 1))
            patch=0
            ;;
        patch)
            patch=$((patch + 1))
            ;;
        *)
            die "unknown Android bump type: $bump_type (use patch, minor, or major)"
            ;;
    esac
    new_version_name="$major.$minor.$patch"

    require_command python3
    python3 - "$pubspec_path" "$old_version_name+$old_version_code" \
        "$new_version_name+$((old_version_code + 1))" <<'PY'
import pathlib
import re
import sys

path = pathlib.Path(sys.argv[1])
old_version = sys.argv[2]
new_version = sys.argv[3]
text = path.read_text(encoding="utf-8")
updated, count = re.subn(
    rf"^version:\s+{re.escape(old_version)}\s*$",
    f"version: {new_version}",
    text,
    count=1,
    flags=re.MULTILINE,
)
if count != 1:
    raise SystemExit(f"could not update version in {path}")
path.write_text(updated, encoding="utf-8")
PY
    echo "Bumped Android version from $old_version_name+$old_version_code to $new_version_name+$((old_version_code + 1))."
}

prepare_version() {
    local pubspec_path="$1"
    local remote_version_json="$2"
    local bump_type="${3:-patch}"
    local local_version_name
    local local_version_code
    local published_version_name
    local published_version_code

    load_pubspec_version "$pubspec_path"
    local_version_name="$PUBSPEC_VERSION_NAME"
    local_version_code="$PUBSPEC_VERSION_CODE"
    load_version_json "$remote_version_json"
    published_version_name="$JSON_VERSION_NAME"
    published_version_code="$JSON_VERSION_CODE"

    if [ "$local_version_code" -lt "$published_version_code" ]; then
        die "local Android build $local_version_code is behind published build $published_version_code; update the checkout before releasing"
    fi
    if [ "$local_version_code" -eq "$published_version_code" ]; then
        if [ "$local_version_name" != "$published_version_name" ]; then
            die "local and published Android builds both use $local_version_code but version names differ ($local_version_name vs $published_version_name)"
        fi
        load_pubspec_version "$pubspec_path"
        bump_pubspec_version "$pubspec_path" "$bump_type"
    else
        echo "Keeping unpublished Android version $local_version_name+$local_version_code; no additional bump needed."
    fi

    assert_newer "$pubspec_path" "$remote_version_json"
}

verify_local() {
    local pubspec_path="$1"
    local apk_path="$2"
    local version_json="$3"

    verify_apk_against_pubspec "$pubspec_path" "$apk_path"
    verify_json_against_apk "$version_json" "$apk_path"
    echo "Local Android release contract verified."
}

verify_apk_copy() {
    local built_apk="$1"
    local copied_apk="$2"
    local built_sha
    local copied_sha

    built_sha="$(sha256_file "$built_apk")"
    copied_sha="$(sha256_file "$copied_apk")"
    [ "$built_sha" = "$copied_sha" ] ||
        die "copied APK SHA-256 does not match built APK (built $built_sha, copied $copied_sha)"

    echo "APK copy verified: SHA-256 $built_sha."
}

apk_signer_digest() {
    local apk_path="$1"
    local signer_output
    local signer_digest

    require_file "$apk_path" "APK"
    resolve_apksigner
    if ! signer_output="$("$APKSIGNER_BIN" verify --print-certs "$apk_path" 2>&1)"; then
        die "apksigner could not verify APK $apk_path: $signer_output"
    fi
    signer_digest="$(
        printf '%s\n' "$signer_output" |
            sed -n 's/^Signer #1 certificate SHA-256 digest: //p' |
            head -n 1
    )"
    [ -n "$signer_digest" ] || die "APK signer certificate digest is missing: $apk_path"
    printf '%s\n' "$signer_digest"
}

verify_signer() {
    local candidate_apk="$1"
    local published_apk="$2"
    local candidate_signer
    local published_signer

    candidate_signer="$(apk_signer_digest "$candidate_apk")"
    published_signer="$(apk_signer_digest "$published_apk")"
    [ "$candidate_signer" = "$published_signer" ] ||
        die "APK signer certificate does not match the published app"

    echo "Android signer certificate continuity verified."
}

verify_static_bundle() {
    local local_version_json="$1"
    local local_apk="$2"
    local remote_version_json="$3"
    local remote_apk="$4"
    local local_json_sha
    local remote_json_sha
    local local_apk_sha
    local remote_apk_sha

    local_json_sha="$(sha256_file "$local_version_json")"
    remote_json_sha="$(sha256_file "$remote_version_json")"
    [ "$local_json_sha" = "$remote_json_sha" ] ||
        die "frontend deploy would change published Android version.json; use --android"

    local_apk_sha="$(sha256_file "$local_apk")"
    remote_apk_sha="$(sha256_file "$remote_apk")"
    [ "$local_apk_sha" = "$remote_apk_sha" ] ||
        die "frontend deploy would change the published Android APK; use --android"

    echo "Frontend bundle preserves the published Android artifacts."
}

verify_published() {
    local pubspec_path="$1"
    local local_apk="$2"
    local remote_version_json="$3"
    local remote_apk="$4"
    local local_sha
    local remote_sha

    verify_apk_against_pubspec "$pubspec_path" "$local_apk"
    verify_apk_against_pubspec "$pubspec_path" "$remote_apk"
    verify_json_against_apk "$remote_version_json" "$remote_apk"
    verify_signer "$local_apk" "$remote_apk"

    local_sha="$(sha256_file "$local_apk")"
    remote_sha="$(sha256_file "$remote_apk")"
    [ "$local_sha" = "$remote_sha" ] ||
        die "published APK SHA-256 does not match local APK (local $local_sha, published $remote_sha)"

    echo "Published Android release verified: APK SHA-256 $local_sha."
}

generate_version_json() {
    local pubspec_path="$1"
    local output_path="$2"
    local release_notes="${3:-最新修复和优化。}"
    local output_dir
    local temp_output

    load_pubspec_version "$pubspec_path"
    output_dir="$(dirname "$output_path")"
    [ -d "$output_dir" ] || die "version.json output directory not found: $output_dir"
    temp_output="$(mktemp "$output_dir/.version.json.XXXXXX")"
    if ! python3 - "$PUBSPEC_VERSION_NAME" "$PUBSPEC_VERSION_CODE" \
        "$CANONICAL_DOWNLOAD_URL" "$release_notes" > "$temp_output" <<'PY'
import json
import sys

payload = {
    "version": sys.argv[1],
    "build_number": int(sys.argv[2]),
    "download_url": sys.argv[3],
    "release_notes": sys.argv[4],
}
json.dump(payload, sys.stdout, ensure_ascii=False, indent=2)
sys.stdout.write("\n")
PY
    then
        rm -f "$temp_output"
        die "could not generate version metadata"
    fi
    mv "$temp_output" "$output_path"
    echo "Generated canonical Android version metadata: $PUBSPEC_VERSION_NAME+$PUBSPEC_VERSION_CODE."
}

resolve_curl() {
    if [ -n "${ANDROID_RELEASE_CURL:-}" ]; then
        [ -x "$ANDROID_RELEASE_CURL" ] ||
            die "ANDROID_RELEASE_CURL is not executable: $ANDROID_RELEASE_CURL"
        CURL_BIN="$ANDROID_RELEASE_CURL"
        return
    fi
    require_command curl
    CURL_BIN="$(command -v curl)"
}

fetch_public_artifact() {
    local url="$1"
    local output_path="$2"
    local cache_buster

    resolve_curl
    cache_buster="$(date +%s)"
    "$CURL_BIN" \
        --fail \
        --silent \
        --show-error \
        --location \
        --retry 3 \
        --connect-timeout 10 \
        --max-time 120 \
        -H "Cache-Control: no-cache" \
        "${url}?freshloop_release_check=$cache_buster" \
        -o "$output_path"
}

fetch_published() {
    local output_dir="$1"

    [ -d "$output_dir" ] || die "published artifact output directory not found: $output_dir"
    resolve_curl
    fetch_public_artifact "$CANONICAL_BASE_URL/version.json" "$output_dir/version.json"
    fetch_public_artifact "$CANONICAL_DOWNLOAD_URL" "$output_dir/android-app.apk"
    echo "Downloaded published Android artifacts from $CANONICAL_BASE_URL."
}

preflight() {
    require_command python3
    resolve_curl
    resolve_aapt
    resolve_apksigner
    if ! command -v shasum >/dev/null 2>&1 &&
        ! command -v sha256sum >/dev/null 2>&1; then
        die "neither shasum nor sha256sum is available"
    fi
    echo "Android release tools verified:"
    echo "  curl: $CURL_BIN"
    echo "  aapt: $AAPT_BIN"
    echo "  apksigner: $APKSIGNER_BIN"
}

assert_newer_online() {
    local pubspec_path="$1"
    local temp_dir
    local status=0

    preflight
    temp_dir="$(mktemp -d "${TMPDIR:-/tmp}/freshloop-release-preflight.XXXXXX")"
    set +e
    fetch_public_artifact "$CANONICAL_BASE_URL/version.json" "$temp_dir/version.json"
    status=$?
    if [ "$status" -eq 0 ]; then
        assert_newer "$pubspec_path" "$temp_dir/version.json"
        status=$?
    fi
    set -e
    rm -rf "$temp_dir"
    return "$status"
}

verify_published_online() {
    local pubspec_path="$1"
    local local_apk="$2"
    local temp_dir
    local status=1
    local attempt

    preflight
    temp_dir="$(mktemp -d "${TMPDIR:-/tmp}/freshloop-release-published.XXXXXX")"
    for attempt in 1 2 3; do
        rm -f "$temp_dir/version.json" "$temp_dir/android-app.apk"
        set +e
        (
            fetch_published "$temp_dir" &&
                verify_published \
                    "$pubspec_path" \
                    "$local_apk" \
                    "$temp_dir/version.json" \
                    "$temp_dir/android-app.apk"
        )
        status=$?
        set -e
        if [ "$status" -eq 0 ]; then
            break
        fi
        if [ "$attempt" -lt 3 ]; then
            echo "Published Android verification attempt $attempt failed; retrying..." >&2
            sleep $((attempt * 2))
        fi
    done
    rm -rf "$temp_dir"
    return "$status"
}

usage() {
    cat <<'EOF'
Usage:
  android_release_contract.sh preflight
  android_release_contract.sh fetch-published <output-directory>
  android_release_contract.sh assert-newer <pubspec.yaml> <published-version.json>
  android_release_contract.sh assert-newer-online <pubspec.yaml>
  android_release_contract.sh prepare-version <pubspec.yaml> <published-version.json> [patch|minor|major]
  android_release_contract.sh generate-version-json <pubspec.yaml> <output.json> [release-notes]
  android_release_contract.sh verify-local <pubspec.yaml> <apk> <version.json>
  android_release_contract.sh verify-apk-copy <built-apk> <copied-apk>
  android_release_contract.sh verify-signer <candidate-apk> <published-apk>
  android_release_contract.sh verify-static-bundle <local-version.json> <local-apk> <published-version.json> <published-apk>
  android_release_contract.sh verify-published <pubspec.yaml> <local-apk> <published-version.json> <published-apk>
  android_release_contract.sh verify-published-online <pubspec.yaml> <local-apk>

The public update endpoint is fixed to https://news.hackerlife.fun:8443.
EOF
}

COMMAND="${1:-}"
case "$COMMAND" in
    preflight)
        [ "$#" -eq 1 ] || die "preflight takes no arguments"
        preflight
        ;;
    fetch-published)
        [ "$#" -eq 2 ] || die "fetch-published requires an output directory"
        fetch_published "$2"
        ;;
    assert-newer)
        [ "$#" -eq 3 ] || die "assert-newer requires pubspec and published version.json"
        assert_newer "$2" "$3"
        ;;
    assert-newer-online)
        [ "$#" -eq 2 ] || die "assert-newer-online requires pubspec"
        assert_newer_online "$2"
        ;;
    prepare-version)
        [ "$#" -ge 3 ] && [ "$#" -le 4 ] ||
            die "prepare-version requires pubspec, published version.json, and optional bump type"
        prepare_version "$2" "$3" "${4:-patch}"
        ;;
    generate-version-json)
        [ "$#" -ge 3 ] && [ "$#" -le 4 ] ||
            die "generate-version-json requires pubspec, output, and optional release notes"
        generate_version_json "$2" "$3" "${4:-最新修复和优化。}"
        ;;
    verify-local)
        [ "$#" -eq 4 ] || die "verify-local requires pubspec, APK, and version.json"
        verify_local "$2" "$3" "$4"
        ;;
    verify-apk-copy)
        [ "$#" -eq 3 ] || die "verify-apk-copy requires built and copied APK paths"
        verify_apk_copy "$2" "$3"
        ;;
    verify-signer)
        [ "$#" -eq 3 ] || die "verify-signer requires candidate and published APK paths"
        verify_signer "$2" "$3"
        ;;
    verify-static-bundle)
        [ "$#" -eq 5 ] ||
            die "verify-static-bundle requires local version.json, local APK, published version.json, and published APK"
        verify_static_bundle "$2" "$3" "$4" "$5"
        ;;
    verify-published)
        [ "$#" -eq 5 ] ||
            die "verify-published requires pubspec, local APK, published version.json, and published APK"
        verify_published "$2" "$3" "$4" "$5"
        ;;
    verify-published-online)
        [ "$#" -eq 3 ] || die "verify-published-online requires pubspec and local APK"
        verify_published_online "$2" "$3"
        ;;
    -h|--help|help)
        usage
        ;;
    *)
        usage >&2
        exit 2
        ;;
esac
