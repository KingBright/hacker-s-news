#!/bin/bash

# Ensure we are in the workspace root
cd "$(dirname "$0")/.."

# Define paths
DB_PATH="freshloop.db"
BACKEND_DB_PATH="backend/nexus/freshloop.db"
CACHE_DIR="$HOME/.freshloop/cache"
AUDIO_DIR="backend/nexus/audio"

# Check for arguments
DELETE_ALL=false
for arg in "$@"; do
    if [ "$arg" == "--all" ]; then
        DELETE_ALL=true
    fi
done

if [ "$DELETE_ALL" = true ]; then
    echo "WARNING: --all flag detected. This will delete ALL data including LLM CACHE (expensive to rebuild)."
else
    echo "Caution: This will delete all NEWS ITEMS and AUDIO CONTENT."
    echo "User accounts and play history will be preserved."
    echo "LLM Cache will be PRESERVED (use --all to wipe it)."
fi

read -p "Are you sure? (y/N) " confirm

if [[ "$confirm" != "y" && "$confirm" != "Y" ]]; then
    echo "Aborted."
    exit 1
fi

# 0. Stop Services (Critical for SQLite VACUUM/WAL)
echo "Stopping services..."
launchctl stop gui/$(id -u)/com.freshloop.nexus 2>/dev/null
launchctl stop gui/$(id -u)/com.freshloop.cortex 2>/dev/null
killall nexus 2>/dev/null
killall cortex 2>/dev/null
sleep 2

# Function to clean SQLite
clean_db() {
    local db=$1
    if [ -f "$db" ]; then
        echo "Cleaning content from $db..."
        # Check if tables exist before deleting
        sqlite3 "$db" "DELETE FROM items;" 2>/dev/null || echo "  - Table 'items' not found or empty."
        sqlite3 "$db" "DELETE FROM source_items;" 2>/dev/null || echo "  - Table 'source_items' not found or empty."
        sqlite3 "$db" "DELETE FROM item_sources;" 2>/dev/null || echo "  - Table 'item_sources' not found or empty."
        
        # Check WAL files
        rm -f "$db-shm" "$db-wal"
        echo "Done."
    else
        echo "Database $db not found, skipping."
    fi
}

# 1. Clean Databases
clean_db "$DB_PATH"
clean_db "$BACKEND_DB_PATH"

# 2. Clean Cortex Caches (Registry & Buffers)
echo "Cleaning Cortex caches..."
if [ "$DELETE_ALL" = true ]; then
    echo "  - Deleting ALL caches (including LLM cache)..."
    rm -rf "$CACHE_DIR"
    mkdir -p "$CACHE_DIR" # Recreate dir
else
    rm -rf "$CACHE_DIR/news_buffer"
    rm -rf "$CACHE_DIR/news_buffer_v2"
    rm -rf "$CACHE_DIR/news_buffer_v3"
    rm -rf "$CACHE_DIR/topic_registry"
    rm -rf "$CACHE_DIR/topic_history"
    rm -rf "$CACHE_DIR/topic_history_v2"
    # NOTE: We implicitly PRESERVE "$CACHE_DIR/llm_cache" to avoid re-running expensive LLM calls.
    echo "  - Preserved LLM Cache (to save tokens/time). Use --all to delete."
fi
# 3. Clean Generated Audio
# The actual audio directory config might be default "audio" relative to running binary
# Also checking common deployment path /opt/nexus/audio or ~/.freshloop/audio
POSSIBLE_AUDIO_DIRS=(
  "$AUDIO_DIR"
  "$HOME/.freshloop/audio"
  "/opt/nexus/audio"
)

for dir in "${POSSIBLE_AUDIO_DIRS[@]}"; do
    if [ -d "$dir" ]; then
        echo "Cleaning GENERATED audio files in $dir (keeping reference files)..."
        # Only delete files starting with a UUID pattern (8-4-4-4-12)
        find "$dir" -type f -name "[0-9a-fA-F]*-[0-9a-fA-F]*-[0-9a-fA-F]*-[0-9a-fA-F]*-[0-9a-fA-F]*_*.wav" -delete 2>/dev/null
        find "$dir" -type f -name "[0-9a-fA-F]*-[0-9a-fA-F]*-[0-9a-fA-F]*-[0-9a-fA-F]*-[0-9a-fA-F]*_*.mp3" -delete 2>/dev/null
    fi
done

echo "Content reset complete. Restarting Local Cortex..."
launchctl kickstart -k gui/$(id -u)/com.freshloop.cortex
