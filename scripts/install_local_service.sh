#!/bin/bash
set -e

APP_NAME="com.freshloop.cortex"
PLIST_PATH="$HOME/Library/LaunchAgents/$APP_NAME.plist"
WORK_DIR="$(cd "$(dirname "$0")/.." && pwd)"
BINARY_SOURCE="$WORK_DIR/backend/target/release/cortex"
BINARY_DEST="$HOME/.freshloop/bin/cortex"
LOG_DIR="$HOME/.freshloop/logs"
CONFIG_SOURCE="$WORK_DIR/config.toml"

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

# Show key config info
FEED_COUNT=$(grep -c "https://" "$CONFIG_SOURCE" 2>/dev/null || echo "0")
HOST_COUNT=$(grep -c "^\[\[hosts\]\]" "$CONFIG_SOURCE" 2>/dev/null || echo "0")
echo "  RSS feeds: ~$FEED_COUNT URLs"
echo "  Hosts: $HOST_COUNT"

# 2. Build Cortex
echo ""
echo ">>> Building Cortex (Release with Metal support)..."
cd "$WORK_DIR/backend"
cargo build -p cortex --release --features metal
cd "$WORK_DIR"

if [ ! -f "$BINARY_SOURCE" ]; then
    echo "Error: Cortex binary not found at $BINARY_SOURCE"
    exit 1
fi

# 3. Setup executable
echo ""
echo ">>> Setting up executable..."
mkdir -p "$HOME/.freshloop/bin"
cp "$BINARY_SOURCE" "$BINARY_DEST"

# Fix macOS quarantine/signing issues
echo ">>> Fixing permissions..."
xattr -d com.apple.quarantine "$BINARY_DEST" 2>/dev/null || true
codesign --force --sign - "$BINARY_DEST"
mkdir -p "$LOG_DIR"

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
        <string>$HOME</string>
    </dict>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>$LOG_DIR/cortex.out.log</string>
    <key>StandardErrorPath</key>
    <string>$LOG_DIR/cortex.err.log</string>
    <key>ProcessType</key>
    <string>Background</string>
</dict>
</plist>
EOF

# 5. Register Service
echo ""
echo ">>> Registering Service..."
# Unload if exists
launchctl unload "$PLIST_PATH" 2>/dev/null || true
# Load new definition
launchctl load "$PLIST_PATH"

# Wait a moment for service to start
sleep 1

echo ""
echo "=========================================="
echo "  Service Installed Successfully!"
echo "=========================================="
echo ""
echo "Status:"
launchctl list | grep "$APP_NAME" || echo "  (starting...)"
echo ""
echo "Logs:"
echo "  stdout: $LOG_DIR/cortex.out.log"
echo "  stderr: $LOG_DIR/cortex.err.log"
echo ""
echo "Useful commands:"
echo "  View logs:     tail -f $LOG_DIR/cortex.err.log"
echo "  Stop service:  launchctl unload $PLIST_PATH"
echo "  Start service: launchctl load $PLIST_PATH"
echo "  Restart:       launchctl unload $PLIST_PATH && launchctl load $PLIST_PATH"
