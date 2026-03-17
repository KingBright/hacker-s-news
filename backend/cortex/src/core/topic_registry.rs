use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sled::Db;

use std::path::Path;

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

    /// Check if topic exists and return the previous record if found
    /// Check if topic exists and return the previous record if found (with distance)
    /// Returns: Option<(candidate_record, hamming_distance)>
    pub fn is_duplicate(&self, text: &str, threshold: u32) -> Result<Option<(TopicRecord, u32)>> {
        let hash = crate::core::utils::calculate_simhash(text);

        let mut best_candidate: Option<(TopicRecord, u32)> = None;
        let mut min_dist = u32::MAX;

        for item in self.db.iter() {
            let (key, val) = item?;
            if key.len() == 8 {
                let stored_hash = u64::from_be_bytes(key[..8].try_into()?);
                let distance = crate::core::utils::hamming_distance(hash, stored_hash);

                if distance < threshold {
                    if distance < min_dist {
                        // Found a better match
                        let record = if let Ok(r) = serde_json::from_slice::<TopicRecord>(&val) {
                            r
                        } else {
                            // Legacy fallback
                            let ts = String::from_utf8(val.to_vec())?;
                            TopicRecord {
                                title: String::new(),
                                summary: String::new(),
                                timestamp: ts,
                            }
                        };

                        best_candidate = Some((record, distance));
                        min_dist = distance;
                    }
                }
            }
        }

        Ok(best_candidate)
    }

    /// Record a topic with full information
    pub fn record_topic(&self, text: &str) -> Result<()> {
        self.record_topic_with_details(text, "", "")
    }

    /// Record a topic with title and summary for better comparison later
    pub fn record_topic_with_details(&self, text: &str, title: &str, summary: &str) -> Result<()> {
        let hash = crate::core::utils::calculate_simhash(text);
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

    /// Get topic count and estimated memory usage
    pub fn get_stats(&self) -> Result<(usize, u64)> {
        let mut count = 0;
        let mut total_size: u64 = 0;
        for item in self.db.iter() {
            let (key, val) = item?;
            count += 1;
            total_size += key.len() as u64 + val.len() as u64;
        }
        Ok((count, total_size))
    }
}
