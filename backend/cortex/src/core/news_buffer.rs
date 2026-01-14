use anyhow::Result;
use serde::{Deserialize, Serialize};
use sled::Db;

use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingNewsItem {
    pub title: String,
    pub link: String,
    pub description: String,
    pub category: String,
    pub source_name: Option<String>,
    pub timestamp: u64,
}

/// A cluster of related news items (same topic from different sources)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterData {
    pub id: String,
    pub main_item: PendingNewsItem,
    pub related_items: Vec<PendingNewsItem>,
    pub simhash: u64,
    pub merged_summary: Option<String>, // LLM-merged summary (if available)
    pub created_at: u64,
}

impl ClusterData {
    pub fn new(item: PendingNewsItem) -> Self {
        let simhash = Self::calculate_simhash(&item.title, &item.description);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            main_item: item,
            related_items: Vec::new(),
            simhash,
            merged_summary: None,
            created_at: now,
        }
    }

    /// Calculate SimHash for quick similarity comparison
    pub fn calculate_simhash(title: &str, description: &str) -> u64 {
        let text = format!("{} {}", title, description);
        crate::core::utils::calculate_simhash(&text)
    }

    /// Hamming distance between two SimHashes
    pub fn hamming_distance(a: u64, b: u64) -> u32 {
        crate::core::utils::hamming_distance(a, b)
    }

    /// Add a related item to this cluster
    pub fn add_related(&mut self, item: PendingNewsItem) {
        self.related_items.push(item);
    }

    /// Update with merged summary from LLM
    pub fn set_merged_summary(&mut self, title: String, summary: String) {
        self.main_item.title = title;
        self.merged_summary = Some(summary);
    }
}

pub struct NewsBuffer {
    db: Db,
}

impl NewsBuffer {
    pub fn new(cache_dir: &str) -> Result<Self> {
        // Use v3 to avoid conflict with old data format
        let db = sled::open(Path::new(cache_dir).join("news_buffer_v3"))?;
        Ok(Self { db })
    }

    /// Key format: "category#cluster_id"
    fn make_key(category: &str, cluster_id: &str) -> String {
        let safe_cat = category.replace("#", "_");
        format!("{}#{}", safe_cat, cluster_id)
    }

    /// Store a cluster
    pub fn store_cluster(&self, cluster: &ClusterData) -> Result<()> {
        let key = Self::make_key(&cluster.main_item.category, &cluster.id);
        let val = serde_json::to_vec(cluster)?;
        self.db.insert(key.as_bytes(), val)?;
        self.db.flush()?;
        Ok(())
    }

    /// Get all clusters for a category (for similarity checking)
    pub fn get_category_clusters(&self, category: &str) -> Result<Vec<ClusterData>> {
        let mut clusters = Vec::new();
        let prefix = format!("{}#", category.replace("#", "_"));

        for item in self.db.scan_prefix(prefix.as_bytes()) {
            let (_, val) = item?;
            let cluster: ClusterData = serde_json::from_slice(&val)?;
            clusters.push(cluster);
        }
        Ok(clusters)
    }

    /// Find clusters with similar SimHash (Hamming distance < threshold)
    pub fn find_similar_clusters(
        &self,
        category: &str,
        simhash: u64,
        threshold: u32,
    ) -> Result<Vec<ClusterData>> {
        let clusters = self.get_category_clusters(category)?;
        Ok(clusters
            .into_iter()
            .filter(|c| ClusterData::hamming_distance(c.simhash, simhash) < threshold)
            .collect())
    }

    /// Get cluster statistics per category: (cluster_count, oldest_timestamp)
    pub fn get_category_stats(&self) -> Result<std::collections::HashMap<String, (usize, u64)>> {
        let mut stats: std::collections::HashMap<String, (usize, u64)> =
            std::collections::HashMap::new();

        for item in self.db.iter() {
            let (_, val) = item?;
            if let Ok(cluster) = serde_json::from_slice::<ClusterData>(&val) {
                let category = cluster.main_item.category.clone();
                let entry = stats.entry(category).or_insert((0, u64::MAX));
                entry.0 += 1; // Count
                if cluster.created_at < entry.1 {
                    entry.1 = cluster.created_at; // Oldest
                }
            }
        }
        Ok(stats)
    }

    /// Pop all clusters for a category (for generation)
    pub fn pop_category_clusters(&self, category: &str) -> Result<Vec<ClusterData>> {
        let mut clusters = Vec::new();
        let prefix = format!("{}#", category.replace("#", "_"));

        for item in self.db.scan_prefix(prefix.as_bytes()) {
            let (key, val) = item?;
            let cluster: ClusterData = serde_json::from_slice(&val)?;
            clusters.push(cluster);
            self.db.remove(key)?;
        }

        self.db.flush()?;
        Ok(clusters)
    }

    /// Remove specific clusters by ID (Ack mechanism)
    pub fn remove_clusters(&self, category: &str, cluster_ids: &[String]) -> Result<()> {
        let safe_cat = category.replace("#", "_");
        for id in cluster_ids {
            let key = format!("{}#{}", safe_cat, id);
            self.db.remove(key)?;
        }
        self.db.flush()?;
        Ok(())
    }

    // ===== Link Tracking for Source Deduplication =====

    /// Check if a link has been processed recently (persisted in DB)
    pub fn has_processed_link(&self, link: &str) -> Result<bool> {
        let tree = self.db.open_tree("processed_links")?;
        Ok(tree.contains_key(link)?)
    }

    /// Mark a link as processed with current timestamp
    pub fn mark_link_processed(&self, link: &str) -> Result<()> {
        let tree = self.db.open_tree("processed_links")?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs();
        tree.insert(link, &now.to_be_bytes())?;
        Ok(())
    }

    /// Prune links older than retention period (e.g., 3 days)
    pub fn prune_old_links(&self, retention_secs: u64) -> Result<usize> {
        let tree = self.db.open_tree("processed_links")?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs();

        let mut count = 0;
        for item in tree.iter() {
            let (key, val) = item?;
            // parse u64
            if val.len() == 8 {
                let ts = u64::from_be_bytes(val.as_ref().try_into().unwrap());
                if now > ts + retention_secs {
                    tree.remove(key)?;
                    count += 1;
                }
            }
        }
        tree.flush()?;
        Ok(count)
    }
}
