#!/bin/bash
set -e

APP_NAME="com.freshloop.cortex"
PLIST_PATH="$HOME/Library/LaunchAgents/$APP_NAME.plist"
WORK_DIR="$(pwd)"
BINARY_SOURCE="$WORK_DIR/backend/target/release/cortex"
BINARY_DEST="$HOME/.freshloop/bin/cortex"
LOG_DIR="$HOME/.freshloop/logs"

echo ">>> Building Cortex (Release)..."
cd backend
cargo build -p cortex --release --features metal
cd ..

if [ ! -f "$BINARY_SOURCE" ]; then
    echo "Error: Cortex binary not found at $BINARY_SOURCE"
    exit 1
fi

echo ">>> Setting up executable..."
mkdir -p "$HOME/.freshloop/bin"
cp "$BINARY_SOURCE" "$BINARY_DEST"
# Fix macOS quarantine/signing issues
echo ">>> Fixing permissions..."
xattr -d com.apple.quarantine "$BINARY_DEST" 2>/dev/null || true
codesign --force --sign - "$BINARY_DEST"
mkdir -p "$LOG_DIR"

echo ">>> Copying latest config..."
CONFIG_SOURCE="$WORK_DIR/config.toml"
if [ -f "$CONFIG_SOURCE" ]; then
    echo "Config file: $CONFIG_SOURCE"
    # Show key config info
    FEED_COUNT=$(grep -c "https://" "$CONFIG_SOURCE" 2>/dev/null || echo "0")
    HOST_COUNT=$(grep -c "^\[\[hosts\]\]" "$CONFIG_SOURCE" 2>/dev/null || echo "0")
    CAT_LINE=$(grep -m1 "categories = \[" "$CONFIG_SOURCE" | head -1)
    echo "  - RSS feeds: ~$FEED_COUNT URLs"
    echo "  - Hosts: $HOST_COUNT"
    echo "  - Categories: $(grep "categories = \[" "$CONFIG_SOURCE" -A1 | tail -1 | tr -d ' \"')"
else
    echo "ERROR: config.toml not found at $CONFIG_SOURCE"
    exit 1
fi

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
        <key>RUST_LOG</key>
        <string>info</string>
    </dict>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>$LOG_DIR/cortex.out.log</string>
    <key>StandardErrorPath</key>
    <string>$LOG_DIR/cortex.err.log</string>
</dict>
</plist>
EOF

echo ">>> Registering Service..."
# Unload if exists
launchctl unload "$PLIST_PATH" 2>/dev/null || true
# Load new definition
launchctl load "$PLIST_PATH"

echo ">>> Service Installed!"
echo "Status:"
launchctl list | grep "$APP_NAME"
echo "Logs are available at: $LOG_DIR"
