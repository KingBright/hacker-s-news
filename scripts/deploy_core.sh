#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

# Deployment configuration is supplied by the ignored scripts/deploy.sh wrapper.
: "${SERVER:?Set SERVER in scripts/deploy.sh}"
: "${SSH_PORT:?Set SSH_PORT in scripts/deploy.sh}"
: "${APP_DIR:?Set APP_DIR in scripts/deploy.sh}"
: "${DATA_DIR:?Set DATA_DIR in scripts/deploy.sh}"
: "${AUDIO_DIR:?Set AUDIO_DIR in scripts/deploy.sh}"
: "${DOMAIN:?Set DOMAIN in scripts/deploy.sh}"

echo ">>> Using configured deployment target."
SSH_OPTS=(
    -p "$SSH_PORT"
    -o ConnectTimeout=15
    -o ServerAliveInterval=10
    -o ServerAliveCountMax=6
    -o TCPKeepAlive=yes
)
BINARY_NAME="nexus"
PUBLIC_URL="https://${DOMAIN}:8443"
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/tmp/hacker-s-news-target-deploy}"
export CARGO_TARGET_DIR
NEXUS_BINARY="$CARGO_TARGET_DIR/x86_64-unknown-linux-musl/release/nexus"
ANDROID_RELEASE_CONTRACT="$REPO_ROOT/scripts/android_release_contract.sh"
ANDROID_PUBSPEC="android_client/pubspec.yaml"
ANDROID_BUILT_APK="android_client/build/app/outputs/flutter-apk/app-release.apk"
ANDROID_PUBLIC_APK="frontend/public/android-app.apk"
ANDROID_VERSION_JSON="frontend/public/version.json"
PUBLISHED_ANDROID_DIR=""
CURRENT_ANDROID_DIR=""
ANDROID_RELEASE_LOCK_DIR=""
ANDROID_RELEASE_LOCK_HELD=false

cleanup() {
    if [ -n "$PUBLISHED_ANDROID_DIR" ] && [ -d "$PUBLISHED_ANDROID_DIR" ]; then
        rm -rf "$PUBLISHED_ANDROID_DIR"
    fi
    if [ -n "$CURRENT_ANDROID_DIR" ] && [ -d "$CURRENT_ANDROID_DIR" ]; then
        rm -rf "$CURRENT_ANDROID_DIR"
    fi
    if [ "$ANDROID_RELEASE_LOCK_HELD" = true ] &&
        [ -n "$ANDROID_RELEASE_LOCK_DIR" ] &&
        [ -d "$ANDROID_RELEASE_LOCK_DIR" ]; then
        rm -f "$ANDROID_RELEASE_LOCK_DIR/pid"
        rmdir "$ANDROID_RELEASE_LOCK_DIR" 2>/dev/null || true
    fi
}
trap cleanup EXIT

configure_android_build_environment() {
    local openjdk_prefix=""

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

    if [ -x "/Users/jinliang/workspace/flutter/bin/flutter" ]; then
        export PATH="/Users/jinliang/workspace/flutter/bin:$PATH"
    fi
    command -v java >/dev/null 2>&1 || {
        echo "Error: Java is unavailable; install Homebrew openjdk@17 before Android release." >&2
        exit 1
    }
    command -v flutter >/dev/null 2>&1 || {
        echo "Error: Flutter is unavailable; configure the FreshLoop Flutter SDK before Android release." >&2
        exit 1
    }
}

ssh_remote() {
    ssh "${SSH_OPTS[@]}" "$SERVER" "$@"
}

run_remote() {
    local attempt
    for attempt in 1 2 3; do
        if ssh_remote "$@"; then
            return 0
        fi

        echo ">>> Remote command attempt $attempt failed; retrying..."
        sleep $((attempt * 3))
    done

    return 1
}

# Reopen stdin for each retry; publish a complete file atomically.
upload_file() {
    local source="$1" destination="$2" mode="$3" attempt
    local temporary="${destination}.upload-$$"
    for attempt in 1 2 3; do
        if ssh_remote "umask 077; cat > '$temporary' && chmod $mode '$temporary' && mv -f '$temporary' '$destination'" < "$source"; then
            return 0
        fi
        echo ">>> File upload attempt $attempt failed; retrying..."
        sleep $((attempt * 3))
    done
    return 1
}

upload_frontend() {
    local attempt
    for attempt in 1 2 3; do
        if COPYFILE_DISABLE=1 tar --disable-copyfile --no-xattrs -czf - -C frontend out | ssh_remote "rm -rf $APP_DIR/frontend && mkdir -p $APP_DIR/frontend && tar -xzf - -C $APP_DIR/frontend --strip-components=1"; then
            return 0
        fi

        echo ">>> Frontend upload attempt $attempt failed; retrying..."
        sleep $((attempt * 3))
    done

    return 1
}

# Security Configuration
# NEXUS_KEY can be supplied by env, config.toml, or a local secret file.
# If none exist, deploy.sh generates and persists a strong random key.
NEXUS_KEY="${NEXUS_KEY:-}"
NEXUS_KEY_FILE="${NEXUS_KEY_FILE:-.secrets/nexus_key}"

is_placeholder_nexus_key() {
    local legacy_demo_key="my-""secret-key-123"
    local legacy_deploy_key="sk-""secure-hackerlife-2026"
    case "${1:-}" in
        ""|"CHANGE_ME_NEXUS_KEY"|"your-secret-api-key"|"$legacy_demo_key"|"$legacy_deploy_key")
            return 0
            ;;
        *)
            return 1
            ;;
    esac
}

read_config_nexus_key() {
    local config_file="${1:-config.toml}"
    [ -f "$config_file" ] || return 1

    awk '
        /^\[nexus\][[:space:]]*$/ { in_nexus = 1; next }
        /^\[/ { in_nexus = 0 }
        in_nexus && /^[[:space:]]*auth_key[[:space:]]*=/ {
            value = $0
            sub(/^[^=]*=[[:space:]]*/, "", value)
            sub(/^[[:space:]]*"/, "", value)
            sub(/"[[:space:]]*(#.*)?$/, "", value)
            sub(/[[:space:]]+$/, "", value)
            print value
            exit
        }
    ' "$config_file"
}

generate_nexus_key() {
    if command -v openssl >/dev/null 2>&1; then
        openssl rand -hex 32
        return
    fi

    LC_ALL=C tr -dc 'A-Za-z0-9' </dev/urandom | head -c 64
    echo
}

persist_nexus_key() {
    local existing_key=""
    if [ -f "$NEXUS_KEY_FILE" ]; then
        existing_key="$(tr -d '\r\n' < "$NEXUS_KEY_FILE")"
    fi

    if [ "$existing_key" != "$NEXUS_KEY" ]; then
        mkdir -p "$(dirname "$NEXUS_KEY_FILE")"
        (umask 077 && printf '%s\n' "$NEXUS_KEY" > "$NEXUS_KEY_FILE")
        chmod 600 "$NEXUS_KEY_FILE" 2>/dev/null || true
    fi
}

sync_placeholder_config_key() {
    [ -f "config.toml" ] || return

    local config_key
    config_key="$(read_config_nexus_key config.toml || true)"
    if is_placeholder_nexus_key "$config_key"; then
        local tmp_file
        tmp_file="$(mktemp)"
        awk -v key="$NEXUS_KEY" '
            BEGIN { in_nexus = 0; seen_nexus = 0; wrote_key = 0 }
            /^\[nexus\][[:space:]]*$/ {
                in_nexus = 1
                seen_nexus = 1
                print
                next
            }
            /^\[/ {
                if (in_nexus && !wrote_key) {
                    print "auth_key = \"" key "\""
                    wrote_key = 1
                }
                in_nexus = 0
            }
            in_nexus && /^[[:space:]]*auth_key[[:space:]]*=/ {
                print "auth_key = \"" key "\""
                wrote_key = 1
                next
            }
            { print }
            END {
                if (in_nexus && !wrote_key) {
                    print "auth_key = \"" key "\""
                    wrote_key = 1
                }
                if (!seen_nexus) {
                    print ""
                    print "[nexus]"
                    print "auth_key = \"" key "\""
                }
            }
        ' config.toml > "$tmp_file"
        mv "$tmp_file" config.toml
        chmod 600 config.toml 2>/dev/null || true
        echo ">>> Synced generated Nexus key into local config.toml"
    elif [ "$config_key" != "$NEXUS_KEY" ]; then
        echo ">>> Warning: config.toml [nexus].auth_key differs from deploy key; Cortex must use the same key."
    fi
}

ensure_nexus_key() {
    [ "$DEPLOY_NEXUS" = true ] || return 0

    local key_source=""
    if ! is_placeholder_nexus_key "$NEXUS_KEY"; then
        key_source="environment"
    else
        NEXUS_KEY="$(read_config_nexus_key config.toml || true)"
        if ! is_placeholder_nexus_key "$NEXUS_KEY"; then
            key_source="config.toml"
        elif [ -f "$NEXUS_KEY_FILE" ]; then
            NEXUS_KEY="$(tr -d '\r\n' < "$NEXUS_KEY_FILE")"
            if ! is_placeholder_nexus_key "$NEXUS_KEY"; then
                key_source="$NEXUS_KEY_FILE"
            fi
        fi
    fi

    if is_placeholder_nexus_key "$NEXUS_KEY"; then
        NEXUS_KEY="$(generate_nexus_key)"
        key_source="generated"
    fi

    persist_nexus_key
    sync_placeholder_config_key
    echo ">>> Nexus key source: $key_source ($NEXUS_KEY_FILE)"
}

# Argument Parsing
DEPLOY_FRONTEND=false
DEPLOY_NEXUS=false
DEPLOY_ANDROID=false
BUMP_TYPE=""

usage() {
    echo "Usage: $0 [options]"
    echo "Options:"
    echo "  -f, --frontend   Deploy Frontend only"
    echo "  -b, --nexus      Deploy Nexus Service (Backend) only"
    echo "  -a, --android    Release Android through the verified 8443 publish contract"
    echo "  --bump <type>    Version bump when public/local builds match (default: patch)"
    echo "  -h, --help       Show this help message"
    echo ""
    echo "If no options are provided, BOTH Frontend and Nexus will be deployed."
    echo ""
    echo "Environment Variables:"
    echo "  NEXUS_KEY        Optional API key override; generated automatically if omitted"
    echo "  NEXUS_KEY_FILE   Optional local key file path (default: .secrets/nexus_key)"
    exit 1
}

if [ $# -eq 0 ]; then
    DEPLOY_FRONTEND=true
    DEPLOY_NEXUS=true
else
    while [[ $# -gt 0 ]]; do
        case $1 in
            -f|--frontend)
            DEPLOY_FRONTEND=true
            shift
            ;;
            -b|--nexus|--backend)
            DEPLOY_NEXUS=true
            shift
            ;;
            -a|--android)
            DEPLOY_ANDROID=true
            shift
            ;;
            --bump)
            if [ "$#" -lt 2 ]; then
                echo "Missing bump type after --bump"
                usage
            fi
            BUMP_TYPE=$2
            shift 2
            ;;
            -h|--help)
            usage
            ;;
            *)
            echo "Unknown option: $1"
            usage
            ;;
        esac
    done
fi

if [ -n "$BUMP_TYPE" ] && [ "$DEPLOY_ANDROID" != true ]; then
    echo "Error: --bump can only be used with --android."
    exit 1
fi

if [ "$DEPLOY_ANDROID" = true ]; then
    ANDROID_RELEASE_LOCK_DIR="${TMPDIR:-/tmp}/freshloop-android-release.lock"
    if ! mkdir "$ANDROID_RELEASE_LOCK_DIR" 2>/dev/null; then
        lock_pid="$(sed -n '1p' "$ANDROID_RELEASE_LOCK_DIR/pid" 2>/dev/null || true)"
        if [[ "$lock_pid" =~ ^[0-9]+$ ]] && kill -0 "$lock_pid" 2>/dev/null; then
            echo "Error: another FreshLoop Android release is running as PID $lock_pid." >&2
            exit 1
        fi
        rm -f "$ANDROID_RELEASE_LOCK_DIR/pid"
        rmdir "$ANDROID_RELEASE_LOCK_DIR" 2>/dev/null || {
            echo "Error: stale Android release lock could not be cleared: $ANDROID_RELEASE_LOCK_DIR" >&2
            exit 1
        }
        mkdir "$ANDROID_RELEASE_LOCK_DIR"
    fi
    ANDROID_RELEASE_LOCK_HELD=true
    printf '%s\n' "$$" > "$ANDROID_RELEASE_LOCK_DIR/pid"
fi

if [ "$DEPLOY_ANDROID" = true ] && [ "$DEPLOY_FRONTEND" != true ]; then
    DEPLOY_FRONTEND=true
    echo ">>> Android deploy includes frontend publish so version.json and android-app.apk go live together."
fi

ensure_nexus_key

# Android files live inside every frontend bundle. Fetch the currently published
# pair before any build or upload so a frontend-only deploy cannot silently
# replace them and an Android deploy can compare against the real 8443 state.
if [ "$DEPLOY_FRONTEND" = true ]; then
    PUBLISHED_ANDROID_DIR="$(mktemp -d "${TMPDIR:-/tmp}/freshloop-published-android.XXXXXX")"
    "$ANDROID_RELEASE_CONTRACT" fetch-published "$PUBLISHED_ANDROID_DIR"
fi

if [ "$DEPLOY_ANDROID" = true ]; then
    configure_android_build_environment
    "$ANDROID_RELEASE_CONTRACT" preflight
    "$ANDROID_RELEASE_CONTRACT" prepare-version \
        "$ANDROID_PUBSPEC" \
        "$PUBLISHED_ANDROID_DIR/version.json" \
        "${BUMP_TYPE:-patch}"
fi

echo "=========================================="
echo "  Hacker's News Deployment Script"
echo "=========================================="
echo "Server: $SERVER (Port: $SSH_PORT)"
echo "App Directory: $APP_DIR"
if [ "$DEPLOY_NEXUS" = true ]; then
    echo "NEXUS_KEY: ${NEXUS_KEY:0:8}****"
fi
echo "Cargo Target: $CARGO_TARGET_DIR"
echo ""

if [ "$DEPLOY_FRONTEND" = true ]; then
    echo ">>> Target: Frontend"
fi
if [ "$DEPLOY_NEXUS" = true ]; then
    echo ">>> Target: Nexus Service (Binary)"
fi

if [ "$DEPLOY_ANDROID" = true ]; then
    echo ">>> Target: Android APK"
fi

# 0. Build Android APK
if [ "$DEPLOY_ANDROID" = true ]; then
    echo ""
    echo ">>> Building Android APK..."

    cd android_client
    flutter build apk --release --target-platform android-arm64
    echo ">>> Copying APK to Frontend public directory..."
    cp build/app/outputs/flutter-apk/app-release.apk ../frontend/public/android-app.apk
    cd ..

    "$ANDROID_RELEASE_CONTRACT" verify-apk-copy \
        "$ANDROID_BUILT_APK" \
        "$ANDROID_PUBLIC_APK"
    "$ANDROID_RELEASE_CONTRACT" generate-version-json \
        "$ANDROID_PUBSPEC" \
        "$ANDROID_VERSION_JSON"
    "$ANDROID_RELEASE_CONTRACT" verify-local \
        "$ANDROID_PUBSPEC" \
        "$ANDROID_PUBLIC_APK" \
        "$ANDROID_VERSION_JSON"
fi

# 1. Build Frontend
if [ "$DEPLOY_FRONTEND" = true ]; then
    echo ""
    echo ">>> Building Frontend..."

    cd frontend
    npm install
    npm run build
    cd ..

    if [ "$DEPLOY_ANDROID" = true ]; then
        # Close the race between initial version selection and upload. If
        # production changed while this machine was building, stop instead of
        # overwriting a newer release.
        CURRENT_ANDROID_DIR="$(mktemp -d "${TMPDIR:-/tmp}/freshloop-current-android.XXXXXX")"
        "$ANDROID_RELEASE_CONTRACT" fetch-published "$CURRENT_ANDROID_DIR"
        "$ANDROID_RELEASE_CONTRACT" verify-static-bundle \
            "$PUBLISHED_ANDROID_DIR/version.json" \
            "$PUBLISHED_ANDROID_DIR/android-app.apk" \
            "$CURRENT_ANDROID_DIR/version.json" \
            "$CURRENT_ANDROID_DIR/android-app.apk"
        "$ANDROID_RELEASE_CONTRACT" verify-signer \
            "$ANDROID_PUBLIC_APK" \
            "$CURRENT_ANDROID_DIR/android-app.apk"
        rm -rf "$CURRENT_ANDROID_DIR"
        CURRENT_ANDROID_DIR=""
    else
        # A normal frontend/full deploy must carry forward the exact Android
        # artifacts already published. Copy into the generated output only, so
        # an unpublished local APK remains untouched and cannot leak online.
        cp "$PUBLISHED_ANDROID_DIR/version.json" frontend/out/version.json
        cp "$PUBLISHED_ANDROID_DIR/android-app.apk" frontend/out/android-app.apk
        "$ANDROID_RELEASE_CONTRACT" verify-static-bundle \
            frontend/out/version.json \
            frontend/out/android-app.apk \
            "$PUBLISHED_ANDROID_DIR/version.json" \
            "$PUBLISHED_ANDROID_DIR/android-app.apk"
    fi
fi

# 2. Build Backend (Native Cross-compile for Linux x86_64)
if [ "$DEPLOY_NEXUS" = true ]; then
    echo ""
    echo ">>> Building Nexus Binary (Linux x86_64)..."
    # Ensure target is installed
    if ! rustup target list | grep -q "x86_64-unknown-linux-musl (installed)"; then
        echo ">>> Installing x86_64-unknown-linux-musl target..."
        rustup target add x86_64-unknown-linux-musl
    fi

    cd backend
    cargo build -p nexus --target x86_64-unknown-linux-musl --release
    cd ..
fi

# 3. Prepare Remote Directories
echo ""
echo ">>> Preparing remote directories..."
run_remote "mkdir -p $APP_DIR $DATA_DIR $AUDIO_DIR"

# 4. Upload Binaries and Frontend
if [ "$DEPLOY_NEXUS" = true ]; then
    echo ""
    echo ">>> Stopping remote service..."
    run_remote "systemctl stop nexus || true"
fi

if [ "$DEPLOY_FRONTEND" = true ]; then
    echo ""
    echo ">>> Uploading and Extracting Frontend..."
    # We pipe tar over SSH because SCP can be unreliable with certain Dropbear configurations
    upload_frontend
    echo ">>> Frontend uploaded successfully!"
fi

if [ "$DEPLOY_NEXUS" = true ]; then
    echo ""
    echo ">>> Uploading Nexus Binary..."
    upload_file "$NEXUS_BINARY" "$APP_DIR/nexus" 755
fi

# 5. Generate and Upload Configuration (Only needed if Backend deployed or first time, but harmless to update)
if [ "$DEPLOY_NEXUS" = true ]; then
    echo ""
    echo ">>> Generating Configuration File..."
    DEPLOY_TEMP_DIR="$(mktemp -d)"
    trap 'rm -rf "$DEPLOY_TEMP_DIR"' EXIT
    cat <<EOF > "$DEPLOY_TEMP_DIR/config.env"
# Nexus Configuration
# Generated by deploy.sh on $(date)

# Server Configuration
PORT=8899
RUST_LOG=info

# Security - NEXUS_KEY is REQUIRED for authentication
# Must match auth_key in cortex's config.toml [nexus] section
NEXUS_KEY=${NEXUS_KEY}

# Directory Configuration
STATIC_DIR=$APP_DIR/frontend
AUDIO_DIR=$AUDIO_DIR

# Database Configuration
DATABASE_URL=sqlite:$DATA_DIR/nexus.db
EOF

    upload_file "$DEPLOY_TEMP_DIR/config.env" "$APP_DIR/config.env" 600

    # 6. Upload Systemd Service
    echo ""
    echo ">>> Configuring Systemd Service..."
    cat <<EOF > "$DEPLOY_TEMP_DIR/nexus.service"
[Unit]
Description=Nexus News Server
After=network.target
Wants=network-online.target

[Service]
Type=simple
User=root
WorkingDirectory=$APP_DIR
ExecStart=$APP_DIR/nexus
Restart=always
RestartSec=5
EnvironmentFile=$APP_DIR/config.env

[Install]
WantedBy=multi-user.target
EOF

    upload_file "$DEPLOY_TEMP_DIR/nexus.service" /etc/systemd/system/nexus.service 644

    echo ""
    echo ">>> Restarting Service..."
    run_remote "systemctl daemon-reload && systemctl enable nexus && systemctl restart nexus"

    # Wait and check status
    sleep 2
    echo ""
    echo ">>> Service Status:"
    run_remote "systemctl status nexus --no-pager | head -15"
fi

if [ "$DEPLOY_ANDROID" = true ]; then
    echo ""
    echo ">>> Verifying Android release through the public 8443 entrypoint..."
    "$ANDROID_RELEASE_CONTRACT" verify-published-online \
        "$ANDROID_PUBSPEC" \
        "$ANDROID_PUBLIC_APK"
fi

echo ""
echo "=========================================="
echo "  Deployment Complete!"
echo "=========================================="
echo "Nexus is running at $PUBLIC_URL"
echo ""
echo "Useful commands:"
echo "  Check logs:    ssh -p $SSH_PORT $SERVER 'journalctl -u nexus -f'"
echo "  Restart:       ssh -p $SSH_PORT $SERVER 'systemctl restart nexus'"
echo "  Check status:  ssh -p $SSH_PORT $SERVER 'systemctl status nexus'"
