#!/bin/bash

SERVER="root@hackerlife.fun"
PORT="222"
# Paths from deploy.sh
REMOTE_DB="/volume1/docker/nexus/data/nexus.db"
REMOTE_AUDIO="/volume1/docker/nexus/audio"

echo "WARNING: This will clean content on the REMOTE SERVER ($SERVER)."
echo "Items and Audio will be deleted. Users preserved."
read -p "Are you sure? (y/N) " confirm

if [[ "$confirm" != "y" && "$confirm" != "Y" ]]; then
    exit 1
fi

ssh -p $PORT $SERVER "bash -s" <<EOF
    echo "Stopping Nexus..."
    systemctl stop nexus
    
    echo "Cleaning DB ($REMOTE_DB)..."
    if [ -f "$REMOTE_DB" ]; then
        sqlite3 "$REMOTE_DB" "DELETE FROM items;"
        sqlite3 "$REMOTE_DB" "DELETE FROM source_items;"
        sqlite3 "$REMOTE_DB" "DELETE FROM item_sources;"
        rm -f "$REMOTE_DB-wal" "$REMOTE_DB-shm"
        sqlite3 "$REMOTE_DB" "VACUUM;"
        echo "DB Cleaned."
    else
        echo "DB not found!"
    fi
    
    echo "Cleaning Audio..."
    rm -rf "$REMOTE_AUDIO"/*.wav
    rm -rf "$REMOTE_AUDIO"/*.mp3
    rm -rf "$REMOTE_AUDIO"/*.mp4
    
    echo "Restarting Nexus..."
    systemctl start nexus
    echo "Remote Reset Complete."
EOF
