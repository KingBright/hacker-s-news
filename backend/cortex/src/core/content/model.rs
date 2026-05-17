use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FetchedEntry {
    pub title: String,
    pub link: String,
    pub description: String,
    pub pub_date: Option<String>,
    pub source_name: Option<String>,
}
