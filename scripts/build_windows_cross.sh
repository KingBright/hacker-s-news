#!/bin/bash
# Cross-compile Cortex for Windows from Mac/Linux
# This script builds a Windows executable from a non-Windows host

set -e

echo ">>> Cortex Windows Cross-Compilation Script <<"
echo ""

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Target architecture
TARGET="x86_64-pc-windows-msvc"
PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CORTEX_DIR="$PROJECT_ROOT/backend/cortex"
DIST_DIR="$PROJECT_ROOT/dist"
WINDOWS_DIST_DIR="$DIST_DIR/windows"

echo ">>> Checking prerequisites..."

# Check if Rust is installed
if ! command -v rustc &> /dev/null; then
    echo -e "${RED}Error: Rust is not installed${NC}"
    echo "Please install Rust from https://rustup.rs/"
    exit 1
fi

# Add Windows target
echo ">>> Adding Windows target..."
rustup target add "$TARGET" || {
    echo -e "${YELLOW}Target already installed or installation failed${NC}"
}

# Check if cargo-xwin is installed
if ! command -v cargo-xwin &> /dev/null; then
    echo ">>> Installing cargo-xwin for cross-compilation..."
    cargo install cargo-xwin
fi

echo ""
echo ">>> Building Cortex for Windows..."
echo "Target: $TARGET"
echo "Source: $CORTEX_DIR"
echo ""

cd "$CORTEX_DIR"

# Build without CUDA/Metal features (CPU only for cross-platform compatibility)
echo ">>> Compiling (this may take a few minutes)..."
cargo xwin build --release --target "$TARGET"

# Check if build succeeded
EXE_SOURCE="$CORTEX_DIR/../../target/$TARGET/release/cortex.exe"
if [ -f "$EXE_SOURCE" ]; then
    echo ""
    echo -e "${GREEN}>>> Build successful!${NC}"
    echo ""

    # Create dist directories
    echo ">>> Creating distribution directory..."
    mkdir -p "$WINDOWS_DIST_DIR"

    # Copy executable to dist
    EXE_DEST="$WINDOWS_DIST_DIR/cortex.exe"
    cp "$EXE_SOURCE" "$EXE_DEST"

    # Copy install script to dist
    INSTALL_SCRIPT_SOURCE="$PROJECT_ROOT/scripts/install_windows.ps1"
    INSTALL_SCRIPT_DEST="$WINDOWS_DIST_DIR/install.ps1"

    if [ -f "$INSTALL_SCRIPT_SOURCE" ]; then
        cp "$INSTALL_SCRIPT_SOURCE" "$INSTALL_SCRIPT_DEST"
        echo ">>> Copied install script to dist directory"
    fi

    # Create default config if not exists
    CONFIG_DEST="$WINDOWS_DIST_DIR/config.toml"
    if [ ! -f "$CONFIG_DEST" ]; then
        echo ">>> Creating default config..."
        cat > "$CONFIG_DEST" << 'EOF'
[nexus]
api_url = "http://localhost:8899"
auth_key = "CHANGE_ME_NEXUS_KEY"

[llm]
model = "llama3"
api_url = "http://localhost:11434"

[tts]
model_path = ".\zh_CN-huayan-medium.onnx"

[[sources]]
name = "Hacker News"
url = "https://news.ycombinator.com/rss"
interval_min = 60
tags = ["Tech", "Global"]
EOF
    fi

    echo ""
    echo -e "${GREEN}>>> Distribution package created!${NC}"
    echo ""
    echo "Output location:"
    echo "  $WINDOWS_DIST_DIR/"
    echo ""
    echo "Contents:"
    ls -lh "$WINDOWS_DIST_DIR/"
    echo ""
    echo "To deploy to Windows:"
    echo "  1. Copy the entire $WINDOWS_DIST_DIR/ folder to your Windows machine"
    echo "  2. Edit config.toml as needed"
    echo "  3. Open PowerShell as Administrator"
    echo "  4. Run: .\install.ps1 -Install"
    echo "  5. Run: .\install.ps1 -Start"
    echo ""
    echo "Or zip the folder for distribution:"
    echo "  cd $DIST_DIR && zip -r windows.zip windows/"
else
    echo -e "${RED}>>> Build failed - executable not found${NC}"
    exit 1
fi
