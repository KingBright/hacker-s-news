#!/bin/bash
set -m # Enable Job Control

# Resolve the project root directory
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )"
PROJECT_ROOT="$SCRIPT_DIR/.."

# Change to the project root
cd "$PROJECT_ROOT"

# 1. Ensure the project is built
if [ ! -d "dist" ]; then
    echo "The 'dist' directory does not exist. Please run 'scripts/build.sh' first."
    exit 1
fi

# 2. Function to kill all background jobs on exit
cleanup() {
    echo "Shutting down services..."
    kill $(jobs -p)
}

trap cleanup EXIT

# Export LibTorch environment variables (Removed for Candle migration)
# export LIBTORCH="$(pwd)/libtorch_v2.4.0/libtorch"
# export DYLD_LIBRARY_PATH="$LIBTORCH/lib:$DYLD_LIBRARY_PATH"

# Export environment variables for network and libraries
export http_proxy="http://127.0.0.1:8228"
export https_proxy="http://127.0.0.1:8228"
export no_proxy="localhost,127.0.0.1"
export DYLD_LIBRARY_PATH="/opt/homebrew/Cellar/onnxruntime/1.22.2_6/lib:$DYLD_LIBRARY_PATH"

# 2. Start the backend services
echo "Starting backend services..."
RUST_LOG=info ./dist/nexus &
RUST_LOG=info ./dist/cortex &

# 4. Start the frontend
echo "Starting the frontend (served by Nexus on port 8899)..."
# Frontend is served by Nexus
cd ../..

echo "Services started. Press Ctrl+C to stop."
wait
