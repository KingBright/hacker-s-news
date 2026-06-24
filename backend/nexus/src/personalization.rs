use crate::{
    routes::{feed::FeedItem, items::Item},
    AppState,
};
use loop_memory::{MemoryEntry, MemoryQuery, MemoryStore, MemoryType};
use serde::Serialize;
use sqlx::FromRow;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet, VecDeque};

const ACTIVE_WINDOW_SECS: i64 = 14 * 24 * 3600;
const PERSONALIZATION_LOOKBACK_POSTS: i64 = 180;
const MIN_CANDIDATE_LIMIT: i64 = 80;
const MAX_CANDIDATE_LIMIT: i64 = 300;

#[derive(Debug, Clone, Copy)]
enum Surface {
    Radio,
    Reading,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FocusBucket {
    Active,
    Stable,
    Explore,
}

#[derive(Debug, Clone, Serialize)]
pub struct FocusCard {
    pub label: String,
    pub kind: String,
    pub score: f32,
    pub evidence: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BalanceRule {
    pub active_pct: u8,
    pub stable_pct: u8,
    pub explore_pct: u8,
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FocusStats {
    pub expression_count: usize,
    pub processed_expression_count: usize,
    pub pending_expression_count: usize,
    pub signal_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct FocusSummaryResponse {
    pub current_focus: Vec<FocusCard>,
    pub long_term_focus: Vec<FocusCard>,
    pub recently_reduced: Vec<FocusCard>,
    pub preferred_sources: Vec<FocusCard>,
    pub preferred_formats: Vec<FocusCard>,
    pub reading_balance: BalanceRule,
    pub radio_balance: BalanceRule,
    pub stats: FocusStats,
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct WhyRecommendedResponse {
    pub item_id: String,
    pub surface: String,
    pub bucket: String,
    pub score: f32,
    pub active_score: f32,
    pub stable_score: f32,
    pub explore_score: f32,
    pub reduce_score: f32,
    pub reasons: Vec<String>,
    pub matched_focus: Vec<FocusCard>,
    pub balance: BalanceRule,
    pub note: String,
}

#[derive(Debug, Clone, Default)]
struct WeightedFocus {
    label: String,
    kind: String,
    score: f32,
    count: usize,
    latest_ts: i64,
    evidence: String,
}

#[derive(Debug, Clone, Default)]
struct UserSnapshot {
    active_sources: HashMap<String, WeightedFocus>,
    stable_sources: HashMap<String, WeightedFocus>,
    reduced_sources: HashMap<String, WeightedFocus>,
    active_topics: HashMap<String, WeightedFocus>,
    stable_topics: HashMap<String, WeightedFocus>,
    reduced_topics: HashMap<String, WeightedFocus>,
    active_signals: Vec<WeightedFocus>,
    stable_signals: Vec<WeightedFocus>,
    reduced_signals: Vec<WeightedFocus>,
    format_preferences: HashMap<String, WeightedFocus>,
    expression_count: usize,
    processed_expression_count: usize,
    pending_expression_count: usize,
    signal_count: usize,
}

#[derive(Debug, FromRow, Clone)]
struct LoopReferenceSeedRow {
    post_id: String,
    feedback_mode: Option<String>,
    preference_status: Option<String>,
    created_at: Option<i64>,
    body: String,
    source_type: Option<String>,
    source_id: Option<String>,
    source_url: Option<String>,
    title: Option<String>,
    quote_text: Option<String>,
}

#[derive(Debug, FromRow, Clone)]
struct FeedReferenceDetails {
    id: String,
    title: String,
    subtitle: Option<String>,
    source_name: Option<String>,
    original_url: Option<String>,
    tags: Option<String>,
}

#[derive(Debug, FromRow, Clone)]
struct RadioReferenceDetails {
    id: String,
    title: String,
    original_url: Option<String>,
    category: Option<String>,
    tags: Option<String>,
}

#[derive(Debug, Clone)]
struct RankFeatures {
    item_id: String,
    source_keys: Vec<String>,
    topic_keys: Vec<String>,
    publish_time: i64,
    quality_score: f32,
}

#[derive(Debug, Clone)]
struct ItemEvaluation {
    item_id: String,
    bucket: FocusBucket,
    active_score: f32,
    stable_score: f32,
    explore_score: f32,
    reduce_score: f32,
    total_score: f32,
    reasons: Vec<String>,
    matched_focus: Vec<FocusCard>,
}

trait Personalizable {
    fn build_features(&self) -> RankFeatures;
}

impl Personalizable for FeedItem {
    fn build_features(&self) -> RankFeatures {
        let mut source_keys = Vec::new();
        let mut topic_keys = Vec::new();

        for label in [
            self.source_name.as_deref(),
            self.original_url.as_deref(),
            self.canonical_url.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            extend_normalized_labels(&mut source_keys, label);
        }

        if let Some(subtitle) = self.subtitle.as_deref() {
            extend_normalized_labels(&mut topic_keys, subtitle);
        }
        if let Some(tags) = self.tags.as_deref() {
            for tag in split_structured_labels(tags) {
                extend_normalized_labels(&mut topic_keys, &tag);
            }
        }
        extend_normalized_labels(&mut topic_keys, &self.title);

        RankFeatures {
            item_id: self.id.clone(),
            source_keys: dedup_keys(source_keys),
            topic_keys: dedup_keys(topic_keys),
            publish_time: self.publish_time.unwrap_or_default(),
            quality_score: self.quality_score.unwrap_or_default() as f32,
        }
    }
}

impl Personalizable for Item {
    fn build_features(&self) -> RankFeatures {
        let mut source_keys = Vec::new();
        let mut topic_keys = Vec::new();

        for label in [self.original_url.as_deref()].into_iter().flatten() {
            extend_normalized_labels(&mut source_keys, label);
        }
        if let Some(category) = self.category.as_deref() {
            extend_normalized_labels(&mut topic_keys, category);
        }
        if let Some(tags) = self.tags.as_deref() {
            for tag in split_structured_labels(tags) {
                extend_normalized_labels(&mut topic_keys, &tag);
            }
        }
        extend_normalized_labels(&mut topic_keys, &self.title);

        RankFeatures {
            item_id: self.id.clone(),
            source_keys: dedup_keys(source_keys),
            topic_keys: dedup_keys(topic_keys),
            publish_time: self.publish_time.unwrap_or_default(),
            quality_score: 0.0,
        }
    }
}

pub fn recommended_candidate_limit(limit: i64, offset: i64) -> i64 {
    ((limit + offset).max(1) * 5).clamp(MIN_CANDIDATE_LIMIT, MAX_CANDIDATE_LIMIT)
}

pub async fn get_focus_summary(
    state: &AppState,
    user_id: &str,
) -> Result<FocusSummaryResponse, String> {
    let snapshot = build_snapshot(state, user_id).await?;
    Ok(build_focus_summary(snapshot))
}

pub async fn personalize_feed_items(
    state: &AppState,
    user_id: &str,
    items: Vec<FeedItem>,
) -> Result<Vec<FeedItem>, String> {
    let snapshot = build_snapshot(state, user_id).await?;
    if snapshot.is_empty() {
        return Ok(items);
    }
    Ok(reorder_items(items, &snapshot, Surface::Reading))
}

pub async fn personalize_radio_items(
    state: &AppState,
    user_id: &str,
    items: Vec<Item>,
) -> Result<Vec<Item>, String> {
    let snapshot = build_snapshot(state, user_id).await?;
    if snapshot.is_empty() {
        return Ok(items);
    }
    Ok(reorder_items(items, &snapshot, Surface::Radio))
}

pub async fn explain_feed_item(
    state: &AppState,
    user_id: &str,
    item: &FeedItem,
) -> Result<WhyRecommendedResponse, String> {
    let snapshot = build_snapshot(state, user_id).await?;
    Ok(build_why_response(
        evaluate_item(item.build_features(), &snapshot, Surface::Reading),
        Surface::Reading,
    ))
}

pub async fn explain_radio_item(
    state: &AppState,
    user_id: &str,
    item: &Item,
) -> Result<WhyRecommendedResponse, String> {
    let snapshot = build_snapshot(state, user_id).await?;
    Ok(build_why_response(
        evaluate_item(item.build_features(), &snapshot, Surface::Radio),
        Surface::Radio,
    ))
}

async fn build_snapshot(state: &AppState, user_id: &str) -> Result<UserSnapshot, String> {
    let namespace = format!("user:{user_id}");
    let memory_entries = state
        .memory_store
        .retrieve(MemoryQuery::TimeRange {
            start: 0,
            end: u64::MAX,
            namespace: Some(namespace),
        })
        .await
        .unwrap_or_default();

    let rows = sqlx::query_as::<_, LoopReferenceSeedRow>(
        r#"
        SELECT
            p.id AS post_id,
            p.feedback_mode AS feedback_mode,
            p.preference_status AS preference_status,
            p.created_at AS created_at,
            p.body AS body,
            r.source_type AS source_type,
            r.source_id AS source_id,
            r.source_url AS source_url,
            r.title AS title,
            r.quote_text AS quote_text
        FROM loop_posts p
        LEFT JOIN loop_post_references r ON r.post_id = p.id
        WHERE p.user_id = ?
          AND COALESCE(p.status, 'published') != 'deleted'
        ORDER BY p.created_at DESC, r.created_at ASC
        LIMIT ?
        "#,
    )
    .bind(user_id)
    .bind(PERSONALIZATION_LOOKBACK_POSTS)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    let feed_ids = rows
        .iter()
        .filter(|row| row.source_type.as_deref() == Some("article"))
        .filter_map(|row| row.source_id.clone())
        .collect::<HashSet<_>>();
    let radio_ids = rows
        .iter()
        .filter(|row| {
            matches!(
                row.source_type.as_deref(),
                Some("radio_item") | Some("audio_offset")
            )
        })
        .filter_map(|row| row.source_id.clone())
        .collect::<HashSet<_>>();

    let feed_details = fetch_feed_reference_details(&state.db, &feed_ids).await?;
    let radio_details = fetch_radio_reference_details(&state.db, &radio_ids).await?;

    let mut snapshot = UserSnapshot::default();
    let mut total_positive_sources = HashMap::<String, WeightedFocus>::new();
    let mut total_positive_topics = HashMap::<String, WeightedFocus>::new();
    let now = chrono::Utc::now().timestamp();
    let mut seen_posts = HashSet::new();

    for row in &rows {
        if seen_posts.insert(row.post_id.clone()) {
            snapshot.expression_count += 1;
            if row.preference_status.as_deref() == Some("processed") {
                snapshot.processed_expression_count += 1;
            } else {
                snapshot.pending_expression_count += 1;
            }
        }

        let created_at = row.created_at.unwrap_or_default();
        let is_active = now.saturating_sub(created_at) <= ACTIVE_WINDOW_SECS;
        let mode = normalize_feedback_mode(row.feedback_mode.as_deref());
        let base_weight = feedback_mode_weight(mode) * recency_weight(created_at, now);

        let mut body_terms = tokenize_text(&row.body);
        body_terms.extend(tokenize_text(row.quote_text.as_deref().unwrap_or_default()));
        body_terms.extend(tokenize_text(row.title.as_deref().unwrap_or_default()));
        body_terms.truncate(10);

        for term in body_terms {
            let evidence = if mode == "reduce" {
                "你最近主动把这个主题往后放了一点".to_string()
            } else {
                "你最近在 Loop 里主动提到过这个主题".to_string()
            };
            if mode == "reduce" {
                add_focus(
                    &mut snapshot.reduced_topics,
                    term,
                    "topic",
                    base_weight * 0.55,
                    created_at,
                    evidence,
                );
            } else {
                let focus = add_focus(
                    &mut total_positive_topics,
                    term.clone(),
                    "topic",
                    base_weight * 0.35,
                    created_at,
                    evidence.clone(),
                );
                if is_active {
                    add_focus(
                        &mut snapshot.active_topics,
                        focus.label.clone(),
                        "topic",
                        base_weight * 0.35,
                        created_at,
                        evidence,
                    );
                }
            }
        }

        match row.source_type.as_deref() {
            Some("article") => {
                if let Some(source_id) = row.source_id.as_deref() {
                    if let Some(detail) = feed_details.get(source_id) {
                        apply_article_reference(
                            &mut snapshot,
                            &mut total_positive_sources,
                            &mut total_positive_topics,
                            detail,
                            mode,
                            is_active,
                            base_weight,
                            created_at,
                        );
                    }
                }
            }
            Some("radio_item") | Some("audio_offset") => {
                if let Some(source_id) = row.source_id.as_deref() {
                    if let Some(detail) = radio_details.get(source_id) {
                        apply_radio_reference(
                            &mut snapshot,
                            &mut total_positive_sources,
                            &mut total_positive_topics,
                            detail,
                            mode,
                            is_active,
                            base_weight,
                            created_at,
                        );
                    }
                }
            }
            _ => {}
        }

        if let Some(source_url) = row.source_url.as_deref() {
            if let Some(domain) = extract_domain(source_url) {
                let evidence = if mode == "reduce" {
                    format!("你最近明确降低了 {domain} 这类来源的优先级")
                } else {
                    format!("你最近引用过来自 {domain} 的内容")
                };
                if mode == "reduce" {
                    add_focus(
                        &mut snapshot.reduced_sources,
                        domain,
                        "source",
                        base_weight,
                        created_at,
                        evidence,
                    );
                } else {
                    let focus = add_focus(
                        &mut total_positive_sources,
                        domain.clone(),
                        "source",
                        base_weight * 0.75,
                        created_at,
                        evidence.clone(),
                    );
                    if is_active {
                        add_focus(
                            &mut snapshot.active_sources,
                            focus.label.clone(),
                            "source",
                            base_weight * 0.75,
                            created_at,
                            evidence,
                        );
                    }
                }
            }
        }
    }

    for entry in memory_entries {
        absorb_memory_entry(&mut snapshot, entry, now, &mut total_positive_topics);
    }

    snapshot.stable_sources = select_stable_focus(total_positive_sources);
    let derived_stable_topics = select_stable_focus(total_positive_topics);
    merge_focus_maps(&mut snapshot.stable_topics, derived_stable_topics);
    snapshot.signal_count = snapshot.active_signals.len()
        + snapshot.stable_signals.len()
        + snapshot.reduced_signals.len();

    if snapshot.processed_expression_count > snapshot.expression_count {
        snapshot.processed_expression_count = snapshot.expression_count;
    }
    snapshot.pending_expression_count = snapshot
        .expression_count
        .saturating_sub(snapshot.processed_expression_count);

    Ok(snapshot)
}

fn build_focus_summary(snapshot: UserSnapshot) -> FocusSummaryResponse {
    FocusSummaryResponse {
        current_focus: top_focus_cards(
            &snapshot.active_topics,
            &snapshot.active_sources,
            &snapshot.active_signals,
            6,
        ),
        long_term_focus: top_focus_cards(
            &snapshot.stable_topics,
            &snapshot.stable_sources,
            &snapshot.stable_signals,
            6,
        ),
        recently_reduced: top_focus_cards(
            &snapshot.reduced_topics,
            &snapshot.reduced_sources,
            &snapshot.reduced_signals,
            4,
        ),
        preferred_sources: top_map_cards(
            &merge_owned_maps(
                snapshot.active_sources.clone(),
                snapshot.stable_sources.clone(),
            ),
            5,
        ),
        preferred_formats: top_map_cards(&snapshot.format_preferences, 3),
        reading_balance: balance_rule(Surface::Reading),
        radio_balance: balance_rule(Surface::Radio),
        stats: FocusStats {
            expression_count: snapshot.expression_count,
            processed_expression_count: snapshot.processed_expression_count,
            pending_expression_count: snapshot.pending_expression_count,
            signal_count: snapshot.signal_count,
        },
        note: "FreshLoop 不会直接砍掉内容，只会按你最近的表达、长期兴趣和探索配比动态重排。"
            .to_string(),
    }
}

fn build_why_response(evaluation: ItemEvaluation, surface: Surface) -> WhyRecommendedResponse {
    WhyRecommendedResponse {
        item_id: evaluation.item_id,
        surface: match surface {
            Surface::Radio => "radio".to_string(),
            Surface::Reading => "reading".to_string(),
        },
        bucket: bucket_name(evaluation.bucket).to_string(),
        score: round_score(evaluation.total_score),
        active_score: round_score(evaluation.active_score),
        stable_score: round_score(evaluation.stable_score),
        explore_score: round_score(evaluation.explore_score),
        reduce_score: round_score(evaluation.reduce_score),
        reasons: if evaluation.reasons.is_empty() {
            vec!["这条内容被保留为探索位，用来避免信息面越来越窄。".to_string()]
        } else {
            evaluation.reasons
        },
        matched_focus: evaluation.matched_focus,
        balance: balance_rule(surface),
        note: "解释只说明为什么被提前或延后，不代表其它主题被永久移除。".to_string(),
    }
}

fn reorder_items<T: Personalizable>(
    items: Vec<T>,
    snapshot: &UserSnapshot,
    surface: Surface,
) -> Vec<T> {
    let mut active = Vec::new();
    let mut stable = Vec::new();
    let mut explore = Vec::new();

    for (index, item) in items.into_iter().enumerate() {
        let evaluation = evaluate_item(item.build_features(), snapshot, surface);
        match evaluation.bucket {
            FocusBucket::Active => active.push((evaluation, index, item)),
            FocusBucket::Stable => stable.push((evaluation, index, item)),
            FocusBucket::Explore => explore.push((evaluation, index, item)),
        }
    }

    let sort_bucket = |bucket: &mut Vec<(ItemEvaluation, usize, T)>| {
        bucket.sort_by(|a, b| {
            b.0.total_score
                .partial_cmp(&a.0.total_score)
                .unwrap_or(Ordering::Equal)
                .then_with(|| a.1.cmp(&b.1))
        });
    };

    sort_bucket(&mut active);
    sort_bucket(&mut stable);
    sort_bucket(&mut explore);

    let mut active = VecDeque::from(active);
    let mut stable = VecDeque::from(stable);
    let mut explore = VecDeque::from(explore);
    let pattern = match surface {
        Surface::Reading => [
            FocusBucket::Active,
            FocusBucket::Stable,
            FocusBucket::Active,
            FocusBucket::Stable,
            FocusBucket::Explore,
        ]
        .to_vec(),
        Surface::Radio => [
            FocusBucket::Active,
            FocusBucket::Stable,
            FocusBucket::Explore,
            FocusBucket::Active,
            FocusBucket::Stable,
            FocusBucket::Explore,
            FocusBucket::Explore,
        ]
        .to_vec(),
    };

    let mut ordered = Vec::new();
    while !(active.is_empty() && stable.is_empty() && explore.is_empty()) {
        let mut placed = false;
        for bucket in &pattern {
            let next = match bucket {
                FocusBucket::Active => active.pop_front(),
                FocusBucket::Stable => stable.pop_front(),
                FocusBucket::Explore => explore.pop_front(),
            };
            if let Some((_, _, item)) = next {
                ordered.push(item);
                placed = true;
                break;
            }
        }

        if placed {
            continue;
        }

        let fallback = best_remaining(&mut active, &mut stable, &mut explore);
        if let Some((_, _, item)) = fallback {
            ordered.push(item);
        } else {
            break;
        }
    }

    ordered
}

fn best_remaining<T>(
    active: &mut VecDeque<(ItemEvaluation, usize, T)>,
    stable: &mut VecDeque<(ItemEvaluation, usize, T)>,
    explore: &mut VecDeque<(ItemEvaluation, usize, T)>,
) -> Option<(ItemEvaluation, usize, T)> {
    let active_score = active.front().map(|entry| entry.0.total_score);
    let stable_score = stable.front().map(|entry| entry.0.total_score);
    let explore_score = explore.front().map(|entry| entry.0.total_score);

    let best_bucket = [
        (FocusBucket::Active, active_score),
        (FocusBucket::Stable, stable_score),
        (FocusBucket::Explore, explore_score),
    ]
    .into_iter()
    .filter_map(|(bucket, score)| score.map(|score| (bucket, score)))
    .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal))
    .map(|(bucket, _)| bucket)?;

    match best_bucket {
        FocusBucket::Active => active.pop_front(),
        FocusBucket::Stable => stable.pop_front(),
        FocusBucket::Explore => explore.pop_front(),
    }
}

fn evaluate_item(
    features: RankFeatures,
    snapshot: &UserSnapshot,
    surface: Surface,
) -> ItemEvaluation {
    let mut active_score = 0.0f32;
    let mut stable_score = 0.0f32;
    let mut reduce_score = 0.0f32;
    let mut reasons = Vec::new();
    let mut matched_focus = Vec::new();
    let mut seen_labels = HashSet::new();

    for key in &features.source_keys {
        if let Some(focus) = snapshot.active_sources.get(key) {
            active_score += (focus.score.min(3.0)) * 0.9;
            push_focus_reason(
                &mut reasons,
                &mut matched_focus,
                &mut seen_labels,
                focus,
                "你最近连续引用过这个来源".to_string(),
            );
        }
        if let Some(focus) = snapshot.stable_sources.get(key) {
            stable_score += (focus.score.min(3.0)) * 0.75;
            push_focus_reason(
                &mut reasons,
                &mut matched_focus,
                &mut seen_labels,
                focus,
                "这是你反复回到的来源".to_string(),
            );
        }
        if let Some(focus) = snapshot.reduced_sources.get(key) {
            reduce_score += (focus.score.min(3.0)) * 0.8;
            push_focus_reason(
                &mut reasons,
                &mut matched_focus,
                &mut seen_labels,
                focus,
                "你最近明确降低过这类来源的权重".to_string(),
            );
        }
    }

    for key in &features.topic_keys {
        if let Some(focus) = snapshot.active_topics.get(key) {
            active_score += (focus.score.min(3.0)) * 0.55;
            push_focus_reason(
                &mut reasons,
                &mut matched_focus,
                &mut seen_labels,
                focus,
                "你最近在 Loop 里持续表达过这个主题".to_string(),
            );
        }
        if let Some(focus) = snapshot.stable_topics.get(key) {
            stable_score += (focus.score.min(3.0)) * 0.45;
            push_focus_reason(
                &mut reasons,
                &mut matched_focus,
                &mut seen_labels,
                focus,
                "这是你长期反复回到的主题".to_string(),
            );
        }
        if let Some(focus) = snapshot.reduced_topics.get(key) {
            reduce_score += (focus.score.min(3.0)) * 0.65;
            push_focus_reason(
                &mut reasons,
                &mut matched_focus,
                &mut seen_labels,
                focus,
                "你最近给这个主题降过一点权重".to_string(),
            );
        }
    }

    let recency = recency_bonus(features.publish_time);
    let quality_bonus = (features.quality_score / 10.0).clamp(0.0, 1.2);
    let matched_weight = active_score + stable_score + reduce_score;
    let explore_score = if matched_weight < 0.8 {
        1.2 + recency + quality_bonus * 0.6
    } else {
        0.35 + recency * 0.55 + quality_bonus * 0.2
    };

    let total_score =
        active_score * 1.0 + stable_score * 0.85 + explore_score * 0.4 - reduce_score * 0.55;
    let bucket = if active_score >= stable_score.max(explore_score * 1.2) && active_score > 0.1 {
        FocusBucket::Active
    } else if stable_score >= explore_score && stable_score > 0.1 {
        FocusBucket::Stable
    } else {
        FocusBucket::Explore
    };

    let mut reasons = reasons.into_iter().take(3).collect::<Vec<_>>();
    if reasons.is_empty() {
        reasons.push(match surface {
            Surface::Reading => "这篇文章被放在探索位，用来保持阅读范围不会越看越窄。".to_string(),
            Surface::Radio => "这条简报被放在探索位，用来给播放列表留出新的信息面。".to_string(),
        });
    }

    ItemEvaluation {
        item_id: features.item_id,
        bucket,
        active_score,
        stable_score,
        explore_score,
        reduce_score,
        total_score,
        reasons,
        matched_focus: matched_focus.into_iter().take(4).collect(),
    }
}

fn apply_article_reference(
    snapshot: &mut UserSnapshot,
    total_positive_sources: &mut HashMap<String, WeightedFocus>,
    total_positive_topics: &mut HashMap<String, WeightedFocus>,
    detail: &FeedReferenceDetails,
    mode: &str,
    is_active: bool,
    base_weight: f32,
    created_at: i64,
) {
    if let Some(source_name) = detail
        .source_name
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        let evidence = if mode == "reduce" {
            format!("你最近把来源 {} 的内容往后放了一点", source_name.trim())
        } else {
            format!("你最近引用过来源 {}", source_name.trim())
        };
        if mode == "reduce" {
            add_focus(
                &mut snapshot.reduced_sources,
                source_name.trim().to_string(),
                "source",
                base_weight,
                created_at,
                evidence,
            );
        } else {
            let focus = add_focus(
                total_positive_sources,
                source_name.trim().to_string(),
                "source",
                base_weight,
                created_at,
                evidence.clone(),
            );
            if is_active {
                add_focus(
                    &mut snapshot.active_sources,
                    focus.label.clone(),
                    "source",
                    base_weight,
                    created_at,
                    evidence,
                );
            }
        }
    }

    if let Some(original_url) = detail.original_url.as_deref() {
        if let Some(domain) = extract_domain(original_url) {
            let evidence = format!("你最近引用过来自 {} 的文章", domain);
            let focus = add_focus(
                total_positive_sources,
                domain.clone(),
                "source",
                base_weight * 0.65,
                created_at,
                evidence.clone(),
            );
            if is_active && mode != "reduce" {
                add_focus(
                    &mut snapshot.active_sources,
                    focus.label.clone(),
                    "source",
                    base_weight * 0.65,
                    created_at,
                    evidence,
                );
            }
        }
    }

    let mut labels = Vec::new();
    if let Some(tags) = detail.tags.as_deref() {
        labels.extend(split_structured_labels(tags));
    }
    if let Some(subtitle) = detail.subtitle.as_deref() {
        labels.push(subtitle.to_string());
    }
    labels.push(detail.title.clone());

    apply_topic_labels(
        snapshot,
        total_positive_topics,
        labels,
        mode,
        is_active,
        base_weight,
        created_at,
        "article",
    );
}

fn apply_radio_reference(
    snapshot: &mut UserSnapshot,
    total_positive_sources: &mut HashMap<String, WeightedFocus>,
    total_positive_topics: &mut HashMap<String, WeightedFocus>,
    detail: &RadioReferenceDetails,
    mode: &str,
    is_active: bool,
    base_weight: f32,
    created_at: i64,
) {
    if let Some(original_url) = detail.original_url.as_deref() {
        if let Some(domain) = extract_domain(original_url) {
            let evidence = format!("你最近引用过来自 {} 的音频来源", domain);
            let focus = add_focus(
                total_positive_sources,
                domain.clone(),
                "source",
                base_weight * 0.65,
                created_at,
                evidence.clone(),
            );
            if is_active && mode != "reduce" {
                add_focus(
                    &mut snapshot.active_sources,
                    focus.label.clone(),
                    "source",
                    base_weight * 0.65,
                    created_at,
                    evidence,
                );
            }
        }
    }

    let mut labels = Vec::new();
    if let Some(category) = detail.category.as_deref() {
        labels.push(category.to_string());
    }
    if let Some(tags) = detail.tags.as_deref() {
        labels.extend(split_structured_labels(tags));
    }
    labels.push(detail.title.clone());

    apply_topic_labels(
        snapshot,
        total_positive_topics,
        labels,
        mode,
        is_active,
        base_weight,
        created_at,
        "radio",
    );
}

fn apply_topic_labels(
    snapshot: &mut UserSnapshot,
    total_positive_topics: &mut HashMap<String, WeightedFocus>,
    labels: Vec<String>,
    mode: &str,
    is_active: bool,
    base_weight: f32,
    created_at: i64,
    format_label: &str,
) {
    let format_evidence = match format_label {
        "article" => "你近期更多是在文章里表达偏好".to_string(),
        "radio" => "你近期更多是在音频里表达偏好".to_string(),
        _ => "你最近在这个表达方式里比较活跃".to_string(),
    };
    add_focus(
        &mut snapshot.format_preferences,
        format_label.to_string(),
        "format",
        base_weight,
        created_at,
        format_evidence,
    );

    for label in labels {
        let evidence = if mode == "reduce" {
            "你最近把这类主题往后放了一点".to_string()
        } else {
            "你最近引用或评论过这类主题".to_string()
        };
        if mode == "reduce" {
            add_focus(
                &mut snapshot.reduced_topics,
                label,
                "topic",
                base_weight * 0.85,
                created_at,
                evidence,
            );
        } else {
            let focus = add_focus(
                total_positive_topics,
                label,
                "topic",
                base_weight * 0.85,
                created_at,
                evidence.clone(),
            );
            if is_active {
                add_focus(
                    &mut snapshot.active_topics,
                    focus.label.clone(),
                    "topic",
                    base_weight * 0.85,
                    created_at,
                    evidence,
                );
            }
        }
    }
}

fn absorb_memory_entry(
    snapshot: &mut UserSnapshot,
    entry: MemoryEntry,
    now: i64,
    total_positive_topics: &mut HashMap<String, WeightedFocus>,
) {
    if !entry.is_retrievable() {
        return;
    }

    let created_at = entry.created_at as i64;
    let weight = entry.current_strength.clamp(0.1, 5.0) * entry.confidence.clamp(0.0, 1.0);
    let is_active = now.saturating_sub(created_at) <= ACTIVE_WINDOW_SECS;
    let polarity = entry
        .metadata
        .get("polarity")
        .map(|value| value.as_str())
        .unwrap_or("positive");
    let signal_type = entry
        .metadata
        .get("signal_type")
        .cloned()
        .unwrap_or_else(|| "preference".to_string());
    let kind = if signal_type.contains("source") {
        "source"
    } else {
        "signal"
    };
    let evidence = entry
        .metadata
        .get("evidence")
        .cloned()
        .unwrap_or_else(|| "来自你在 Loop 里的表达".to_string());

    match entry.memory_type {
        MemoryType::PreferenceSignal => {
            let focus = WeightedFocus {
                label: entry.content.trim().to_string(),
                kind: kind.to_string(),
                score: weight,
                count: 1,
                latest_ts: created_at,
                evidence: evidence.clone(),
            };
            if polarity == "negative" || signal_type == "noise_boundary" {
                snapshot.reduced_signals.push(focus.clone());
                add_focus(
                    &mut snapshot.reduced_topics,
                    focus.label.clone(),
                    "signal",
                    weight * 0.7,
                    created_at,
                    evidence,
                );
            } else {
                if is_active {
                    snapshot.active_signals.push(focus.clone());
                }
                if entry.is_static || entry.access_count >= 2 || weight >= 1.8 {
                    snapshot.stable_signals.push(focus.clone());
                }
                for token in tokenize_text(&entry.content).into_iter().take(8) {
                    let focus = add_focus(
                        total_positive_topics,
                        token.clone(),
                        "topic",
                        weight * 0.18,
                        created_at,
                        "模型从你的表达里提炼了这个偏好".to_string(),
                    );
                    if is_active {
                        add_focus(
                            &mut snapshot.active_topics,
                            focus.label.clone(),
                            "topic",
                            weight * 0.18,
                            created_at,
                            "模型从你的近期表达里提炼了这个偏好".to_string(),
                        );
                    }
                }
            }
        }
        MemoryType::UserProfileStatic => {
            snapshot.stable_signals.push(WeightedFocus {
                label: entry.content.trim().to_string(),
                kind: "signal".to_string(),
                score: weight,
                count: 1,
                latest_ts: created_at,
                evidence,
            });
        }
        MemoryType::UserProfileDynamic | MemoryType::InteractionEvent => {
            if is_active {
                snapshot.active_signals.push(WeightedFocus {
                    label: entry.content.trim().to_string(),
                    kind: "signal".to_string(),
                    score: weight,
                    count: 1,
                    latest_ts: created_at,
                    evidence,
                });
            }
        }
        _ => {}
    }
}

async fn fetch_feed_reference_details(
    db: &sqlx::SqlitePool,
    ids: &HashSet<String>,
) -> Result<HashMap<String, FeedReferenceDetails>, String> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }

    let placeholders = std::iter::repeat("?")
        .take(ids.len())
        .collect::<Vec<_>>()
        .join(", ");
    let query = format!(
        "SELECT id, title, subtitle, source_name, original_url, tags FROM feed_items WHERE id IN ({placeholders})"
    );
    let mut query = sqlx::query_as::<_, FeedReferenceDetails>(&query);
    for id in ids {
        query = query.bind(id);
    }
    let rows = query.fetch_all(db).await.map_err(|e| e.to_string())?;
    Ok(rows
        .into_iter()
        .map(|row| (row.id.clone(), row))
        .collect::<HashMap<_, _>>())
}

async fn fetch_radio_reference_details(
    db: &sqlx::SqlitePool,
    ids: &HashSet<String>,
) -> Result<HashMap<String, RadioReferenceDetails>, String> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }

    let placeholders = std::iter::repeat("?")
        .take(ids.len())
        .collect::<Vec<_>>()
        .join(", ");
    let query = format!(
        "SELECT id, title, original_url, category, tags FROM items WHERE id IN ({placeholders})"
    );
    let mut query = sqlx::query_as::<_, RadioReferenceDetails>(&query);
    for id in ids {
        query = query.bind(id);
    }
    let rows = query.fetch_all(db).await.map_err(|e| e.to_string())?;
    Ok(rows
        .into_iter()
        .map(|row| (row.id.clone(), row))
        .collect::<HashMap<_, _>>())
}

fn select_stable_focus(input: HashMap<String, WeightedFocus>) -> HashMap<String, WeightedFocus> {
    input
        .into_iter()
        .filter_map(|(key, mut focus)| {
            if focus.count >= 2 || focus.score >= 2.2 {
                focus.score *= 0.85;
                Some((key, focus))
            } else {
                None
            }
        })
        .collect()
}

fn top_focus_cards(
    topics: &HashMap<String, WeightedFocus>,
    sources: &HashMap<String, WeightedFocus>,
    signals: &[WeightedFocus],
    limit: usize,
) -> Vec<FocusCard> {
    let mut cards = top_map_cards(topics, limit);
    cards.extend(top_map_cards(sources, limit));
    cards.extend(
        signals
            .iter()
            .cloned()
            .map(weighted_to_card)
            .collect::<Vec<_>>(),
    );
    cards.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.label.cmp(&b.label))
    });
    cards.truncate(limit);
    cards
}

fn top_map_cards(map: &HashMap<String, WeightedFocus>, limit: usize) -> Vec<FocusCard> {
    let mut items = map.values().cloned().collect::<Vec<_>>();
    items.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.label.cmp(&b.label))
    });
    items
        .into_iter()
        .take(limit)
        .map(weighted_to_card)
        .collect()
}

fn weighted_to_card(focus: WeightedFocus) -> FocusCard {
    FocusCard {
        label: focus.label,
        kind: focus.kind,
        score: round_score(focus.score),
        evidence: focus.evidence,
    }
}

fn merge_focus_maps(
    target: &mut HashMap<String, WeightedFocus>,
    source: HashMap<String, WeightedFocus>,
) {
    for (key, focus) in source {
        let entry = target.entry(key).or_insert_with(|| focus.clone());
        if entry.label.is_empty() {
            entry.label = focus.label.clone();
        }
        if entry.kind.is_empty() {
            entry.kind = focus.kind.clone();
        }
        entry.score += focus.score;
        entry.count += focus.count;
        entry.latest_ts = entry.latest_ts.max(focus.latest_ts);
        if entry.evidence.is_empty() {
            entry.evidence = focus.evidence.clone();
        }
    }
}

fn merge_owned_maps(
    mut first: HashMap<String, WeightedFocus>,
    second: HashMap<String, WeightedFocus>,
) -> HashMap<String, WeightedFocus> {
    merge_focus_maps(&mut first, second);
    first
}

fn add_focus(
    map: &mut HashMap<String, WeightedFocus>,
    label: String,
    kind: &str,
    delta: f32,
    created_at: i64,
    evidence: String,
) -> WeightedFocus {
    let normalized = normalize_key(&label);
    if normalized.is_empty() {
        return WeightedFocus::default();
    }
    let entry = map.entry(normalized).or_default();
    if entry.label.is_empty() {
        entry.label = label.trim().to_string();
    }
    if entry.kind.is_empty() {
        entry.kind = kind.to_string();
    }
    entry.score += delta.max(0.0);
    entry.count += 1;
    entry.latest_ts = entry.latest_ts.max(created_at);
    if entry.evidence.is_empty() {
        entry.evidence = evidence;
    }
    entry.clone()
}

fn push_focus_reason(
    reasons: &mut Vec<String>,
    matched_focus: &mut Vec<FocusCard>,
    seen_labels: &mut HashSet<String>,
    focus: &WeightedFocus,
    prefix: String,
) {
    if !seen_labels.insert(focus.label.clone()) {
        return;
    }
    reasons.push(format!("{prefix}：{}", focus.label));
    matched_focus.push(FocusCard {
        label: focus.label.clone(),
        kind: focus.kind.clone(),
        score: round_score(focus.score),
        evidence: focus.evidence.clone(),
    });
}

fn extend_normalized_labels(target: &mut Vec<String>, label: &str) {
    let trimmed = label.trim();
    if trimmed.is_empty() {
        return;
    }
    let normalized = normalize_key(trimmed);
    if !normalized.is_empty() {
        target.push(normalized);
    }
    if let Some(domain) = extract_domain(trimmed) {
        target.push(domain);
    }
    target.extend(tokenize_text(trimmed));
}

fn split_structured_labels(value: &str) -> Vec<String> {
    value
        .split([',', '|', ';', '/', '\n'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn tokenize_text(text: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut ascii = String::new();
    let mut cjk = String::new();

    let flush_ascii = |result: &mut Vec<String>, ascii: &mut String| {
        if ascii.len() >= 3 {
            result.push(normalize_key(ascii));
        }
        ascii.clear();
    };
    let flush_cjk = |result: &mut Vec<String>, cjk: &mut String| {
        let char_count = cjk.chars().count();
        if (2..=12).contains(&char_count) {
            result.push(normalize_key(cjk));
        }
        cjk.clear();
    };

    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            flush_cjk(&mut result, &mut cjk);
            ascii.push(ch.to_ascii_lowercase());
        } else if is_cjk(ch) {
            flush_ascii(&mut result, &mut ascii);
            cjk.push(ch);
        } else {
            flush_ascii(&mut result, &mut ascii);
            flush_cjk(&mut result, &mut cjk);
        }
    }

    flush_ascii(&mut result, &mut ascii);
    flush_cjk(&mut result, &mut cjk);
    dedup_keys(result)
}

fn dedup_keys(keys: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    keys.into_iter()
        .filter(|key| !key.is_empty() && seen.insert(key.clone()))
        .collect()
}

fn normalize_key(value: &str) -> String {
    value
        .trim()
        .trim_matches(|ch: char| !ch.is_alphanumeric() && !is_cjk(ch))
        .to_lowercase()
}

fn extract_domain(url_or_label: &str) -> Option<String> {
    let raw = url_or_label
        .trim()
        .strip_prefix("https://")
        .or_else(|| url_or_label.trim().strip_prefix("http://"))
        .unwrap_or(url_or_label.trim());
    let host = raw.split('/').next()?.trim().trim_start_matches("www.");
    if host.contains('.') {
        Some(host.to_lowercase())
    } else {
        None
    }
}

fn recency_weight(created_at: i64, now: i64) -> f32 {
    let age_days = now.saturating_sub(created_at).max(0) as f32 / 86_400.0;
    (1.25 / (1.0 + age_days / 7.0)).clamp(0.35, 1.25)
}

fn recency_bonus(publish_time: i64) -> f32 {
    let now = chrono::Utc::now().timestamp();
    let age_days = now.saturating_sub(publish_time).max(0) as f32 / 86_400.0;
    (0.9 / (1.0 + age_days / 5.0)).clamp(0.05, 0.9)
}

fn feedback_mode_weight(mode: &str) -> f32 {
    match mode {
        "boost" => 1.4,
        "reduce" => 1.15,
        "observe" => 0.7,
        _ => 1.0,
    }
}

fn normalize_feedback_mode(value: Option<&str>) -> &'static str {
    match value.unwrap_or("balance").trim() {
        "boost" => "boost",
        "reduce" => "reduce",
        "observe" => "observe",
        _ => "balance",
    }
}

fn balance_rule(surface: Surface) -> BalanceRule {
    match surface {
        Surface::Reading => BalanceRule {
            active_pct: 45,
            stable_pct: 35,
            explore_pct: 20,
            note: "阅读优先级 = 最近表达 45% + 长期兴趣 35% + 探索 20%".to_string(),
        },
        Surface::Radio => BalanceRule {
            active_pct: 40,
            stable_pct: 30,
            explore_pct: 30,
            note: "播放队列 = 最近表达 40% + 长期兴趣 30% + 探索 30%".to_string(),
        },
    }
}

fn bucket_name(bucket: FocusBucket) -> &'static str {
    match bucket {
        FocusBucket::Active => "active",
        FocusBucket::Stable => "stable",
        FocusBucket::Explore => "explore",
    }
}

fn round_score(value: f32) -> f32 {
    (value * 100.0).round() / 100.0
}

fn is_cjk(ch: char) -> bool {
    matches!(ch as u32, 0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0x20000..=0x2A6DF)
}

impl UserSnapshot {
    fn is_empty(&self) -> bool {
        self.expression_count == 0 && self.signal_count == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizes_mixed_text() {
        let tokens = tokenize_text("AI agent 工作流 / Karpathy");
        assert!(tokens.contains(&"agent".to_string()));
        assert!(tokens.contains(&"工作流".to_string()));
        assert!(tokens.contains(&"karpathy".to_string()));
    }

    #[test]
    fn derives_candidate_limit_with_guardrails() {
        assert_eq!(recommended_candidate_limit(10, 0), 80);
        assert_eq!(recommended_candidate_limit(40, 10), 250);
        assert_eq!(recommended_candidate_limit(100, 100), 300);
    }

    #[test]
    fn reading_pattern_keeps_explore_items() {
        let mut snapshot = UserSnapshot::default();
        add_focus(
            &mut snapshot.active_topics,
            "agent".to_string(),
            "topic",
            2.0,
            1,
            "recent".to_string(),
        );
        add_focus(
            &mut snapshot.stable_topics,
            "systems".to_string(),
            "topic",
            2.0,
            1,
            "stable".to_string(),
        );

        let items = vec![
            FeedItem {
                id: "active".to_string(),
                product_line: "curated_feed".to_string(),
                item_type: "article".to_string(),
                primary_mode: "read".to_string(),
                title: "Agent design".to_string(),
                subtitle: None,
                source_name: None,
                source_url: None,
                original_url: None,
                canonical_url: None,
                content_hash: None,
                publish_time: Some(1),
                created_at: Some(1),
                updated_at: Some(1),
                has_audio: Some(false),
                audio_url: None,
                duration_sec: None,
                reading_time_min: None,
                quality_score: Some(8),
                tags: None,
                status: Some("published".to_string()),
            },
            FeedItem {
                id: "stable".to_string(),
                product_line: "curated_feed".to_string(),
                item_type: "article".to_string(),
                primary_mode: "read".to_string(),
                title: "Systems memo".to_string(),
                subtitle: None,
                source_name: None,
                source_url: None,
                original_url: None,
                canonical_url: None,
                content_hash: None,
                publish_time: Some(1),
                created_at: Some(1),
                updated_at: Some(1),
                has_audio: Some(false),
                audio_url: None,
                duration_sec: None,
                reading_time_min: None,
                quality_score: Some(8),
                tags: None,
                status: Some("published".to_string()),
            },
            FeedItem {
                id: "explore".to_string(),
                product_line: "curated_feed".to_string(),
                item_type: "article".to_string(),
                primary_mode: "read".to_string(),
                title: "Fresh territory".to_string(),
                subtitle: None,
                source_name: None,
                source_url: None,
                original_url: None,
                canonical_url: None,
                content_hash: None,
                publish_time: Some(1),
                created_at: Some(1),
                updated_at: Some(1),
                has_audio: Some(false),
                audio_url: None,
                duration_sec: None,
                reading_time_min: None,
                quality_score: Some(8),
                tags: None,
                status: Some("published".to_string()),
            },
        ];

        let ordered = reorder_items(items, &snapshot, Surface::Reading);
        let ids = ordered.into_iter().map(|item| item.id).collect::<Vec<_>>();
        assert_eq!(ids, vec!["active", "stable", "explore"]);
    }
}
