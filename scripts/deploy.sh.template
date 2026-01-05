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

# Argument Parsing
DEPLOY_FRONTEND=false
DEPLOY_NEXUS=false

if [ $# -eq 0 ]; then
    DEPLOY_FRONTEND=true
    DEPLOY_NEXUS=true
else
    for arg in "$@"; do
        case $arg in
            --frontend)
            DEPLOY_FRONTEND=true
            shift
            ;;
            --nexus)
            DEPLOY_NEXUS=true
            shift
            ;;
            *)
            # unknown option
            ;;
        esac
    done
fi

echo "Using server: $SERVER (Port: $SSH_PORT)"
echo "Deploying to: $APP_DIR"

if [ "$DEPLOY_FRONTEND" = true ]; then
    echo ">>> Target: Frontend"
fi
if [ "$DEPLOY_NEXUS" = true ]; then
    echo ">>> Target: Nexus Service (Binary)"
fi

# 1. Build Frontend
if [ "$DEPLOY_FRONTEND" = true ]; then
    echo ">>> Building Frontend..."
    cd frontend
    npm install
    npm run build
    cd ..
fi

# 2. Build Backend (Native Cross-compile for Linux x86_64)
if [ "$DEPLOY_NEXUS" = true ]; then
    echo ">>> Building Nexus Binary (Linux x86_64)..."
    # Ensure target is installed
    if ! rustup target list | grep -q "x86_64-unknown-linux-musl (installed)"; then
        echo ">>> Installing x86_64-unknown-linux-musl target..."
        rustup target add x86_64-unknown-linux-musl
    fi

    cd backend
    cargo build -p nexus --target x86_64-unknown-linux-musl --release
    cd ..
fi

# 3. Prepare Remote Directories
echo ">>> Preparing remote directories..."
ssh -p $SSH_PORT $SERVER "mkdir -p $APP_DIR $DATA_DIR $AUDIO_DIR"

# 4. Upload Binaries and Frontend
if [ "$DEPLOY_NEXUS" = true ]; then
    echo ">>> Stopping remote service..."
    ssh -p $SSH_PORT $SERVER "systemctl stop nexus || true"
fi

if [ "$DEPLOY_FRONTEND" = true ]; then
    echo ">>> Uploading Frontend..."
    ssh -p $SSH_PORT $SERVER "rm -rf $APP_DIR/frontend"
    scp -O -P $SSH_PORT -r frontend/out $SERVER:$APP_DIR/frontend
fi

if [ "$DEPLOY_NEXUS" = true ]; then
    echo ">>> Uploading Nexus Binary..."
    scp -O -P $SSH_PORT backend/target/x86_64-unknown-linux-musl/release/nexus $SERVER:$APP_DIR/
fi

# 5. Generate and Upload Configuration (Only needed if Backend deployed or first time, but harmless to update)
if [ "$DEPLOY_NEXUS" = true ]; then
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
    rm nexus.service
    
    echo ">>> Restarting Service..."
    ssh -p $SSH_PORT $SERVER "systemctl daemon-reload && systemctl enable nexus && systemctl restart nexus"
fi

echo ">>> Deployment Complete!"
echo "Nexus is running at https://$DOMAIN"
