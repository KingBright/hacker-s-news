#!/bin/bash
AUDIO_DIR="/volume1/docker/nexus/audio"
DB_PATH="/volume1/docker/nexus/data/nexus.db"

echo "Starting Audio Migration (WAV -> MP3)..."
cd "$AUDIO_DIR" || { echo "Audio dir not found"; exit 1; }

# Loop through all WAV files
for wav_file in *.wav; do
    [ -e "$wav_file" ] || continue
    
    mp3_file="${wav_file%.wav}.mp3"
    
    echo "Processing: $wav_file"
    
    # 1. Convert to MP3 if not exists
    if [ ! -f "$mp3_file" ]; then
        echo "  -> Converting to MP3..."
        ffmpeg -i "$wav_file" -b:a 128k "$mp3_file" -y -v error
        if [ $? -ne 0 ]; then
            echo "  [ERROR] Conversion failed for $wav_file"
            continue
        fi
    else
        echo "  -> MP3 already exists."
    fi
    
    # 2. Fix Permissions
    chmod 644 "$mp3_file"
    
    # 3. Update SQLite Database
    echo "  -> Updating Database..."
    # Match precise URL path
    sqlite3 "$DB_PATH" "UPDATE items SET audio_url = '/audio/$mp3_file' WHERE audio_url LIKE '%/$wav_file';"
    
    echo "  [DONE]"
done

echo "Migration Complete."
