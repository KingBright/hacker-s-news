use crate::engine::MemoryStore;
use crate::types::{MemoryQuery, MemoryType, UserProfile};
use std::sync::Arc;
use tokio::sync::RwLock;

const DYNAMIC_WINDOW_SECS: u64 = 72 * 3600;
const MAX_DYNAMIC_ENTRIES: usize = 20;
const MAX_PREFERENCE_SIGNALS: usize = 40;
const DEFAULT_MAX_PROFILE_CHARS: usize = 3200;

pub async fn build_user_profile(
    memory_store: &dyn MemoryStore,
    namespace: Option<&str>,
) -> UserProfile {
    let entries = match memory_store
        .retrieve(MemoryQuery::TimeRange {
            start: 0,
            end: u64::MAX,
            namespace: namespace.map(str::to_string),
        })
        .await
    {
        Ok(entries) => entries,
        Err(_) => return UserProfile::default(),
    };

    let now_ts = crate::decay::get_current_timestamp();
    let cutoff = now_ts.saturating_sub(DYNAMIC_WINDOW_SECS);
    let mut static_facts = Vec::new();
    let mut dynamic_entries = Vec::new();
    let mut preference_entries = Vec::new();

    for entry in entries {
        if !entry.is_retrievable() {
            continue;
        }
        match entry.memory_type {
            MemoryType::UserProfileStatic => static_facts.push(entry.content),
            MemoryType::UserProfileDynamic | MemoryType::InteractionEvent => {
                if entry.created_at >= cutoff {
                    dynamic_entries.push(entry);
                }
            }
            MemoryType::PreferenceSignal => preference_entries.push(entry),
            _ => {}
        }
    }

    dynamic_entries.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    dynamic_entries.truncate(MAX_DYNAMIC_ENTRIES);
    preference_entries.sort_by(|a, b| {
        b.current_strength
            .partial_cmp(&a.current_strength)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.created_at.cmp(&a.created_at))
    });
    preference_entries.truncate(MAX_PREFERENCE_SIGNALS);

    UserProfile {
        static_facts,
        dynamic_context: dynamic_entries
            .into_iter()
            .map(|entry| entry.content)
            .collect(),
        preference_signals: preference_entries
            .into_iter()
            .map(|entry| entry.content)
            .collect(),
    }
}

pub fn format_profile_for_prompt(profile: &UserProfile) -> String {
    format_profile_for_prompt_budgeted(profile, None)
}

pub fn format_profile_for_prompt_budgeted(
    profile: &UserProfile,
    max_chars: Option<usize>,
) -> String {
    if profile.static_facts.is_empty()
        && profile.dynamic_context.is_empty()
        && profile.preference_signals.is_empty()
    {
        return String::new();
    }

    let budget = max_chars.unwrap_or(DEFAULT_MAX_PROFILE_CHARS);
    let mut output = String::new();

    append_section(
        &mut output,
        "[USER BACKGROUND — PERSISTENT KNOWLEDGE]\n",
        &profile.static_facts,
        budget / 3,
    );
    append_section(
        &mut output,
        "[CURRENT CONTEXT — RECENT ACTIVITY]\n",
        &profile.dynamic_context,
        budget / 4,
    );
    append_section(
        &mut output,
        "[PREFERENCE SIGNALS — HOW TO SHAPE THE NEXT LOOP]\n",
        &profile.preference_signals,
        budget - (budget / 3) - (budget / 4),
    );

    output
}

fn append_section(output: &mut String, header: &str, items: &[String], budget: usize) {
    if items.is_empty() {
        return;
    }
    let start_len = output.len();
    output.push_str(header);
    for item in items {
        let line = format!("• {}\n", item);
        if output.len().saturating_sub(start_len) + line.len() > budget {
            output.push_str("• ... (more memories available via search)\n");
            break;
        }
        output.push_str(&line);
    }
    output.push('\n');
}

pub async fn build_and_format_profile(
    memory_store: &dyn MemoryStore,
    namespace: Option<&str>,
) -> String {
    let profile = build_user_profile(memory_store, namespace).await;
    format_profile_for_prompt(&profile)
}

pub struct ProfileCache {
    cached: Arc<RwLock<Option<(String, u64)>>>,
    ttl_ms: u64,
}

impl ProfileCache {
    pub fn new(ttl_ms: u64) -> Self {
        Self {
            cached: Arc::new(RwLock::new(None)),
            ttl_ms,
        }
    }

    pub async fn get_or_refresh(
        &self,
        memory_store: &dyn MemoryStore,
        namespace: Option<&str>,
        max_chars: Option<usize>,
    ) -> String {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        {
            let cached = self.cached.read().await;
            if let Some((profile, ts)) = cached.as_ref() {
                if now_ms.saturating_sub(*ts) < self.ttl_ms {
                    return profile.clone();
                }
            }
        }

        let profile = build_user_profile(memory_store, namespace).await;
        let formatted = format_profile_for_prompt_budgeted(&profile, max_chars);
        {
            let mut cached = self.cached.write().await;
            *cached = Some((formatted.clone(), now_ms));
        }
        formatted
    }

    pub async fn invalidate(&self) {
        let mut cached = self.cached.write().await;
        *cached = None;
    }
}
