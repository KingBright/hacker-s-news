#!/bin/bash
set -e

APP_NAME="com.freshloop.cortex"
WORK_DIR="$(cd "$(dirname "$0")/.." && pwd)"
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$WORK_DIR/backend/target-local}"
export CARGO_TARGET_DIR
TARGET_DIR="$CARGO_TARGET_DIR"
BINARY_SOURCE="$TARGET_DIR/release/cortex"
CONFIG_SOURCE="$WORK_DIR/config.toml"
LAUNCH_DOMAIN="gui/$(id -u)"

can_write_dir() {
    local dir="$1"
    local probe="$dir/.write-test-$$"
    if mkdir -p "$dir" 2>/dev/null && touch "$probe" >/dev/null 2>&1; then
        rm -f "$probe"
        return 0
    fi
    return 1
}

RUNTIME_MODE="system"
SERVICE_HOME="$HOME"
PLIST_PATH="$HOME/Library/LaunchAgents/$APP_NAME.plist"
BINARY_DEST="$HOME/.freshloop/bin/cortex"
LOG_DIR="$HOME/.freshloop/logs"

if ! can_write_dir "$HOME/.freshloop/bin" || ! can_write_dir "$HOME/Library/LaunchAgents"; then
    RUNTIME_MODE="workspace"
    RUNTIME_ROOT="$WORK_DIR/.runtime/cortex-service"
    SERVICE_HOME="$RUNTIME_ROOT/home"
    PLIST_PATH="$RUNTIME_ROOT/$APP_NAME.plist"
    BINARY_DEST="$RUNTIME_ROOT/bin/cortex"
    LOG_DIR="$SERVICE_HOME/.freshloop/logs"
fi

echo "=========================================="
echo "  Cortex Local Service Installer"
echo "=========================================="

# 1. Verify config.toml exists
echo ""
echo ">>> Checking configuration..."
if [ ! -f "$CONFIG_SOURCE" ]; then
    echo "ERROR: config.toml not found at $CONFIG_SOURCE"
    exit 1
fi

# Verify critical config values
NEXUS_KEY_IN_CONFIG=$(grep -A2 "^\[nexus\]" "$CONFIG_SOURCE" | grep "auth_key" | cut -d'"' -f2)
NEXUS_URL_IN_CONFIG=$(grep -A2 "^\[nexus\]" "$CONFIG_SOURCE" | grep "api_url" | cut -d'"' -f2)

if [ -z "$NEXUS_KEY_IN_CONFIG" ]; then
    echo "ERROR: [nexus].auth_key not found in config.toml"
    exit 1
fi

echo "  Nexus URL: $NEXUS_URL_IN_CONFIG"
echo "  Auth Key: ${NEXUS_KEY_IN_CONFIG:0:8}****"
echo "  Cargo Target: $CARGO_TARGET_DIR"
echo "  Install Mode: $RUNTIME_MODE"
echo "  Service Home: $SERVICE_HOME"

# Show key config info
FEED_COUNT=$(grep -c "https://" "$CONFIG_SOURCE" 2>/dev/null || echo "0")
HOST_COUNT=$(grep -c "^\[\[hosts\]\]" "$CONFIG_SOURCE" 2>/dev/null || echo "0")
echo "  RSS feeds: ~$FEED_COUNT URLs"
echo "  Hosts: $HOST_COUNT"

# 2. Build Cortex
echo ""
echo ">>> Building Cortex (Release with Metal support)..."
cd "$WORK_DIR/backend"
# Force recompile of external path dependency (qwen3-tts-rs is outside workspace)
# Cargo's incremental compilation may not detect changes in external path deps
cargo clean -p qwen3-tts 2>/dev/null || true
cargo build -p cortex --release --features metal
cd "$WORK_DIR"

if [ ! -f "$BINARY_SOURCE" ]; then
    echo "Error: Cortex binary not found at $BINARY_SOURCE"
    exit 1
fi

# 3. Setup executable
echo ""
echo ">>> Setting up executable..."
mkdir -p "$(dirname "$BINARY_DEST")"
mkdir -p "$(dirname "$PLIST_PATH")"
mkdir -p "$LOG_DIR"
mkdir -p "$SERVICE_HOME"
cp "$BINARY_SOURCE" "$BINARY_DEST"

# Fix macOS quarantine/signing issues
echo ">>> Fixing permissions..."
xattr -d com.apple.quarantine "$BINARY_DEST" 2>/dev/null || true
codesign --force --sign - "$BINARY_DEST"

# 4. Generate LaunchAgent Plist
echo ""
echo ">>> Generating LaunchAgent Plist..."
cat <<EOF > "$PLIST_PATH"
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>$APP_NAME</string>
    <key>ProgramArguments</key>
    <array>
        <string>$BINARY_DEST</string>
    </array>
    <key>WorkingDirectory</key>
    <string>$WORK_DIR</string>
    <key>EnvironmentVariables</key>
    <dict>
        <key>PATH</key>
        <string>/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin</string>
        <key>RUST_LOG</key>
        <string>info</string>
        <key>HOME</key>
        <string>$SERVICE_HOME</string>
    </dict>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>/dev/null</string>
    <key>StandardErrorPath</key>
    <string>/dev/null</string>
    <key>ProcessType</key>
    <string>Background</string>
</dict>
</plist>
EOF

# 5. Register Service
echo ""
echo ">>> Registering Service..."
if launchctl print "$LAUNCH_DOMAIN/$APP_NAME" >/dev/null 2>&1; then
    echo ">>> Stopping existing service..."
    CURRENT_PLIST_PATH=$(launchctl print "$LAUNCH_DOMAIN/$APP_NAME" 2>/dev/null | awk -F' = ' '/^[[:space:]]*path = / { print $2; exit }')
    launchctl bootout "$LAUNCH_DOMAIN/$APP_NAME" 2>/dev/null || true
    if launchctl print "$LAUNCH_DOMAIN/$APP_NAME" >/dev/null 2>&1; then
        launchctl unload "$CURRENT_PLIST_PATH" 2>/dev/null || true
    fi
fi

if ! launchctl bootstrap "$LAUNCH_DOMAIN" "$PLIST_PATH" 2>/dev/null; then
    launchctl load "$PLIST_PATH"
fi

launchctl enable "$LAUNCH_DOMAIN/$APP_NAME" 2>/dev/null || true
launchctl kickstart -k "$LAUNCH_DOMAIN/$APP_NAME" 2>/dev/null || true

# Wait a moment for service to start
sleep 1

JOB_INFO=$(launchctl print "$LAUNCH_DOMAIN/$APP_NAME" 2>/dev/null || true)
ACTIVE_PROGRAM=$(echo "$JOB_INFO" | awk -F' = ' '/^[[:space:]]*program = / { print $2; exit }')
ACTIVE_HOME=$(echo "$JOB_INFO" | awk -F'=> ' '/^[[:space:]]*HOME => / { print $2; exit }')

if [ "$ACTIVE_PROGRAM" != "$BINARY_DEST" ] || [ "$ACTIVE_HOME" != "$SERVICE_HOME" ]; then
    echo ""
    echo "ERROR: Cortex service did not switch to the expected runtime."
    echo "  Expected Program: $BINARY_DEST"
    echo "  Active Program:   ${ACTIVE_PROGRAM:-<unavailable>}"
    echo "  Expected HOME:    $SERVICE_HOME"
    echo "  Active HOME:      ${ACTIVE_HOME:-<unavailable>}"
    echo ""
    echo "This environment can build the new binary, but it cannot replace the existing user LaunchAgent."
    echo "Run this script in a normal local Terminal session to complete the final switchover."
    exit 1
fi

echo ""
echo "=========================================="
echo "  Service Installed Successfully!"
echo "=========================================="
echo ""
echo "Status:"
launchctl print "$LAUNCH_DOMAIN/$APP_NAME" 2>/dev/null | sed -n '1,20p' || echo "  (starting...)"
echo ""
TODAY=$(date +%Y-%m-%d)
echo "Logs (daily rotating, 30-day retention):"
echo "  Today: $LOG_DIR/cortex-$TODAY.log"
echo "  All:   ls $LOG_DIR/cortex-*.log"
echo ""
echo "Useful commands:"
echo "  View logs:     tail -f $LOG_DIR/cortex-$TODAY.log"
echo "  Stop service:  launchctl unload $PLIST_PATH"
echo "  Start service: launchctl load $PLIST_PATH"
echo "  Restart:       launchctl unload $PLIST_PATH && launchctl load $PLIST_PATH"
