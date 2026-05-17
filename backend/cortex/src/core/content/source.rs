use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProductLine {
    Radio,
    CuratedFeed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContentSource {
    pub id: String,
    pub name: String,
    pub url: String,
    pub product_line: ProductLine,
    pub source_group: Option<String>,
    pub tags: Vec<String>,
}

impl ContentSource {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        url: impl Into<String>,
        product_line: ProductLine,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            url: url.into(),
            product_line,
            source_group: None,
            tags: Vec::new(),
        }
    }
}
