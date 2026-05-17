use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, Datelike, Duration, FixedOffset, TimeZone, Weekday};
use futures::stream::{self, StreamExt};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::core::config::{Config, CuratedFeedConfig, CuratedFeedSource};
use crate::core::content::{
    clean_text_for_processing, fetch_feed_entries, fetch_url_bytes, parse_opml_sources,
    ContentSource, FeedFetchOptions, FetchedEntry, ProductLine,
};
use crate::core::llm::LlmClient;
use crate::core::nexus::{
    FeedItemContentPayload, FeedItemPayload, NexusClient, WeeklyDigestPayload,
};
use crate::core::tts::TtsClient;

const DEFAULT_MAX_ITEMS_PER_CYCLE: usize = 30;
const DEFAULT_MAX_AGE_DAYS: i64 = 2;
const DEFAULT_MIN_QUALITY_SCORE: u8 = 6;
const DEFAULT_ARTICLE_AUDIO_MIN_QUALITY_SCORE: u8 = 8;
const DEFAULT_ARTICLE_AUDIO_MAX_ITEMS_PER_CYCLE: usize = 3;
const DEFAULT_WEEKLY_DIGEST_MIN_ITEMS: usize = 3;
const DEFAULT_WEEKLY_DIGEST_MAX_ITEMS: usize = 12;
const SOURCE_FETCH_CONCURRENCY: usize = 12;
const ARTICLE_CONTEXT_LIMIT: usize = 12_000;
const ARTICLE_AUDIO_CHAR_LIMIT: usize = 5_500;
const WEEKLY_DIGEST_CONTEXT_LIMIT: usize = 18_000;
const WEEKLY_AUDIO_CHAR_LIMIT: usize = 6_500;

#[derive(Debug, Clone, Serialize)]
pub struct CuratedFeedRunStats {
    pub configured_sources: usize,
    pub resolved_sources: usize,
    pub fetched_entries: usize,
    pub published_items: usize,
    pub published_audio_items: usize,
    pub skipped_items: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct WeeklyDigestRunStats {
    pub considered_items: usize,
    pub included_items: usize,
    pub published: bool,
    pub skipped_reason: Option<String>,
}

#[derive(Debug, Clone)]
struct ArticleSnapshot {
    reader_markdown: String,
    plain_text: String,
    reading_time_min: i64,
    content_hash: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct ArticleAnalysis {
    title: String,
    quality_score: u8,
    topic: String,
    should_publish: bool,
    compressed_markdown: String,
    audio_script: String,
    key_points: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
struct WeeklyDigestAnalysis {
    title: String,
    digest_markdown: String,
    audio_script: String,
    themes: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct WeeklyDigestDraft {
    title: String,
    digest_markdown: String,
    audio_script: String,
    themes: Vec<String>,
}

#[derive(Debug, Clone)]
struct WeeklyDigestCandidate {
    id: String,
    title: String,
    source_name: Option<String>,
    original_url: Option<String>,
    publish_time: i64,
    quality_score: i32,
    content: FeedItemContentPayload,
}

#[derive(Debug, Clone)]
enum ProcessEntryOutcome {
    Published {
        item_id: String,
        audio_generated: bool,
    },
    Skipped,
}

pub struct CuratedFeedPipeline {
    config: Arc<Config>,
    llm: Arc<LlmClient>,
    tts: Arc<TtsClient>,
    nexus: Arc<NexusClient>,
}

impl CuratedFeedPipeline {
    pub fn new(
        config: Arc<Config>,
        llm: Arc<LlmClient>,
        tts: Arc<TtsClient>,
        nexus: Arc<NexusClient>,
    ) -> Self {
        Self {
            config,
            llm,
            tts,
            nexus,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.config
            .curated_feed
            .as_ref()
            .is_some_and(|config| config.enabled)
    }

    pub async fn run_once(&self, now: DateTime<FixedOffset>) -> Result<CuratedFeedRunStats> {
        let Some(feed_config) = self.config.curated_feed.as_ref() else {
            return Ok(CuratedFeedRunStats {
                configured_sources: 0,
                resolved_sources: 0,
                fetched_entries: 0,
                published_items: 0,
                published_audio_items: 0,
                skipped_items: 0,
            });
        };

        if !feed_config.enabled {
            log::info!("[CuratedFeed] disabled; skipping run");
            return Ok(CuratedFeedRunStats {
                configured_sources: feed_config.feeds.as_ref().map_or(0, Vec::len),
                resolved_sources: 0,
                fetched_entries: 0,
                published_items: 0,
                published_audio_items: 0,
                skipped_items: 0,
            });
        }

        let sources = self.resolve_sources(feed_config).await?;
        let mut stats = CuratedFeedRunStats {
            configured_sources: feed_config.feeds.as_ref().map_or(0, Vec::len),
            resolved_sources: sources.len(),
            fetched_entries: 0,
            published_items: 0,
            published_audio_items: 0,
            skipped_items: 0,
        };

        if sources.is_empty() {
            log::info!("[CuratedFeed] no sources configured");
            return Ok(stats);
        }

        let fetch_options = FeedFetchOptions::new(self.config.http_proxy.clone())
            .with_prefer_proxy(feed_config.prefer_proxy.unwrap_or(false));
        let max_age_days = feed_config.max_age_days.unwrap_or(DEFAULT_MAX_AGE_DAYS);
        let min_quality_score = feed_config
            .min_quality_score
            .unwrap_or(DEFAULT_MIN_QUALITY_SCORE);
        let max_items = feed_config
            .max_items_per_cycle
            .unwrap_or(DEFAULT_MAX_ITEMS_PER_CYCLE);
        let article_audio_enabled = feed_config.article_audio_enabled.unwrap_or(true);
        let article_audio_min_quality_score = feed_config
            .article_audio_min_quality_score
            .unwrap_or(DEFAULT_ARTICLE_AUDIO_MIN_QUALITY_SCORE);
        let article_audio_max_items = feed_config
            .article_audio_max_items_per_cycle
            .unwrap_or(DEFAULT_ARTICLE_AUDIO_MAX_ITEMS_PER_CYCLE);
        let mut audio_items_used = 0usize;
        let mut scheduled_audio_item_ids = HashSet::new();
        if article_audio_enabled {
            let backfilled = self
                .backfill_missing_article_audio(
                    article_audio_max_items,
                    article_audio_min_quality_score,
                    &scheduled_audio_item_ids,
                )
                .await?;
            for item_id in backfilled {
                scheduled_audio_item_ids.insert(item_id);
                audio_items_used += 1;
                stats.published_audio_items += 1;
            }
        }

        log::info!(
            "[CuratedFeed] resolved {} sources from {} configured feed entries",
            stats.resolved_sources,
            stats.configured_sources
        );

        let fetch_results = stream::iter(sources.iter().cloned())
            .map(|source| {
                let fetch_options = fetch_options.clone();
                async move {
                    let url = source.url.clone();
                    let error_source = source.clone();
                    fetch_feed_entries(&url, &fetch_options)
                        .await
                        .map(|entries| (source, entries))
                        .map_err(|e| (error_source, e))
                }
            })
            .buffer_unordered(SOURCE_FETCH_CONCURRENCY)
            .collect::<Vec<_>>()
            .await;

        let mut seen_links = HashSet::new();
        let mut candidates = Vec::new();

        for result in fetch_results {
            match result {
                Ok((source, entries)) => {
                    stats.fetched_entries += entries.len();
                    candidates.extend(entries.into_iter().filter_map(|entry| {
                        if !seen_links.insert(entry.link.clone()) {
                            return None;
                        }
                        if is_recent_enough(entry.pub_date.as_deref(), now, max_age_days) {
                            Some((source.clone(), entry))
                        } else {
                            None
                        }
                    }));
                }
                Err((source, e)) => {
                    log::warn!("[CuratedFeed] failed to fetch {}: {}", source.url, e);
                }
            }
        }

        candidates.sort_by(|(_, a), (_, b)| b.pub_date.cmp(&a.pub_date));
        candidates = self.filter_existing_candidates(candidates).await;
        candidates.truncate(max_items);

        for (source, entry) in candidates {
            let allow_audio = article_audio_enabled && audio_items_used < article_audio_max_items;
            match self
                .process_entry(
                    &source,
                    entry,
                    min_quality_score,
                    allow_audio,
                    article_audio_min_quality_score,
                    now,
                )
                .await
            {
                Ok(ProcessEntryOutcome::Published {
                    item_id,
                    audio_generated,
                }) => {
                    stats.published_items += 1;
                    if audio_generated {
                        scheduled_audio_item_ids.insert(item_id);
                        audio_items_used += 1;
                        stats.published_audio_items += 1;
                    }
                }
                Ok(ProcessEntryOutcome::Skipped) => stats.skipped_items += 1,
                Err(e) => {
                    stats.skipped_items += 1;
                    log::warn!("[CuratedFeed] item processing failed: {}", e);
                }
            }
        }

        if article_audio_enabled && audio_items_used < article_audio_max_items {
            let backfilled = self
                .backfill_missing_article_audio(
                    article_audio_max_items - audio_items_used,
                    article_audio_min_quality_score,
                    &scheduled_audio_item_ids,
                )
                .await?;
            for item_id in backfilled {
                scheduled_audio_item_ids.insert(item_id);
                stats.published_audio_items += 1;
            }
        }

        log::info!(
            "[CuratedFeed] done: sources={}, entries={}, published={}, audio={}, skipped={}",
            stats.resolved_sources,
            stats.fetched_entries,
            stats.published_items,
            stats.published_audio_items,
            stats.skipped_items
        );

        Ok(stats)
    }

    async fn filter_existing_candidates(
        &self,
        candidates: Vec<(ContentSource, FetchedEntry)>,
    ) -> Vec<(ContentSource, FetchedEntry)> {
        if candidates.is_empty() {
            return candidates;
        }

        let urls = candidates
            .iter()
            .map(|(_, entry)| entry.link.clone())
            .collect::<Vec<_>>();
        let existing = match self.nexus.check_urls(urls).await {
            Ok(urls) => urls.into_iter().collect::<HashSet<_>>(),
            Err(e) => {
                log::warn!(
                    "[CuratedFeed] dedup check failed before item selection; continuing without prefilter: {}",
                    e
                );
                return candidates;
            }
        };

        if existing.is_empty() {
            return candidates;
        }

        let before = candidates.len();
        let filtered = candidates
            .into_iter()
            .filter(|(_, entry)| !existing.contains(&entry.link))
            .collect::<Vec<_>>();
        log::info!(
            "[CuratedFeed] removed {} already-published candidate URLs before selection",
            before.saturating_sub(filtered.len())
        );
        filtered
    }

    async fn resolve_sources(&self, feed_config: &CuratedFeedConfig) -> Result<Vec<ContentSource>> {
        let mut resolved = Vec::new();
        let Some(configured_sources) = feed_config.feeds.as_ref() else {
            return Ok(resolved);
        };

        let fetch_options = FeedFetchOptions::new(self.config.http_proxy.clone())
            .with_prefer_proxy(feed_config.prefer_proxy.unwrap_or(false));
        for configured in configured_sources {
            let kind = configured.kind.as_deref().unwrap_or_else(|| {
                if configured.url.to_ascii_lowercase().ends_with(".opml") {
                    "opml"
                } else {
                    "rss"
                }
            });

            if kind.eq_ignore_ascii_case("opml") {
                match fetch_url_bytes(&configured.url, &fetch_options).await {
                    Ok(bytes) => {
                        let group = configured
                            .source_group
                            .as_deref()
                            .or(feed_config.source_group.as_deref());
                        let mut sources = parse_opml_sources(&bytes, group)?;
                        for source in &mut sources {
                            source.tags = configured.tags.clone().unwrap_or_default();
                        }
                        resolved.extend(sources);
                    }
                    Err(e) => log::warn!(
                        "[CuratedFeed] failed to fetch OPML {}: {}",
                        configured.url,
                        e
                    ),
                }
            } else {
                resolved.push(configured_source_to_content_source(
                    configured,
                    feed_config.source_group.as_deref(),
                ));
            }
        }

        Ok(resolved)
    }

    async fn process_entry(
        &self,
        source: &ContentSource,
        entry: FetchedEntry,
        min_quality_score: u8,
        allow_audio: bool,
        article_audio_min_quality_score: u8,
        now: DateTime<FixedOffset>,
    ) -> Result<ProcessEntryOutcome> {
        let snapshot = build_article_snapshot(&entry);
        if snapshot.plain_text.chars().count() < 80 {
            log::info!("[CuratedFeed] skip short entry: {}", entry.title);
            return Ok(ProcessEntryOutcome::Skipped);
        }

        let analysis = match self.analyze_article(source, &entry, &snapshot).await {
            Ok(analysis) => analysis,
            Err(e) => {
                log::warn!(
                    "[CuratedFeed] LLM analysis failed for '{}', publishing original only: {}",
                    entry.title,
                    e
                );
                fallback_analysis(&entry, &snapshot)
            }
        };

        if !analysis.should_publish || analysis.quality_score < min_quality_score {
            log::info!(
                "[CuratedFeed] skip low quality article score={} title={}",
                analysis.quality_score,
                entry.title
            );
            return Ok(ProcessEntryOutcome::Skipped);
        }

        let publish_time = entry
            .pub_date
            .as_deref()
            .and_then(parse_entry_timestamp)
            .unwrap_or_else(|| now.timestamp());
        let title = if analysis.title.trim().is_empty() {
            entry.title.clone()
        } else {
            analysis.title.clone()
        };
        let key_points_json = serde_json::to_string(&analysis.key_points).ok();
        let tags = build_tags(source, &analysis);
        let compressed_markdown = non_empty_string(analysis.compressed_markdown.clone());
        let audio_script = non_empty_string(analysis.audio_script.clone());
        let should_generate_audio =
            allow_audio && analysis.quality_score >= article_audio_min_quality_score;
        let audio_text = if should_generate_audio {
            Some(build_article_audio_text(
                &title, source, &entry, &analysis, &snapshot,
            ))
        } else {
            None
        };
        let source_name = source_name(source, &entry);

        let item = FeedItemPayload {
            id: None,
            product_line: Some("curated_feed".to_string()),
            item_type: "article".to_string(),
            primary_mode: "read".to_string(),
            title: title.clone(),
            subtitle: Some(analysis.topic.clone()),
            source_name: source_name.clone(),
            source_url: Some(source.url.clone()),
            original_url: Some(entry.link.clone()),
            canonical_url: Some(entry.link.clone()),
            content_hash: Some(snapshot.content_hash.clone()),
            publish_time: Some(publish_time),
            has_audio: Some(false),
            audio_url: None,
            duration_sec: None,
            reading_time_min: Some(snapshot.reading_time_min),
            quality_score: Some(i32::from(analysis.quality_score)),
            tags: tags.clone(),
            status: Some("published".to_string()),
            content: Some(FeedItemContentPayload {
                original_html: Some(entry.description),
                reader_markdown: Some(snapshot.reader_markdown),
                plain_text: Some(snapshot.plain_text),
                compressed_markdown,
                audio_script,
                key_points_json,
            }),
        };

        let push_result = self.nexus.push_feed_item(item).await?;
        if push_result.status != "created" {
            log::info!(
                "[CuratedFeed] skip duplicate feed item {} status={}",
                push_result.id,
                push_result.status
            );
            return Ok(ProcessEntryOutcome::Skipped);
        }

        log::info!(
            "[CuratedFeed] published readable feed item {}",
            push_result.id
        );
        let mut audio_generated = false;
        if let Some(audio_text) = audio_text {
            let nexus = self.nexus.clone();
            let tts = self.tts.clone();
            let item_id = push_result.id.clone();
            let title_for_log = title.clone();
            let audio_update = FeedItemPayload {
                id: Some(push_result.id.clone()),
                product_line: Some("curated_feed".to_string()),
                item_type: "article".to_string(),
                primary_mode: "read".to_string(),
                title: title.clone(),
                subtitle: Some(analysis.topic.clone()),
                source_name,
                source_url: Some(source.url.clone()),
                original_url: Some(entry.link.clone()),
                canonical_url: Some(entry.link.clone()),
                content_hash: Some(snapshot.content_hash.clone()),
                publish_time: Some(publish_time),
                has_audio: Some(true),
                audio_url: None,
                duration_sec: None,
                reading_time_min: Some(snapshot.reading_time_min),
                quality_score: Some(i32::from(analysis.quality_score)),
                tags,
                status: Some("published".to_string()),
                content: None,
            };
            tokio::spawn(async move {
                match generate_audio_file_with(
                    tts,
                    nexus.clone(),
                    "curated_article",
                    &audio_text,
                    ARTICLE_AUDIO_CHAR_LIMIT,
                )
                .await
                {
                    Ok((url, duration)) => {
                        let mut update = audio_update;
                        update.audio_url = Some(url);
                        update.duration_sec = Some(duration);
                        match nexus.push_feed_item(update).await {
                            Ok(result) => log::info!(
                                "[CuratedFeed] attached article audio {} status={}",
                                result.id,
                                result.status
                            ),
                            Err(e) => log::warn!(
                                "[CuratedFeed] failed to attach article audio {}: {}",
                                item_id,
                                e
                            ),
                        }
                    }
                    Err(e) => {
                        log::warn!(
                            "[CuratedFeed] article audio failed for '{}'; readable item remains published: {}",
                            title_for_log,
                            e
                        );
                    }
                }
            });
            audio_generated = true;
        }

        Ok(ProcessEntryOutcome::Published {
            item_id: push_result.id,
            audio_generated,
        })
    }

    async fn backfill_missing_article_audio(
        &self,
        max_items: usize,
        min_quality_score: u8,
        excluded_ids: &HashSet<String>,
    ) -> Result<Vec<String>> {
        if max_items == 0 {
            return Ok(Vec::new());
        }

        let items = self
            .nexus
            .fetch_feed_items("curated_feed", "article", 100)
            .await?;
        let mut scheduled_ids = Vec::new();

        for item in items {
            if scheduled_ids.len() >= max_items {
                break;
            }
            let Some(item_id) = item.id.clone() else {
                continue;
            };
            if excluded_ids.contains(&item_id) {
                continue;
            }
            if item.has_audio.unwrap_or(false)
                || item
                    .audio_url
                    .as_deref()
                    .is_some_and(|url| !url.trim().is_empty())
            {
                continue;
            }
            let quality_score = item.quality_score.unwrap_or_default();
            if quality_score < i32::from(min_quality_score) {
                continue;
            }
            let Some(content) = self.nexus.fetch_feed_item_content(&item_id).await? else {
                continue;
            };
            let Some(audio_text) = article_audio_text_from_content(&item.title, &content) else {
                continue;
            };

            let mut update = item.clone();
            update.id = Some(item_id.clone());
            update.product_line = Some("curated_feed".to_string());
            update.item_type = "article".to_string();
            update.primary_mode = "read".to_string();
            update.has_audio = Some(true);
            update.audio_url = None;
            update.duration_sec = None;
            update.status = Some("published".to_string());
            update.content = None;

            let nexus = self.nexus.clone();
            let tts = self.tts.clone();
            let title_for_log = item.title.clone();
            let scheduled_id = item_id.clone();
            tokio::spawn(async move {
                match generate_audio_file_with(
                    tts,
                    nexus.clone(),
                    "curated_article",
                    &audio_text,
                    ARTICLE_AUDIO_CHAR_LIMIT,
                )
                .await
                {
                    Ok((url, duration)) => {
                        update.audio_url = Some(url);
                        update.duration_sec = Some(duration);
                        match nexus.push_feed_item(update).await {
                            Ok(result) => log::info!(
                                "[CuratedFeed] backfilled article audio {} status={}",
                                result.id,
                                result.status
                            ),
                            Err(e) => log::warn!(
                                "[CuratedFeed] failed to backfill article audio {}: {}",
                                item_id,
                                e
                            ),
                        }
                    }
                    Err(e) => log::warn!(
                        "[CuratedFeed] article audio backfill failed for '{}': {}",
                        title_for_log,
                        e
                    ),
                }
            });
            scheduled_ids.push(scheduled_id);
        }

        if !scheduled_ids.is_empty() {
            log::info!(
                "[CuratedFeed] scheduled {} missing article audio backfills",
                scheduled_ids.len()
            );
        }

        Ok(scheduled_ids)
    }

    async fn analyze_article(
        &self,
        source: &ContentSource,
        entry: &FetchedEntry,
        snapshot: &ArticleSnapshot,
    ) -> Result<ArticleAnalysis> {
        let context = snapshot
            .plain_text
            .chars()
            .take(ARTICLE_CONTEXT_LIMIT)
            .collect::<String>();
        let prompt = format!(
            "你是 FreshLoop 精选阅读频道的主编。请评估这篇订阅文章是否值得进入每日精选，并生成中文干货压缩和单独的收听稿。\n\n\
来源: {}\n标题: {}\nURL: {}\n正文:\n{}\n\n\
要求:\n\
1. quality_score 用 0-10 评分，重点看原创性、信息密度、长期价值、技术/思想含量。\n\
2. should_publish 只有在值得读时才为 true。\n\
3. compressed_markdown 用中文 Markdown，结构为：核心观点、关键干货、限制/风险、适合谁读。\n\
4. audio_script 是给耳朵听的干货版，不朗读原文，不保留 Markdown 符号；开头直入主题，口语自然，2-4 分钟。\n\
5. key_points 给出 3-7 条最重要结论。\n\
6. 不要编造正文外的信息。",
            source.name, entry.title, entry.link, context
        );

        self.llm
            .chat_json::<ArticleAnalysis>(&prompt, "curated_article_analysis", false)
            .await
    }

    pub async fn run_weekly_digest(
        &self,
        now: DateTime<FixedOffset>,
        force: bool,
    ) -> Result<WeeklyDigestRunStats> {
        let Some(feed_config) = self.config.curated_feed.as_ref() else {
            return Ok(WeeklyDigestRunStats {
                considered_items: 0,
                included_items: 0,
                published: false,
                skipped_reason: Some("curated_feed_not_configured".to_string()),
            });
        };

        if !feed_config.enabled || !feed_config.weekly_digest_enabled.unwrap_or(true) {
            return Ok(WeeklyDigestRunStats {
                considered_items: 0,
                included_items: 0,
                published: false,
                skipped_reason: Some("weekly_digest_disabled".to_string()),
            });
        }

        if !force && now.weekday() != Weekday::Sun {
            return Ok(WeeklyDigestRunStats {
                considered_items: 0,
                included_items: 0,
                published: false,
                skipped_reason: Some("not_scheduled_weekday".to_string()),
            });
        }

        let (week_start, week_end) = rolling_week_bounds(now);
        let effective_end = week_end.min(now.timestamp());
        let existing_digest = self.weekly_digest_for_week(week_start, week_end).await?;
        if existing_digest
            .as_ref()
            .and_then(|digest| digest.audio_url.as_deref())
            .is_some_and(|url| !url.trim().is_empty())
        {
            return Ok(WeeklyDigestRunStats {
                considered_items: 0,
                included_items: 0,
                published: false,
                skipped_reason: Some("duplicate_week_with_audio".to_string()),
            });
        }

        let min_items = feed_config
            .weekly_digest_min_items
            .unwrap_or(DEFAULT_WEEKLY_DIGEST_MIN_ITEMS);
        let max_items = feed_config
            .weekly_digest_max_items
            .unwrap_or(DEFAULT_WEEKLY_DIGEST_MAX_ITEMS);

        let mut candidates = self
            .fetch_weekly_candidates(week_start, effective_end, max_items)
            .await?;
        let considered_items = candidates.len();

        if candidates.len() < min_items {
            return Ok(WeeklyDigestRunStats {
                considered_items,
                included_items: candidates.len(),
                published: false,
                skipped_reason: Some("not_enough_items".to_string()),
            });
        }

        candidates.sort_by(|a, b| {
            b.quality_score
                .cmp(&a.quality_score)
                .then_with(|| b.publish_time.cmp(&a.publish_time))
        });
        candidates.truncate(max_items);
        candidates.sort_by_key(|item| item.publish_time);

        let included_ids = candidates
            .iter()
            .map(|item| item.id.clone())
            .collect::<Vec<_>>();
        let draft = self
            .load_or_generate_weekly_draft(week_start, week_end, &candidates)
            .await?;

        let themes_json = serde_json::to_string(&draft.themes).ok();
        let included_item_ids_json = serde_json::to_string(&included_ids).ok();

        let digest = WeeklyDigestPayload {
            id: existing_digest
                .as_ref()
                .and_then(|digest| digest.id.clone()),
            feed_item_id: existing_digest
                .as_ref()
                .and_then(|digest| digest.feed_item_id.clone()),
            week_start,
            week_end,
            title: draft.title.clone(),
            digest_markdown: Some(draft.digest_markdown.clone()),
            audio_script: Some(draft.audio_script.clone()),
            audio_url: None,
            duration_sec: None,
            included_item_ids_json: included_item_ids_json.clone(),
            themes_json: themes_json.clone(),
            status: Some("published".to_string()),
        };

        let id = self.nexus.push_weekly_digest(digest).await?;
        log::info!("[CuratedFeed] published weekly digest manuscript {}", id);
        if !draft.audio_script.trim().is_empty() {
            let tts = self.tts.clone();
            let nexus = self.nexus.clone();
            let audio_script = draft.audio_script.clone();
            let title = draft.title.clone();
            tokio::spawn(async move {
                match generate_audio_file_with(
                    tts,
                    nexus.clone(),
                    "curated_weekly",
                    &audio_script,
                    WEEKLY_AUDIO_CHAR_LIMIT,
                )
                .await
                {
                    Ok((url, duration_sec)) => {
                        let update = WeeklyDigestPayload {
                            id: Some(id.clone()),
                            feed_item_id: None,
                            week_start,
                            week_end,
                            title,
                            digest_markdown: None,
                            audio_script: Some(audio_script),
                            audio_url: Some(url),
                            duration_sec: Some(duration_sec),
                            included_item_ids_json,
                            themes_json,
                            status: Some("published".to_string()),
                        };
                        match nexus.push_weekly_digest(update).await {
                            Ok(updated_id) => {
                                log::info!(
                                    "[CuratedFeed] attached weekly digest audio {}",
                                    updated_id
                                )
                            }
                            Err(e) => log::warn!(
                                "[CuratedFeed] failed to attach weekly digest audio {}: {}",
                                id,
                                e
                            ),
                        }
                    }
                    Err(e) => log::warn!(
                        "[CuratedFeed] weekly audio failed; manuscript remains published: {}",
                        e
                    ),
                }
            });
        }
        let _ = fs::remove_file(weekly_draft_cache_path(week_start, week_end));

        Ok(WeeklyDigestRunStats {
            considered_items,
            included_items: candidates.len(),
            published: true,
            skipped_reason: None,
        })
    }

    async fn weekly_digest_for_week(
        &self,
        week_start: i64,
        week_end: i64,
    ) -> Result<Option<WeeklyDigestPayload>> {
        let digests = self.nexus.fetch_weekly_digests().await?;
        Ok(digests
            .into_iter()
            .find(|digest| digest.week_start == week_start && digest.week_end == week_end))
    }

    async fn fetch_weekly_candidates(
        &self,
        week_start: i64,
        week_end: i64,
        max_items: usize,
    ) -> Result<Vec<WeeklyDigestCandidate>> {
        let limit = (max_items.max(DEFAULT_WEEKLY_DIGEST_MAX_ITEMS) * 4).min(100) as u32;
        let items = self
            .nexus
            .fetch_feed_items("curated_feed", "article", limit)
            .await?;
        let mut candidates = Vec::new();

        for item in items {
            let Some(id) = item.id.clone() else {
                continue;
            };
            let publish_time = item.publish_time.unwrap_or_default();
            if publish_time < week_start || publish_time > week_end {
                continue;
            }
            let Some(content) = self.nexus.fetch_feed_item_content(&id).await? else {
                continue;
            };
            candidates.push(WeeklyDigestCandidate {
                id,
                title: item.title,
                source_name: item.source_name,
                original_url: item.original_url,
                publish_time,
                quality_score: item.quality_score.unwrap_or_default(),
                content,
            });
        }

        Ok(candidates)
    }

    async fn load_or_generate_weekly_draft(
        &self,
        week_start: i64,
        week_end: i64,
        candidates: &[WeeklyDigestCandidate],
    ) -> Result<WeeklyDigestDraft> {
        let cache_path = weekly_draft_cache_path(week_start, week_end);
        if let Ok(bytes) = fs::read(&cache_path) {
            if let Ok(draft) = serde_json::from_slice::<WeeklyDigestDraft>(&bytes) {
                log::info!(
                    "[CuratedFeed] weekly digest draft cache hit: {:?}",
                    cache_path
                );
                return Ok(draft);
            }
        }

        let draft = match self.generate_weekly_digest_draft(candidates).await {
            Ok(analysis) => WeeklyDigestDraft {
                title: analysis.title,
                digest_markdown: analysis.digest_markdown,
                audio_script: analysis.audio_script,
                themes: analysis.themes,
            },
            Err(e) => {
                log::warn!(
                    "[CuratedFeed] weekly LLM generation failed; using deterministic fallback: {}",
                    e
                );
                fallback_weekly_digest(candidates)
            }
        };

        if let Some(parent) = cache_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_vec(&draft) {
            let _ = fs::write(&cache_path, json);
        }

        Ok(draft)
    }

    async fn generate_weekly_digest_draft(
        &self,
        candidates: &[WeeklyDigestCandidate],
    ) -> Result<WeeklyDigestAnalysis> {
        let context = build_weekly_digest_context(candidates, WEEKLY_DIGEST_CONTEXT_LIMIT);
        let prompt = format!(
            "你是 FreshLoop 精选订阅频道的周报主编。请基于本周精选文章生成一份中文周汇总，并写出适合语音收听的播客脚本。\n\n\
本周素材:\n{}\n\n\
要求:\n\
1. title 要像一个周报音频标题，短而有信息量。\n\
2. digest_markdown 面向阅读，结构为：本周主线、重要文章、交叉趋势、下周值得关注。\n\
3. audio_script 面向收听，语气自然，3-6 分钟，避免逐条机械报标题。\n\
4. themes 给出 3-6 个本周主题词。\n\
5. 只能使用素材中的事实，不要编造外部信息。",
            context
        );

        self.llm
            .chat_json::<WeeklyDigestAnalysis>(&prompt, "curated_weekly_digest", false)
            .await
    }
}

async fn generate_audio_file_with(
    tts: Arc<TtsClient>,
    nexus: Arc<NexusClient>,
    file_prefix: &str,
    text: &str,
    max_chars: usize,
) -> Result<(String, i64)> {
    let tts_text = prepare_tts_text(text, max_chars);
    if tts_text.trim().is_empty() {
        anyhow::bail!("empty TTS text");
    }

    let wav_bytes = tts.speak(&tts_text).await?;
    let duration = wav_duration_sec(&wav_bytes)?;
    let mp3_bytes = tts.convert_to_mp3(&wav_bytes).await?;
    let file_name = format!("{}_{}.mp3", file_prefix, Uuid::new_v4());
    let url = nexus.upload_audio(mp3_bytes, &file_name).await?;
    Ok((url, duration))
}

fn build_article_audio_text(
    title: &str,
    source: &ContentSource,
    entry: &FetchedEntry,
    analysis: &ArticleAnalysis,
    snapshot: &ArticleSnapshot,
) -> String {
    let body = if !analysis.audio_script.trim().is_empty() {
        analysis.audio_script.as_str()
    } else if !analysis.compressed_markdown.trim().is_empty() {
        analysis.compressed_markdown.as_str()
    } else {
        snapshot.plain_text.as_str()
    };
    let source = source_name(source, entry).unwrap_or_else(|| source.name.clone());
    format!(
        "这里是 FreshLoop 精选订阅。今天听一篇来自{}的干货版，标题是《{}》。\n\n{}",
        source, title, body
    )
}

fn article_audio_text_from_content(
    title: &str,
    content: &FeedItemContentPayload,
) -> Option<String> {
    let body = content
        .audio_script
        .as_deref()
        .filter(|text| !text.trim().is_empty())
        .or_else(|| {
            content
                .compressed_markdown
                .as_deref()
                .filter(|text| !text.trim().is_empty())
        })
        .or_else(|| {
            content
                .reader_markdown
                .as_deref()
                .filter(|text| !text.trim().is_empty())
        })
        .or_else(|| {
            content
                .plain_text
                .as_deref()
                .filter(|text| !text.trim().is_empty())
        })?;
    let prepared = prepare_tts_text(body, ARTICLE_AUDIO_CHAR_LIMIT);
    if prepared.chars().count() < 80 {
        return None;
    }

    Some(format!(
        "这里是 FreshLoop 精选订阅。下面是《{}》的干货版。\n\n{}",
        title, prepared
    ))
}

fn non_empty_string(value: String) -> Option<String> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value)
    }
}

fn configured_source_to_content_source(
    source: &CuratedFeedSource,
    default_group: Option<&str>,
) -> ContentSource {
    let mut content_source = ContentSource::new(
        stable_source_id(&source.url),
        source.name.clone().unwrap_or_else(|| source.url.clone()),
        source.url.clone(),
        ProductLine::CuratedFeed,
    );
    content_source.source_group = source
        .source_group
        .clone()
        .or_else(|| default_group.map(str::to_string));
    content_source.tags = source.tags.clone().unwrap_or_default();
    content_source
}

fn build_article_snapshot(entry: &FetchedEntry) -> ArticleSnapshot {
    let reader_markdown = if entry.description.trim().is_empty() {
        String::new()
    } else {
        html2md::parse_html(&entry.description)
    };
    let plain_text = clean_text_for_processing(&entry.description, 50_000);
    let reading_time_min = estimate_reading_time_min(&plain_text);
    let content_hash = content_hash(&entry.title, &entry.link, &plain_text);

    ArticleSnapshot {
        reader_markdown,
        plain_text,
        reading_time_min,
        content_hash,
    }
}

fn estimate_reading_time_min(text: &str) -> i64 {
    let chars = text.chars().filter(|ch| !ch.is_whitespace()).count() as i64;
    (chars / 500).clamp(1, 60)
}

fn content_hash(title: &str, link: &str, plain_text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(title.as_bytes());
    hasher.update(b"\n");
    hasher.update(link.as_bytes());
    hasher.update(b"\n");
    hasher.update(plain_text.as_bytes());
    hex::encode(hasher.finalize())
}

fn fallback_analysis(entry: &FetchedEntry, snapshot: &ArticleSnapshot) -> ArticleAnalysis {
    ArticleAnalysis {
        title: entry.title.clone(),
        quality_score: heuristic_quality_score(snapshot),
        topic: "精选文章".to_string(),
        should_publish: true,
        compressed_markdown: String::new(),
        audio_script: fallback_article_audio_script(entry, snapshot),
        key_points: Vec::new(),
    }
}

fn fallback_article_audio_script(entry: &FetchedEntry, snapshot: &ArticleSnapshot) -> String {
    let excerpt = prepare_tts_text(&snapshot.plain_text, 1_600);
    if excerpt.trim().is_empty() {
        return String::new();
    }

    format!(
        "这篇文章《{}》可以先抓住几个核心信息。{}\n\n以上是这篇文章的干货版，适合先听完再决定是否打开原文细读。",
        entry.title, excerpt
    )
}

fn heuristic_quality_score(snapshot: &ArticleSnapshot) -> u8 {
    let len = snapshot
        .plain_text
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .count();
    match len {
        0..=250 => 4,
        251..=800 => 6,
        801..=2_500 => 7,
        _ => 8,
    }
}

fn build_tags(source: &ContentSource, analysis: &ArticleAnalysis) -> Option<String> {
    let mut tags = source.tags.clone();
    if !analysis.topic.trim().is_empty() {
        tags.push(analysis.topic.trim().to_string());
    }
    if let Some(group) = &source.source_group {
        tags.push(group.clone());
    }
    tags.sort();
    tags.dedup();
    if tags.is_empty() {
        None
    } else {
        serde_json::to_string(&tags).ok()
    }
}

fn source_name(source: &ContentSource, entry: &FetchedEntry) -> Option<String> {
    entry
        .source_name
        .clone()
        .filter(|name| !name.trim().is_empty())
        .or_else(|| Some(source.name.clone()))
}

fn build_weekly_digest_context(candidates: &[WeeklyDigestCandidate], max_chars: usize) -> String {
    let mut context = String::new();
    for (idx, item) in candidates.iter().enumerate() {
        let compressed = item
            .content
            .compressed_markdown
            .as_deref()
            .filter(|text| !text.trim().is_empty());
        let plain = item
            .content
            .plain_text
            .as_deref()
            .filter(|text| !text.trim().is_empty());
        let body = compressed.or(plain).unwrap_or("");
        let body = body.chars().take(1_600).collect::<String>();
        let key_points = item.content.key_points_json.as_deref().unwrap_or("[]");

        context.push_str(&format!(
            "## Article {}\nTitle: {}\nSource: {}\nURL: {}\nQuality: {}\nKey points: {}\nContent:\n{}\n\n",
            idx + 1,
            item.title,
            item.source_name.as_deref().unwrap_or("Unknown"),
            item.original_url.as_deref().unwrap_or(""),
            item.quality_score,
            key_points,
            body
        ));

        if context.chars().count() >= max_chars {
            break;
        }
    }

    context.chars().take(max_chars).collect()
}

fn fallback_weekly_digest(candidates: &[WeeklyDigestCandidate]) -> WeeklyDigestDraft {
    let mut digest =
        String::from("## 本周主线\n\n本周精选订阅内容集中在以下几篇高价值文章。\n\n## 重要文章\n");
    let mut script =
        String::from("这里是 FreshLoop 精选订阅周汇总。本周值得集中听的内容有几条：\n\n");
    let mut themes = Vec::new();

    for item in candidates {
        digest.push_str(&format!("\n- **{}**", item.title));
        if let Some(source) = &item.source_name {
            digest.push_str(&format!("（{}）", source));
        }
        if let Some(points) = item.content.key_points_json.as_deref() {
            digest.push_str(&format!("：{}\n", points));
        } else {
            digest.push('\n');
        }

        script.push_str(&format!("《{}》。", item.title));
        if let Some(compressed) = item.content.compressed_markdown.as_deref() {
            let snippet = prepare_tts_text(compressed, 500);
            script.push_str(&snippet);
        }
        script.push_str("\n\n");

        if let Some(source) = &item.source_name {
            themes.push(source.clone());
        }
    }

    themes.sort();
    themes.dedup();
    themes.truncate(6);
    if themes.is_empty() {
        themes.push("精选订阅".to_string());
    }

    WeeklyDigestDraft {
        title: "FreshLoop 精选订阅周汇总".to_string(),
        digest_markdown: digest,
        audio_script: script,
        themes,
    }
}

fn prepare_tts_text(text: &str, max_chars: usize) -> String {
    let mut cleaned = text
        .replace("```", "")
        .replace('#', "")
        .replace('*', "")
        .replace('`', "")
        .replace('[', "")
        .replace(']', "")
        .replace('(', " ")
        .replace(')', " ");
    cleaned = cleaned
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n");

    if cleaned.chars().count() <= max_chars {
        cleaned
    } else {
        let mut truncated = cleaned
            .chars()
            .take(max_chars.saturating_sub(12))
            .collect::<String>();
        truncated.push_str("……本段先到这里。");
        truncated
    }
}

fn wav_duration_sec(wav_bytes: &[u8]) -> Result<i64> {
    let cursor = std::io::Cursor::new(wav_bytes);
    let reader = hound::WavReader::new(cursor)?;
    Ok((reader.duration() as f64 / reader.spec().sample_rate as f64).ceil() as i64)
}

fn rolling_week_bounds(now: DateTime<FixedOffset>) -> (i64, i64) {
    let start_day = now.date_naive() - Duration::days(6);
    let start_naive = start_day
        .and_hms_opt(0, 0, 0)
        .expect("midnight is always valid");
    let end_naive = now
        .date_naive()
        .and_hms_opt(23, 59, 59)
        .expect("end of day is always valid");
    let start = now
        .offset()
        .from_local_datetime(&start_naive)
        .single()
        .expect("FixedOffset has a single local datetime mapping");
    let end = now
        .offset()
        .from_local_datetime(&end_naive)
        .single()
        .expect("FixedOffset has a single local datetime mapping");
    (start.timestamp(), end.timestamp())
}

fn weekly_draft_cache_path(week_start: i64, week_end: i64) -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".freshloop/cache/curated_feed")
        .join(format!("weekly_{}_{}.json", week_start, week_end))
}

fn is_recent_enough(pub_date: Option<&str>, now: DateTime<FixedOffset>, max_age_days: i64) -> bool {
    let Some(pub_date) = pub_date else {
        return true;
    };
    let Some(ts) = parse_entry_timestamp(pub_date) else {
        return true;
    };
    ts >= now.timestamp() - max_age_days.max(1) * 24 * 3600
}

fn parse_entry_timestamp(value: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.timestamp())
        .ok()
}

fn stable_source_id(url: &str) -> String {
    let slug = url
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_ascii_lowercase();

    if slug.is_empty() {
        "curated-source".to_string()
    } else {
        slug
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimates_reading_time_with_floor() {
        assert_eq!(estimate_reading_time_min("short"), 1);
        assert_eq!(estimate_reading_time_min(&"a".repeat(1_100)), 2);
    }

    #[test]
    fn rejects_old_entries() {
        let now = DateTime::parse_from_rfc3339("2026-05-17T08:00:00+08:00").unwrap();
        assert!(is_recent_enough(Some("2026-05-16T09:00:00+08:00"), now, 2));
        assert!(!is_recent_enough(Some("2026-05-10T09:00:00+08:00"), now, 2));
    }

    #[test]
    fn calculates_fixed_offset_rolling_week_bounds() {
        let now = DateTime::parse_from_rfc3339("2026-05-18T01:30:00+08:00").unwrap();
        let (start, end) = rolling_week_bounds(now);
        assert_eq!(start, 1_778_515_200);
        assert_eq!(end, 1_779_119_999);
    }

    #[test]
    fn prepares_tts_text_without_markdown_noise() {
        let prepared = prepare_tts_text("## Title\n\n- **Point** with `code`", 200);
        assert!(!prepared.contains('#'));
        assert!(!prepared.contains('*'));
        assert!(prepared.contains("Point"));
    }
}
