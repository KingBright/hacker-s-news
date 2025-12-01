#!/bin/bash
set -m # Enable Job Control

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

# 3. Start the backend services
echo "Starting backend services..."
./dist/nexus &
./dist/cortex &

# 4. Start the frontend
echo "Starting the frontend..."
cd dist/frontend
npm run start
cd ../..

echo "All services have been shut down."
