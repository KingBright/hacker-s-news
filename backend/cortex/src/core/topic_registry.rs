use anyhow::Result;
use sled::Db;
use std::path::Path;
use chrono::{DateTime, Utc, Duration};
use std::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;
use serde::{Serialize, Deserialize};

/// Stored topic information for better follow-up story detection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopicRecord {
    pub title: String,
    pub summary: String,
    pub timestamp: String,
}

pub struct TopicRegistry {
    db: Db,
    ttl: Duration,
}

impl TopicRegistry {
    pub fn new(cache_dir: &str) -> Result<Self> {
        // Use v2 to avoid conflict with old hash-only format
        let db = sled::open(Path::new(cache_dir).join("topic_history_v2"))?;
        let ttl = Duration::hours(72);
        Ok(Self { db, ttl })
    }

    /// Calculate SimHash (64-bit fingerprint)
    fn calculate_hash(text: &str) -> u64 {
        let mut counts = [0i32; 64];
        let chars: Vec<char> = text.chars().collect();
        
        if chars.is_empty() { return 0; }
        
        let mut tokens = Vec::new();
        for window in chars.windows(2) {
             tokens.push(window.iter().collect::<String>());
        }
        if tokens.is_empty() {
             tokens.push(text.to_string());
        }

        for token in tokens {
            let mut hasher = DefaultHasher::new();
            token.hash(&mut hasher);
            let hash = hasher.finish();
            
            for i in 0..64 {
                let bit = (hash >> i) & 1;
                if bit == 1 {
                    counts[i] += 1;
                } else {
                    counts[i] -= 1;
                }
            }
        }
        
        let mut fingerprint: u64 = 0;
        for i in 0..64 {
            if counts[i] > 0 {
                fingerprint |= 1 << i;
            }
        }
        
        fingerprint
    }

    /// Check if topic exists and return the previous record if found
    pub fn is_duplicate(&self, text: &str) -> Result<Option<TopicRecord>> {
        let hash = Self::calculate_hash(text);
        let text_len = text.chars().count();
        
        let distance_threshold = if text_len < 50 { 1 } else { 3 };
        
        for item in self.db.iter() {
            let (key, val) = item?;
            if key.len() == 8 {
                let stored_hash = u64::from_be_bytes(key[..8].try_into()?);
                let distance = (hash ^ stored_hash).count_ones();
                
                if distance < distance_threshold {
                    // Try to parse as TopicRecord (new format)
                    if let Ok(record) = serde_json::from_slice::<TopicRecord>(&val) {
                        return Ok(Some(record));
                    }
                    // Fallback for old format (just timestamp string)
                    let ts = String::from_utf8(val.to_vec())?;
                    return Ok(Some(TopicRecord {
                        title: String::new(),
                        summary: String::new(),
                        timestamp: ts,
                    }));
                }
            }
        }
        
        Ok(None)
    }
    
    /// Record a topic with full information
    pub fn record_topic(&self, text: &str) -> Result<()> {
        self.record_topic_with_details(text, "", "")
    }
    
    /// Record a topic with title and summary for better comparison later
    pub fn record_topic_with_details(&self, text: &str, title: &str, summary: &str) -> Result<()> {
        let hash = Self::calculate_hash(text);
        let record = TopicRecord {
            title: title.to_string(),
            summary: summary.to_string(),
            timestamp: Utc::now().to_rfc3339(),
        };
        let val = serde_json::to_vec(&record)?;
        self.db.insert(&hash.to_be_bytes(), val)?;
        self.db.flush()?;
        Ok(())
    }
    
    pub fn prune(&self) -> Result<usize> {
        let now = Utc::now();
        let mut count = 0;
        for item in self.db.iter() {
            let (key, val) = item?;
            
            // Try new format first
            if let Ok(record) = serde_json::from_slice::<TopicRecord>(&val) {
                if let Ok(ts) = DateTime::parse_from_rfc3339(&record.timestamp) {
                    if now.signed_duration_since(ts) > self.ttl {
                        self.db.remove(key)?;
                        count += 1;
                    }
                }
            } else {
                // Fallback: old format (just timestamp string)
                let ts_str = String::from_utf8(val.to_vec())?;
                if let Ok(ts) = DateTime::parse_from_rfc3339(&ts_str) {
                    if now.signed_duration_since(ts) > self.ttl {
                        self.db.remove(key)?;
                        count += 1;
                    }
                }
            }
        }
        Ok(count)
    }
}
