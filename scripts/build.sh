#!/bin/bash
set -e

# Resolve the project root directory
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )"
PROJECT_ROOT="$SCRIPT_DIR/.."

# Change to the project root
cd "$PROJECT_ROOT"

# 1. Clean and create the distribution directory
echo "Cleaning up and creating the distribution directory..."
rm -rf dist
mkdir -p dist/frontend

# 2. Build the backend
echo "Building the backend..."
cd backend
cargo build --release
cd ..

# 3. Copy the backend binaries
echo "Copying backend binaries..."
cp backend/target/release/nexus dist/
cp backend/target/release/cortex dist/

# 4. Build the frontend
echo "Building the frontend..."
cd frontend
npm install
npm run build
cd ..

# 5. Copy the frontend build artifacts
echo "Copying frontend build artifacts..."
# Copy static export
cp -r frontend/out/* dist/frontend/

echo "Build complete. Artifacts are in the dist/ directory."
