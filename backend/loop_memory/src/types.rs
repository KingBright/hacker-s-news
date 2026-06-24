use serde::{Deserialize, Serialize};
use std::collections::HashMap;

fn default_confidence() -> f32 {
    1.0
}

fn default_version() -> u32 {
    1
}

fn default_true() -> bool {
    true
}

fn default_provenance() -> Provenance {
    Provenance::LlmExtracted
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Provenance {
    UserExplicit,
    LlmExtracted,
    SystemInferred,
    ExternalSync,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum MemoryType {
    Episodic,
    Semantic,
    Procedural,
    #[serde(alias = "UserProfile")]
    UserProfileStatic,
    UserProfileDynamic,
    InteractionEvent,
    UserExpression,
    PreferenceSignal,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum MemoryRelation {
    Updates,
    Extends,
    Derives,
    References,
    Supports,
    Opposes,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum EdgeDirection {
    Incoming,
    Outgoing,
    Both,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MemoryQuery {
    VectorSearch {
        query: Vec<f32>,
        top_k: usize,
        namespace: Option<String>,
    },
    SemanticSearch {
        query: String,
        top_k: usize,
        namespace: Option<String>,
    },
    EntityLookup {
        entity: String,
        namespace: Option<String>,
    },
    TimeRange {
        start: u64,
        end: u64,
        namespace: Option<String>,
    },
    VectorSearchWithHistory {
        query: Vec<f32>,
        top_k: usize,
        namespace: Option<String>,
    },
    RelatedTo {
        target_id: String,
        relation: Option<MemoryRelation>,
        direction: EdgeDirection,
        namespace: Option<String>,
    },
    TemporalSnapshot {
        as_of: u64,
        namespace: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: String,
    pub memory_type: MemoryType,
    pub content: String,
    pub base_strength: f32,
    pub current_strength: f32,
    pub created_at: u64,
    pub last_accessed: u64,
    pub embedding: Option<Vec<f32>>,
    #[serde(default)]
    pub access_count: u32,
    #[serde(default = "default_confidence")]
    pub confidence: f32,
    #[serde(default)]
    pub similarity_score: Option<f32>,
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default = "default_true")]
    pub is_latest: bool,
    #[serde(default)]
    pub parent_memory_id: Option<String>,
    #[serde(default)]
    pub root_memory_id: Option<String>,
    #[serde(default)]
    pub memory_relations: HashMap<String, MemoryRelation>,
    #[serde(default)]
    pub forget_after: Option<u64>,
    #[serde(default)]
    pub is_forgotten: bool,
    #[serde(default)]
    pub forget_reason: Option<String>,
    #[serde(default)]
    pub is_static: bool,
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub valid_from: Option<u64>,
    #[serde(default)]
    pub valid_to: Option<u64>,
    #[serde(default = "default_provenance")]
    pub provenance: Provenance,
    #[serde(default)]
    pub source_session_id: Option<String>,
    #[serde(default)]
    pub source_content_hash: Option<String>,
    #[serde(default)]
    pub source_ref: Option<String>,
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

impl MemoryEntry {
    pub fn new(
        id: String,
        memory_type: MemoryType,
        content: String,
        created_at: u64,
        embedding: Option<Vec<f32>>,
    ) -> Self {
        Self {
            id,
            memory_type,
            content,
            base_strength: 1.0,
            current_strength: 1.0,
            created_at,
            last_accessed: created_at,
            embedding,
            access_count: 0,
            confidence: 1.0,
            similarity_score: None,
            version: 1,
            is_latest: true,
            parent_memory_id: None,
            root_memory_id: None,
            memory_relations: HashMap::new(),
            forget_after: None,
            is_forgotten: false,
            forget_reason: None,
            is_static: false,
            namespace: None,
            valid_from: None,
            valid_to: None,
            provenance: Provenance::LlmExtracted,
            source_session_id: None,
            source_content_hash: None,
            source_ref: None,
            metadata: HashMap::new(),
        }
    }

    pub fn access(&mut self, timestamp: u64) {
        self.last_accessed = timestamp;
        self.access_count += 1;
        let boost = 0.5 / (1.0 + self.access_count as f32 * 0.1);
        self.base_strength = (self.base_strength + boost).min(5.0);
        self.current_strength = self.base_strength;
    }

    pub fn is_retrievable(&self) -> bool {
        !self.is_forgotten && self.is_latest && self.confidence >= 0.3
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct UserProfile {
    pub static_facts: Vec<String>,
    pub dynamic_context: Vec<String>,
    pub preference_signals: Vec<String>,
}
