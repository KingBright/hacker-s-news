use crate::types::{MemoryEntry, MemoryType};

pub fn get_current_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn apply_decay(entry: &mut MemoryEntry, current_ts: u64, min_strength: f32) -> bool {
    if entry.is_static {
        return false;
    }

    if let Some(forget_after) = entry.forget_after {
        if current_ts > forget_after {
            entry.is_forgotten = true;
            entry.forget_reason = Some("temporal_expiry".to_string());
            return true;
        }
    }

    let half_life_hours: f32 = match entry.memory_type {
        MemoryType::Episodic => 24.0,
        MemoryType::InteractionEvent => 48.0,
        MemoryType::UserProfileDynamic => 72.0,
        MemoryType::UserExpression => 24.0 * 30.0,
        MemoryType::PreferenceSignal => 24.0 * 90.0,
        MemoryType::Semantic | MemoryType::Procedural | MemoryType::UserProfileStatic => {
            return false
        }
    };

    let age_hours = current_ts.saturating_sub(entry.last_accessed) as f32 / 3600.0;
    let decay_factor = (-age_hours / half_life_hours).exp();
    entry.current_strength = entry.base_strength * decay_factor;

    if entry.current_strength < min_strength {
        entry.is_forgotten = true;
        entry.forget_reason = Some("decayed_below_strength".to_string());
        return true;
    }

    false
}
