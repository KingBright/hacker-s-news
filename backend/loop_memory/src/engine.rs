use crate::conflict;
use crate::decay;
use crate::types::{EdgeDirection, MemoryEntry, MemoryQuery, MemoryRelation, MemoryType};
use async_trait::async_trait;
use redb::{Database, ReadableTable, TableDefinition};
use std::sync::Arc;

const MEMORY_TABLE: TableDefinition<&str, &str> = TableDefinition::new("memories");

#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, String>;
}

#[derive(Debug, Clone, Copy)]
pub struct ConflictResolution {
    pub new_confidence: f32,
    pub existing_confidence: f32,
}

#[async_trait]
pub trait ConflictResolver: Send + Sync {
    async fn resolve(&self, new_content: &str, existing_content: &str) -> ConflictResolution;
}

#[async_trait]
pub trait MemoryStore: Send + Sync {
    async fn store(&self, entry: MemoryEntry) -> Result<(), String>;
    async fn retrieve(&self, query: MemoryQuery) -> Result<Vec<MemoryEntry>, String>;
    async fn trigger_fade_consolidation(&self) -> Result<(), String>;
    async fn delete(&self, id: &str) -> Result<(), String>;
    async fn retrieve_all(&self) -> Result<Vec<MemoryEntry>, String>;

    async fn expand_relations(&self, entries: &[MemoryEntry]) -> Result<Vec<MemoryEntry>, String> {
        Ok(entries.to_vec())
    }
}

pub struct RedbMemoryStore {
    db: Arc<Database>,
    embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
    conflict_resolver: Option<Arc<dyn ConflictResolver>>,
}

impl RedbMemoryStore {
    pub fn new(path: impl AsRef<str>) -> Result<Self, String> {
        Self::new_with_options(path, None, None)
    }

    pub fn new_with_options(
        path: impl AsRef<str>,
        embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
        conflict_resolver: Option<Arc<dyn ConflictResolver>>,
    ) -> Result<Self, String> {
        let path = path.as_ref();
        if let Some(parent) = std::path::Path::new(path).parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }

        let db = Database::create(path).map_err(|e| e.to_string())?;
        let write_txn = db.begin_write().map_err(|e| e.to_string())?;
        {
            let _ = write_txn
                .open_table(MEMORY_TABLE)
                .map_err(|e| e.to_string())?;
        }
        write_txn.commit().map_err(|e| e.to_string())?;

        Ok(Self {
            db: Arc::new(db),
            embedding_provider,
            conflict_resolver,
        })
    }

    fn query_namespace(query: &MemoryQuery) -> Option<&str> {
        match query {
            MemoryQuery::VectorSearch { namespace, .. }
            | MemoryQuery::SemanticSearch { namespace, .. }
            | MemoryQuery::EntityLookup { namespace, .. }
            | MemoryQuery::TimeRange { namespace, .. }
            | MemoryQuery::VectorSearchWithHistory { namespace, .. }
            | MemoryQuery::RelatedTo { namespace, .. }
            | MemoryQuery::TemporalSnapshot { namespace, .. } => namespace.as_deref(),
        }
    }

    fn namespace_matches(entry: &MemoryEntry, namespace: Option<&str>) -> bool {
        namespace.map_or(true, |scope| entry.namespace.as_deref() == Some(scope))
    }

    async fn embed_text(&self, text: &str) -> Option<Vec<f32>> {
        let provider = self.embedding_provider.as_ref()?;
        match provider.embed(text).await {
            Ok(embedding) => Some(embedding),
            Err(e) => {
                tracing::warn!("[loop_memory] embedding failed: {}", e);
                None
            }
        }
    }

    fn needs_embedding(memory_type: &MemoryType) -> bool {
        matches!(
            memory_type,
            MemoryType::Semantic
                | MemoryType::UserProfileStatic
                | MemoryType::UserProfileDynamic
                | MemoryType::InteractionEvent
                | MemoryType::Procedural
                | MemoryType::UserExpression
                | MemoryType::PreferenceSignal
        )
    }

    async fn touch_entries(&self, entries: &[MemoryEntry]) {
        let now_ts = decay::get_current_timestamp();
        if let Ok(write_txn) = self.db.begin_write() {
            let update_result: Result<(), String> = (|| {
                let mut table = write_txn
                    .open_table(MEMORY_TABLE)
                    .map_err(|e| e.to_string())?;
                for entry in entries {
                    let mut updated = entry.clone();
                    updated.access(now_ts);
                    let serialized = serde_json::to_string(&updated).map_err(|e| e.to_string())?;
                    table
                        .insert(updated.id.as_str(), serialized.as_str())
                        .map_err(|e| e.to_string())?;
                }
                Ok(())
            })();
            if update_result.is_ok() {
                let _ = write_txn.commit();
            }
        }
    }
}

#[async_trait]
impl MemoryStore for RedbMemoryStore {
    async fn store(&self, mut entry: MemoryEntry) -> Result<(), String> {
        if Self::needs_embedding(&entry.memory_type) && entry.embedding.is_none() {
            entry.embedding = self.embed_text(&entry.content).await;
        }

        let conflict_types = matches!(
            entry.memory_type,
            MemoryType::Semantic
                | MemoryType::UserProfileStatic
                | MemoryType::UserProfileDynamic
                | MemoryType::PreferenceSignal
        );

        if conflict_types && entry.embedding.is_some() {
            let existing = self.retrieve_all().await.unwrap_or_default();
            let conflicts = conflict::detect_conflicts(&entry, &existing, 0.88, 3);
            for conflict in conflicts {
                let resolution = if let Some(resolver) = &self.conflict_resolver {
                    resolver
                        .resolve(&entry.content, &conflict.existing.content)
                        .await
                } else {
                    ConflictResolution {
                        new_confidence: 1.0,
                        existing_confidence: 0.7,
                    }
                };

                let now_ts = decay::get_current_timestamp();
                let mut updated_existing = conflict.existing.clone();
                updated_existing.confidence = resolution.existing_confidence.clamp(0.0, 1.0);

                if updated_existing.confidence < 0.4 {
                    updated_existing.is_latest = false;
                    updated_existing.valid_to = Some(now_ts);
                    entry.parent_memory_id = Some(conflict.existing.id.clone());
                    entry.root_memory_id = Some(
                        conflict
                            .existing
                            .root_memory_id
                            .clone()
                            .unwrap_or_else(|| conflict.existing.id.clone()),
                    );
                    entry.version = conflict.existing.version + 1;
                    if entry.namespace.is_none() {
                        entry.namespace = conflict.existing.namespace.clone();
                    }
                    entry.valid_from = Some(now_ts);
                    entry
                        .memory_relations
                        .insert(conflict.existing.id.clone(), MemoryRelation::Updates);
                } else if entry.confidence >= 0.4 && updated_existing.confidence >= 0.4 {
                    entry
                        .memory_relations
                        .insert(conflict.existing.id.clone(), MemoryRelation::Extends);
                    updated_existing
                        .memory_relations
                        .insert(entry.id.clone(), MemoryRelation::Extends);
                }

                entry.confidence = resolution.new_confidence.clamp(0.0, 1.0);

                let write_txn = self.db.begin_write().map_err(|e| e.to_string())?;
                {
                    let mut table = write_txn
                        .open_table(MEMORY_TABLE)
                        .map_err(|e| e.to_string())?;
                    let serialized =
                        serde_json::to_string(&updated_existing).map_err(|e| e.to_string())?;
                    table
                        .insert(updated_existing.id.as_str(), serialized.as_str())
                        .map_err(|e| e.to_string())?;
                }
                write_txn.commit().map_err(|e| e.to_string())?;
            }
        }

        let write_txn = self.db.begin_write().map_err(|e| e.to_string())?;
        {
            let mut table = write_txn
                .open_table(MEMORY_TABLE)
                .map_err(|e| e.to_string())?;
            let serialized = serde_json::to_string(&entry).map_err(|e| e.to_string())?;
            table
                .insert(entry.id.as_str(), serialized.as_str())
                .map_err(|e| e.to_string())?;
        }
        write_txn.commit().map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn retrieve(&self, query: MemoryQuery) -> Result<Vec<MemoryEntry>, String> {
        let include_history = matches!(query, MemoryQuery::VectorSearchWithHistory { .. });
        let query_namespace = Self::query_namespace(&query).map(str::to_string);

        let effective_query = match query {
            MemoryQuery::SemanticSearch {
                query,
                top_k,
                namespace,
            } => {
                if let Some(embedding) = self.embed_text(&query).await {
                    MemoryQuery::VectorSearch {
                        query: embedding,
                        top_k,
                        namespace,
                    }
                } else {
                    MemoryQuery::EntityLookup {
                        entity: query,
                        namespace,
                    }
                }
            }
            MemoryQuery::VectorSearchWithHistory {
                query,
                top_k,
                namespace,
            } => MemoryQuery::VectorSearch {
                query,
                top_k,
                namespace,
            },
            other => other,
        };

        let read_txn = self.db.begin_read().map_err(|e| e.to_string())?;
        let table = read_txn
            .open_table(MEMORY_TABLE)
            .map_err(|e| e.to_string())?;
        let now_ts = decay::get_current_timestamp();

        if let MemoryQuery::RelatedTo {
            target_id,
            relation,
            direction,
            namespace,
        } = &effective_query
        {
            if *direction == EdgeDirection::Outgoing {
                let mut fast_results = Vec::new();
                if let Some(value) = table.get(target_id.as_str()).map_err(|e| e.to_string())? {
                    let target_entry: MemoryEntry =
                        serde_json::from_str(value.value()).map_err(|e| e.to_string())?;
                    for (rel_id, rel_type) in target_entry.memory_relations {
                        if relation.as_ref().map_or(true, |r| r == &rel_type) {
                            if let Some(rel_value) =
                                table.get(rel_id.as_str()).map_err(|e| e.to_string())?
                            {
                                let rel_entry: MemoryEntry =
                                    serde_json::from_str(rel_value.value())
                                        .map_err(|e| e.to_string())?;
                                if rel_entry.is_retrievable()
                                    && Self::namespace_matches(&rel_entry, namespace.as_deref())
                                {
                                    fast_results.push(rel_entry);
                                }
                            }
                        }
                    }
                }
                if !fast_results.is_empty() {
                    self.touch_entries(&fast_results).await;
                }
                return Ok(fast_results);
            }
        }

        let mut results = Vec::new();
        let iter = table.iter().map_err(|e| e.to_string())?;
        for result in iter {
            let (_key, value) = result.map_err(|e| e.to_string())?;
            let mut entry: MemoryEntry =
                serde_json::from_str(value.value()).map_err(|e| e.to_string())?;

            if !Self::namespace_matches(&entry, query_namespace.as_deref()) {
                continue;
            }
            if !include_history && !entry.is_retrievable() {
                continue;
            }
            if let Some(forget_after) = entry.forget_after {
                if now_ts > forget_after && !include_history {
                    continue;
                }
            }

            let matches = match &effective_query {
                MemoryQuery::EntityLookup { entity, .. } => {
                    let haystack = entry.content.to_lowercase();
                    let needle = entity.to_lowercase();
                    matches!(
                        entry.memory_type,
                        MemoryType::Semantic
                            | MemoryType::UserProfileStatic
                            | MemoryType::UserProfileDynamic
                            | MemoryType::InteractionEvent
                            | MemoryType::Procedural
                            | MemoryType::UserExpression
                            | MemoryType::PreferenceSignal
                    ) && haystack.contains(&needle)
                }
                MemoryQuery::TimeRange { start, end, .. } => {
                    entry.created_at >= *start && entry.created_at <= *end
                }
                MemoryQuery::VectorSearch { query, .. } => {
                    if let Some(doc_vec) = &entry.embedding {
                        let sim = conflict::cosine_similarity(doc_vec, query);
                        if sim > 0.5 {
                            entry.similarity_score = Some(sim);
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                }
                MemoryQuery::RelatedTo {
                    target_id,
                    relation,
                    direction,
                    ..
                } => match direction {
                    EdgeDirection::Outgoing => false,
                    EdgeDirection::Incoming | EdgeDirection::Both => entry
                        .memory_relations
                        .get(target_id)
                        .is_some_and(|rel| relation.as_ref().map_or(true, |r| r == rel)),
                },
                MemoryQuery::TemporalSnapshot { as_of, .. } => {
                    let time_valid = entry.valid_from.map_or(true, |vf| vf <= *as_of)
                        && entry.valid_to.map_or(true, |vt| vt >= *as_of);
                    let type_valid = matches!(
                        entry.memory_type,
                        MemoryType::Semantic
                            | MemoryType::UserProfileStatic
                            | MemoryType::UserProfileDynamic
                            | MemoryType::Procedural
                            | MemoryType::PreferenceSignal
                    );
                    time_valid && type_valid
                }
                MemoryQuery::SemanticSearch { .. }
                | MemoryQuery::VectorSearchWithHistory { .. } => false,
            };

            if matches {
                results.push(entry);
            }
        }

        if let MemoryQuery::VectorSearch { top_k, .. } = effective_query {
            const ONE_DAY_SECS: f64 = 86_400.0;
            for entry in &mut results {
                let sim = entry.similarity_score.unwrap_or(0.0) as f64;
                let age_days = (now_ts.saturating_sub(entry.created_at) as f64) / ONE_DAY_SECS;
                let recency = 1.0 / (1.0 + age_days);
                entry.similarity_score = Some((sim * 0.7 + recency * 0.3) as f32);
            }
            results.sort_by(|a, b| {
                b.similarity_score
                    .unwrap_or(0.0)
                    .partial_cmp(&a.similarity_score.unwrap_or(0.0))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            results.truncate(top_k);
        }

        if !results.is_empty() {
            self.touch_entries(&results).await;
        }
        Ok(results)
    }

    async fn trigger_fade_consolidation(&self) -> Result<(), String> {
        let mut entries = self.retrieve_all().await?;
        let now_ts = decay::get_current_timestamp();
        let write_txn = self.db.begin_write().map_err(|e| e.to_string())?;
        {
            let mut table = write_txn
                .open_table(MEMORY_TABLE)
                .map_err(|e| e.to_string())?;
            for entry in &mut entries {
                if matches!(
                    entry.memory_type,
                    MemoryType::Episodic
                        | MemoryType::InteractionEvent
                        | MemoryType::UserProfileDynamic
                        | MemoryType::UserExpression
                        | MemoryType::PreferenceSignal
                ) {
                    decay::apply_decay(entry, now_ts, 0.2);
                    let serialized = serde_json::to_string(entry).map_err(|e| e.to_string())?;
                    table
                        .insert(entry.id.as_str(), serialized.as_str())
                        .map_err(|e| e.to_string())?;
                }
            }
        }
        write_txn.commit().map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn delete(&self, id: &str) -> Result<(), String> {
        let write_txn = self.db.begin_write().map_err(|e| e.to_string())?;
        {
            let mut table = write_txn
                .open_table(MEMORY_TABLE)
                .map_err(|e| e.to_string())?;
            table.remove(id).map_err(|e| e.to_string())?;
        }
        write_txn.commit().map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn retrieve_all(&self) -> Result<Vec<MemoryEntry>, String> {
        let read_txn = self.db.begin_read().map_err(|e| e.to_string())?;
        let table = read_txn
            .open_table(MEMORY_TABLE)
            .map_err(|e| e.to_string())?;
        let mut entries = Vec::new();
        let iter = table.iter().map_err(|e| e.to_string())?;
        for result in iter {
            let (_key, value) = result.map_err(|e| e.to_string())?;
            let entry: MemoryEntry =
                serde_json::from_str(value.value()).map_err(|e| e.to_string())?;
            entries.push(entry);
        }
        Ok(entries)
    }

    async fn expand_relations(&self, entries: &[MemoryEntry]) -> Result<Vec<MemoryEntry>, String> {
        let mut result = entries.to_vec();
        let mut seen = result
            .iter()
            .map(|entry| entry.id.clone())
            .collect::<std::collections::HashSet<_>>();
        let read_txn = self.db.begin_read().map_err(|e| e.to_string())?;
        let table = read_txn
            .open_table(MEMORY_TABLE)
            .map_err(|e| e.to_string())?;

        for entry in entries {
            for related_id in entry.memory_relations.keys() {
                if !seen.insert(related_id.clone()) {
                    continue;
                }
                if let Some(value) = table.get(related_id.as_str()).map_err(|e| e.to_string())? {
                    let related: MemoryEntry =
                        serde_json::from_str(value.value()).map_err(|e| e.to_string())?;
                    if related.is_retrievable() {
                        result.push(related);
                    }
                }
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{MemoryEntry, MemoryQuery, MemoryType, Provenance};

    fn test_path(name: &str) -> String {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir()
            .join(format!("loop_memory_{}_{}.redb", name, nanos))
            .to_string_lossy()
            .to_string()
    }

    #[tokio::test]
    async fn stores_and_retrieves_user_scoped_memory() {
        let path = test_path("scoped");
        let store = RedbMemoryStore::new(&path).unwrap();
        let mut entry = MemoryEntry::new(
            "mem_1".to_string(),
            MemoryType::UserExpression,
            "I care about graceful product memory design.".to_string(),
            decay::get_current_timestamp(),
            None,
        );
        entry.namespace = Some("user:alice".to_string());
        entry.provenance = Provenance::UserExplicit;
        store.store(entry).await.unwrap();

        let results = store
            .retrieve(MemoryQuery::EntityLookup {
                entity: "product memory".to_string(),
                namespace: Some("user:alice".to_string()),
            })
            .await
            .unwrap();
        assert_eq!(results.len(), 1);

        let other_results = store
            .retrieve(MemoryQuery::EntityLookup {
                entity: "product memory".to_string(),
                namespace: Some("user:bob".to_string()),
            })
            .await
            .unwrap();
        assert!(other_results.is_empty());

        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn vector_search_uses_supplied_embeddings() {
        let path = test_path("vector");
        let store = RedbMemoryStore::new(&path).unwrap();
        let mut first = MemoryEntry::new(
            "vec_1".to_string(),
            MemoryType::PreferenceSignal,
            "Likes preference learning".to_string(),
            decay::get_current_timestamp(),
            Some(vec![1.0, 0.0]),
        );
        first.namespace = Some("user:alice".to_string());
        let mut second = MemoryEntry::new(
            "vec_2".to_string(),
            MemoryType::PreferenceSignal,
            "Likes generic tutorials".to_string(),
            decay::get_current_timestamp(),
            Some(vec![0.0, 1.0]),
        );
        second.namespace = Some("user:alice".to_string());

        store.store(first).await.unwrap();
        store.store(second).await.unwrap();

        let results = store
            .retrieve(MemoryQuery::VectorSearch {
                query: vec![0.9, 0.1],
                top_k: 1,
                namespace: Some("user:alice".to_string()),
            })
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "vec_1");

        let _ = std::fs::remove_file(path);
    }
}
