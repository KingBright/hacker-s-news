#!/bin/bash
set -e

# Configuration
SERVER="root@hackerlife.fun"
SSH_PORT="222"
APP_DIR="/opt/nexus"
DATA_DIR="/volume1/docker/nexus/data"
AUDIO_DIR="/volume1/docker/nexus/audio"
BINARY_NAME="nexus"
DOMAIN="news.hackerlife.fun"

echo "Using server: $SERVER (Port: $SSH_PORT)"
echo "Deploying to: $APP_DIR"
echo "Data stored in: $DATA_DIR"

# 1. Build Frontend
echo ">>> Building Frontend..."
cd frontend
npm install
npm run build
cd ..

# 2. Build Backend (Native Cross-compile for Linux x86_64)
echo ">>> Building Nexus Backend (Linux x86_64)..."
# Ensure target is installed
if ! rustup target list | grep -q "x86_64-unknown-linux-musl (installed)"; then
    echo ">>> Installing x86_64-unknown-linux-musl target..."
    rustup target add x86_64-unknown-linux-musl
fi

cd backend
# Using native cargo build. Requires a linker if not pure Rust, but rustls helps. 
# We might need CC_x86_64_unknown_linux_musl if linking C code.
# Assuming basic musl support or user has linker.
cargo build -p nexus --target x86_64-unknown-linux-musl --release
cd ..

# 3. Prepare Remote Directories
echo ">>> preparing remote directories..."
ssh -p $SSH_PORT $SERVER "mkdir -p $APP_DIR $DATA_DIR $AUDIO_DIR"

# 4. Upload Binaries and Frontend
echo ">>> Stopping remote service to allow overwrite..."
ssh -p $SSH_PORT $SERVER "systemctl stop nexus || true"
ssh -p $SSH_PORT $SERVER "rm -rf $APP_DIR/frontend"

echo ">>> Uploading files..."
scp -O -P $SSH_PORT backend/target/x86_64-unknown-linux-musl/release/nexus $SERVER:$APP_DIR/
scp -O -P $SSH_PORT -r frontend/out $SERVER:$APP_DIR/frontend

# 5. Generate and Upload Configuration
echo ">>> Generating Configuration File..."
cat <<EOF > config.env
PORT=8899
NEXUS_KEY=sk-secure-hackerlife-2026
STATIC_DIR=$APP_DIR/frontend
AUDIO_DIR=$AUDIO_DIR
DATABASE_URL=sqlite:$DATA_DIR/nexus.db
RUST_LOG=info
EOF

scp -O -P $SSH_PORT config.env $SERVER:$APP_DIR/config.env
rm config.env

# 6. Upload Systemd Service
echo ">>> Configuring Systemd Service..."
cat <<EOF > nexus.service
[Unit]
Description=Nexus News Server
After=network.target

[Service]
Type=simple
User=root
WorkingDirectory=$APP_DIR
ExecStart=$APP_DIR/nexus
Restart=always
EnvironmentFile=$APP_DIR/config.env

[Install]
WantedBy=network.target
EOF

scp -O -P $SSH_PORT nexus.service $SERVER:/etc/systemd/system/nexus.service
ssh -p $SSH_PORT $SERVER "systemctl daemon-reload && systemctl enable nexus && systemctl restart nexus"

# Cleanup
rm nexus.service

echo ">>> Deployment Complete!"
echo "Nexus is running at https://$DOMAIN"
