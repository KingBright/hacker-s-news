use crate::core::config::Config;
use crate::core::llm::LlmClient;
use crate::core::nexus::{LoopPostPayload, MemoryEntryPayload, NexusClient};
use anyhow::Result;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

const DEFAULT_MAX_POSTS_PER_CYCLE: usize = 20;
const LOOP_CONTEXT_LIMIT: usize = 16_000;

#[derive(Debug, Clone, Serialize)]
pub struct LoopPreferenceRunStats {
    pub considered_posts: usize,
    pub processed_posts: usize,
    pub skipped_posts: usize,
    pub failed_posts: usize,
    pub written_signals: usize,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct PreferenceSignalAnalysis {
    signals: Vec<PreferenceSignalDraft>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct PreferenceSignalDraft {
    content: String,
    signal_type: String,
    polarity: String,
    confidence: f32,
    strength: f32,
    evidence: String,
}

pub struct LoopPreferencePipeline {
    config: Arc<Config>,
    llm: Arc<LlmClient>,
    nexus: Arc<NexusClient>,
}

impl LoopPreferencePipeline {
    pub fn new(config: Arc<Config>, llm: Arc<LlmClient>, nexus: Arc<NexusClient>) -> Self {
        Self { config, llm, nexus }
    }

    pub fn is_enabled(&self) -> bool {
        self.config
            .loop_preferences
            .as_ref()
            .is_none_or(|config| config.enabled)
    }

    pub async fn run_once(&self) -> Result<LoopPreferenceRunStats> {
        if !self.is_enabled() {
            return Ok(LoopPreferenceRunStats {
                considered_posts: 0,
                processed_posts: 0,
                skipped_posts: 0,
                failed_posts: 0,
                written_signals: 0,
            });
        }

        let max_posts = self
            .config
            .loop_preferences
            .as_ref()
            .and_then(|config| config.max_posts_per_cycle)
            .unwrap_or(DEFAULT_MAX_POSTS_PER_CYCLE)
            .clamp(1, 100);
        let posts = self
            .nexus
            .fetch_pending_loop_posts(max_posts as u32)
            .await?;
        let mut stats = LoopPreferenceRunStats {
            considered_posts: posts.len(),
            processed_posts: 0,
            skipped_posts: 0,
            failed_posts: 0,
            written_signals: 0,
        };

        for post in posts {
            match self.process_post(&post).await {
                Ok(0) => {
                    stats.skipped_posts += 1;
                    if let Err(e) = self
                        .nexus
                        .mark_loop_post_preference_result(&post.id, "skipped", None)
                        .await
                    {
                        log::warn!(
                            "[LoopPreference] failed to mark skipped post {}: {}",
                            post.id,
                            e
                        );
                    }
                }
                Ok(count) => {
                    stats.processed_posts += 1;
                    stats.written_signals += count;
                    if let Err(e) = self
                        .nexus
                        .mark_loop_post_preference_result(&post.id, "processed", None)
                        .await
                    {
                        log::warn!(
                            "[LoopPreference] failed to mark processed post {}: {}",
                            post.id,
                            e
                        );
                    }
                }
                Err(e) => {
                    stats.failed_posts += 1;
                    let err = truncate_chars(&e.to_string(), 500);
                    if let Err(mark_err) = self
                        .nexus
                        .mark_loop_post_preference_result(&post.id, "failed", Some(err))
                        .await
                    {
                        log::warn!(
                            "[LoopPreference] failed to mark failed post {}: {}",
                            post.id,
                            mark_err
                        );
                    }
                }
            }
        }

        Ok(stats)
    }

    async fn process_post(&self, post: &LoopPostPayload) -> Result<usize> {
        let prompt = build_preference_extraction_prompt(post);
        let analysis = self
            .llm
            .chat_json::<PreferenceSignalAnalysis>(&prompt, "loop_preference_signal", true)
            .await?;
        let signals = filter_signals(analysis.signals);
        if signals.is_empty() {
            return Ok(0);
        }

        let mut written = 0usize;
        for signal in signals {
            let mut metadata = HashMap::new();
            metadata.insert("post_id".to_string(), post.id.clone());
            metadata.insert("post_type".to_string(), post.post_type.clone());
            if let Some(feedback_mode) = &post.feedback_mode {
                metadata.insert("feedback_mode".to_string(), feedback_mode.clone());
            }
            metadata.insert("signal_type".to_string(), signal.signal_type.clone());
            metadata.insert("polarity".to_string(), signal.polarity.clone());
            metadata.insert(
                "evidence".to_string(),
                truncate_chars(&signal.evidence, 1000),
            );
            if let Some(source_ref) = &post.source_ref {
                metadata.insert("post_source_ref".to_string(), source_ref.clone());
            }

            let entry = MemoryEntryPayload {
                user_id: post.user_id.clone(),
                content: signal_to_memory_content(&signal, post),
                memory_type: Some("PreferenceSignal".to_string()),
                strength: Some(signal.strength.clamp(0.1, 5.0)),
                source_ref: Some(format!("loop_post:{}", post.id)),
                metadata: Some(metadata),
                provenance: Some("LlmExtracted".to_string()),
                confidence: Some(signal.confidence.clamp(0.0, 1.0)),
                is_static: Some(false),
                forget_after: None,
            };
            self.nexus.push_memory_entry(entry).await?;
            written += 1;
        }

        Ok(written)
    }
}

fn filter_signals(signals: Vec<PreferenceSignalDraft>) -> Vec<PreferenceSignalDraft> {
    signals
        .into_iter()
        .filter_map(|mut signal| {
            signal.content = signal.content.trim().to_string();
            signal.signal_type = signal.signal_type.trim().to_string();
            signal.polarity = signal.polarity.trim().to_string();
            signal.evidence = signal.evidence.trim().to_string();
            if signal.content.is_empty()
                || signal.signal_type.is_empty()
                || signal.confidence < 0.35
                || signal.strength < 0.1
            {
                return None;
            }
            Some(signal)
        })
        .take(8)
        .collect()
}

fn build_preference_extraction_prompt(post: &LoopPostPayload) -> String {
    let context = format_loop_post_context(post, LOOP_CONTEXT_LIMIT);
    format!(
        "你是 FreshLoop 的个人记忆分析器。请从用户的一条 Loop 发布中提取偏好信号。\n\n\
Loop 内容:\n{}\n\n\
任务:\n\
1. 只提取能影响后续推荐、简报、阅读压缩、声音脚本或产品理解的偏好。\n\
2. 不要把整篇原文主题粗暴理解成用户喜欢；要判断用户真正赞成、反感、关注或想持续跟踪的对象。\n\
3. 允许灰色判断，例如“整体一般但某个机制有价值”。这种情况要把信号落在具体机制上，而不是原文整体。\n\
3.5. 如果 Feedback Mode 是 boost / reduce / observe，请把它视为强约束：boost 表示用户想临时加重，reduce 表示用户想暂时降一点，但都不是永久拉黑。\n\
4. 忽略寒暄、纯事实复述、没有偏好含义的摘录。\n\
5. 每条 signal 的 content 用中文，写成可长期保存的第一人称偏好事实，例如：“用户关注把反馈转化为下一轮推荐输入的产品机制”。\n\
6. signal_type 可使用：liked_concept, disliked_concept, noise_boundary, source_preference, format_preference, project_focus, follow_up_interest, action_preference。\n\
7. polarity 使用 positive、negative、mixed、neutral 之一。\n\
8. confidence 取 0-1；strength 取 0.1-5.0，越能影响长期推荐越高。\n\n\
输出 JSON：{{\"signals\":[{{\"content\":\"...\",\"signal_type\":\"...\",\"polarity\":\"positive\",\"confidence\":0.8,\"strength\":3.0,\"evidence\":\"用户原话或近似证据\"}}]}}\n\
如果没有可用偏好，返回 {{\"signals\":[]}}。",
        context
    )
}

fn format_loop_post_context(post: &LoopPostPayload, max_chars: usize) -> String {
    let mut context = String::new();
    context.push_str(&format!("Post ID: {}\n", post.id));
    context.push_str(&format!("User ID: {}\n", post.user_id));
    context.push_str(&format!("Type: {}\n", post.post_type));
    if let Some(feedback_mode) = post
        .feedback_mode
        .as_deref()
        .filter(|feedback_mode| !feedback_mode.is_empty())
    {
        context.push_str(&format!("Feedback Mode: {}\n", feedback_mode));
    }
    if let Some(title) = post.title.as_deref().filter(|title| !title.is_empty()) {
        context.push_str(&format!("Title: {}\n", title));
    }
    if let Some(source_ref) = post
        .source_ref
        .as_deref()
        .filter(|source_ref| !source_ref.is_empty())
    {
        context.push_str(&format!("Source Ref: {}\n", source_ref));
    }
    context.push_str("\nBody:\n");
    context.push_str(&post.body);

    if !post.references.is_empty() {
        context.push_str("\n\nReferences:\n");
        for reference in &post.references {
            context.push_str(&format!("- Type: {}", reference.source_type));
            if let Some(title) = reference.title.as_deref().filter(|title| !title.is_empty()) {
                context.push_str(&format!(" | Title: {}", title));
            }
            if let Some(source_id) = reference
                .source_id
                .as_deref()
                .filter(|source_id| !source_id.is_empty())
            {
                context.push_str(&format!(" | ID: {}", source_id));
            }
            context.push('\n');
            if let Some(quote) = reference
                .quote_text
                .as_deref()
                .filter(|quote| !quote.is_empty())
            {
                context.push_str("  Quote: ");
                context.push_str(quote);
                context.push('\n');
            }
        }
    }

    truncate_chars(&context, max_chars)
}

fn signal_to_memory_content(signal: &PreferenceSignalDraft, post: &LoopPostPayload) -> String {
    format!(
        "[PreferenceSignal]\nType: {}\nPolarity: {}\nSource: loop_post:{}\nSignal: {}\nEvidence: {}",
        signal.signal_type, signal.polarity, post.id, signal.content, signal.evidence
    )
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for ch in value.chars().take(max_chars) {
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::nexus::LoopPostReferencePayload;

    fn sample_post() -> LoopPostPayload {
        LoopPostPayload {
            id: "post_1".to_string(),
            user_id: "user_1".to_string(),
            post_type: "quote_comment".to_string(),
            feedback_mode: Some("boost".to_string()),
            title: Some("Claude agent note".to_string()),
            body: "这篇整体一般，但把评价变成下一轮推荐输入这个点很有用。".to_string(),
            visibility: "private".to_string(),
            source_ref: Some("feed_item:abc".to_string()),
            memory_entry_id: Some("mem_1".to_string()),
            preference_status: Some("pending".to_string()),
            preference_extracted_at: None,
            preference_error: None,
            created_at: Some(1),
            updated_at: Some(1),
            status: Some("published".to_string()),
            references: vec![LoopPostReferencePayload {
                id: "ref_1".to_string(),
                post_id: "post_1".to_string(),
                source_type: "article".to_string(),
                source_id: Some("abc".to_string()),
                source_url: Some("https://example.com".to_string()),
                title: Some("Original article".to_string()),
                quote_text: Some("Feedback loop improves the brief.".to_string()),
                start_ms: None,
                end_ms: None,
                created_at: Some(1),
            }],
        }
    }

    #[test]
    fn prompt_preserves_gray_area_instruction() {
        let prompt = build_preference_extraction_prompt(&sample_post());
        assert!(prompt.contains("整体一般但某个机制有价值"));
        assert!(prompt.contains("评价变成下一轮推荐输入"));
    }

    #[test]
    fn filters_weak_or_empty_signals() {
        let signals = vec![
            PreferenceSignalDraft {
                content: "用户关注反馈循环".to_string(),
                signal_type: "liked_concept".to_string(),
                polarity: "positive".to_string(),
                confidence: 0.8,
                strength: 3.0,
                evidence: "很有用".to_string(),
            },
            PreferenceSignalDraft {
                content: "   ".to_string(),
                signal_type: "liked_concept".to_string(),
                polarity: "positive".to_string(),
                confidence: 0.9,
                strength: 3.0,
                evidence: "x".to_string(),
            },
            PreferenceSignalDraft {
                content: "低置信度".to_string(),
                signal_type: "liked_concept".to_string(),
                polarity: "positive".to_string(),
                confidence: 0.2,
                strength: 3.0,
                evidence: "x".to_string(),
            },
        ];

        let filtered = filter_signals(signals);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].content, "用户关注反馈循环");
    }

    #[test]
    fn signal_memory_content_links_source_post() {
        let post = sample_post();
        let signal = PreferenceSignalDraft {
            content: "用户关注反馈进入推荐闭环".to_string(),
            signal_type: "project_focus".to_string(),
            polarity: "positive".to_string(),
            confidence: 0.86,
            strength: 4.0,
            evidence: "这个点很有用".to_string(),
        };
        let content = signal_to_memory_content(&signal, &post);
        assert!(content.contains("loop_post:post_1"));
        assert!(content.contains("用户关注反馈进入推荐闭环"));
    }
}
