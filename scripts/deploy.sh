#!/bin/bash
set -e

# 1. Ensure the project is built
if [ ! -d "dist" ]; then
    echo "The 'dist' directory does not exist. Please run 'scripts/build.sh' first."
    exit 1
fi

# 2. Clean and create the deployment directory
echo "Cleaning up and creating the deployment directory..."
rm -rf deploy
mkdir -p deploy/server/data/audio
mkdir -p deploy/worker
mkdir -p deploy/frontend

# 3. Copy the backend binaries
echo "Copying backend binaries..."
cp dist/nexus deploy/server/nexus_app
cp dist/cortex deploy/worker/cortex_app

# 4. Copy the frontend build artifacts
echo "Copying frontend build artifacts..."
cp -r dist/frontend/* deploy/frontend/

# 5. Create an example config file
echo "Creating an example config file..."
cat > deploy/worker/config.toml.example << EOL
[nexus]
api_url = "http://127.0.0.1:8080"
auth_key = "my-secret-key-123"

[llm]
model = "llama3"
api_url = "http://localhost:11434"

[tts]
model_path = "./zh_CN-huayan-medium.onnx"

[[sources]]
name = "Hacker News"
url = "https://news.ycombinator.com/rss"
interval_min = 60
tags = ["Tech", "Global"]

[[sources]]
name = "少数派"
url = "https://sspai.com/feed"
interval_min = 120
tags = ["Life", "Digital"]
EOL

echo "Deployment package created in the deploy/ directory."
