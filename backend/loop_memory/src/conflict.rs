use crate::types::{MemoryEntry, MemoryType};

#[derive(Debug)]
pub struct ConflictResult {
    pub existing: MemoryEntry,
    pub similarity: f32,
}

pub fn detect_conflicts(
    new_entry: &MemoryEntry,
    existing_entries: &[MemoryEntry],
    similarity_threshold: f32,
    top_k: usize,
) -> Vec<ConflictResult> {
    let Some(new_embedding) = &new_entry.embedding else {
        return Vec::new();
    };

    let mut conflicts = Vec::new();
    for existing in existing_entries {
        if !existing.is_latest || existing.is_forgotten {
            continue;
        }
        if new_entry.namespace != existing.namespace {
            continue;
        }
        if existing.id == new_entry.id {
            continue;
        }
        if existing.memory_type != new_entry.memory_type {
            continue;
        }
        if !matches!(
            existing.memory_type,
            MemoryType::Semantic
                | MemoryType::UserProfileStatic
                | MemoryType::UserProfileDynamic
                | MemoryType::PreferenceSignal
        ) {
            continue;
        }
        let Some(existing_embedding) = &existing.embedding else {
            continue;
        };
        let similarity = cosine_similarity(new_embedding, existing_embedding);
        if similarity > similarity_threshold && existing.content != new_entry.content {
            conflicts.push(ConflictResult {
                existing: existing.clone(),
                similarity,
            });
        }
    }

    conflicts.sort_by(|a, b| {
        b.similarity
            .partial_cmp(&a.similarity)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    conflicts.truncate(top_k);
    conflicts
}

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let mut dot_product = 0.0;
    let mut norm_a = 0.0;
    let mut norm_b = 0.0;
    for (x, y) in a.iter().zip(b.iter()) {
        dot_product += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot_product / (norm_a.sqrt() * norm_b.sqrt())
    }
}
