#!/bin/bash
set -e

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
cp -r frontend/.next dist/frontend/
cp frontend/package.json dist/frontend/
cp frontend/package-lock.json dist/frontend/
cp -r frontend/public dist/frontend/

echo "Build complete. Artifacts are in the dist/ directory."
