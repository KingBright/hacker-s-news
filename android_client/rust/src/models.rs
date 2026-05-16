use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    pub title: String,
    pub summary: Option<String>,
    pub original_url: Option<String>,
    pub cover_image_url: Option<String>,
    pub audio_url: Option<String>,
    pub publish_time: Option<i64>,
    pub created_at: Option<i64>,
    pub rating: Option<i32>,
    pub tags: Option<String>,
    pub is_deleted: Option<bool>,
    pub duration_sec: Option<i64>,
    pub status: Option<String>,
    pub category: Option<String>,
}
