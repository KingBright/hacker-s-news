use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{FromRow, SqlitePool};
use uuid::Uuid;

use crate::{personalization, AppState};

const MAX_TITLE_LENGTH: usize = 500;
const MAX_URL_LENGTH: usize = 2048;
const MAX_BODY_LENGTH: usize = 2_000_000;
const MIN_LIMIT: i64 = 1;
const MAX_LIMIT: i64 = 100;

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct FeedItem {
    pub id: String,
    pub product_line: String,
    pub item_type: String,
    pub primary_mode: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub source_name: Option<String>,
    pub source_url: Option<String>,
    pub original_url: Option<String>,
    pub canonical_url: Option<String>,
    pub content_hash: Option<String>,
    pub publish_time: Option<i64>,
    pub created_at: Option<i64>,
    pub updated_at: Option<i64>,
    pub has_audio: Option<bool>,
    pub audio_url: Option<String>,
    pub duration_sec: Option<i64>,
    pub reading_time_min: Option<i64>,
    pub quality_score: Option<i32>,
    pub tags: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Serialize, FromRow)]
pub struct FeedItemContent {
    pub item_id: String,
    pub original_html: Option<String>,
    pub reader_markdown: Option<String>,
    pub plain_text: Option<String>,
    pub compressed_markdown: Option<String>,
    pub audio_script: Option<String>,
    pub key_points_json: Option<String>,
    pub created_at: Option<i64>,
    pub updated_at: Option<i64>,
}

#[derive(Debug, Serialize, FromRow)]
pub struct WeeklyDigest {
    pub id: String,
    pub feed_item_id: Option<String>,
    pub week_start: i64,
    pub week_end: i64,
    pub title: String,
    pub digest_markdown: Option<String>,
    pub audio_script: Option<String>,
    pub audio_url: Option<String>,
    pub duration_sec: Option<i64>,
    pub included_item_ids_json: Option<String>,
    pub themes_json: Option<String>,
    pub created_at: Option<i64>,
    pub status: Option<String>,
}

#[derive(Deserialize)]
pub struct FeedPagination {
    pub page: Option<i64>,
    pub limit: Option<i64>,
    pub product_line: Option<String>,
    pub item_type: Option<String>,
    pub primary_mode: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct FeedItemContentPayload {
    pub original_html: Option<String>,
    pub reader_markdown: Option<String>,
    pub plain_text: Option<String>,
    pub compressed_markdown: Option<String>,
    pub audio_script: Option<String>,
    pub key_points_json: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateFeedItemRequest {
    pub id: Option<String>,
    pub product_line: Option<String>,
    pub item_type: String,
    pub primary_mode: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub source_name: Option<String>,
    pub source_url: Option<String>,
    pub original_url: Option<String>,
    pub canonical_url: Option<String>,
    pub content_hash: Option<String>,
    pub publish_time: Option<i64>,
    pub has_audio: Option<bool>,
    pub audio_url: Option<String>,
    pub clear_audio: Option<bool>,
    pub duration_sec: Option<i64>,
    pub reading_time_min: Option<i64>,
    pub quality_score: Option<i32>,
    pub tags: Option<String>,
    pub status: Option<String>,
    pub content: Option<FeedItemContentPayload>,
}

#[derive(Debug, Deserialize)]
pub struct CreateWeeklyDigestRequest {
    pub id: Option<String>,
    pub feed_item_id: Option<String>,
    pub week_start: i64,
    pub week_end: i64,
    pub title: String,
    pub digest_markdown: Option<String>,
    pub audio_script: Option<String>,
    pub audio_url: Option<String>,
    pub duration_sec: Option<i64>,
    pub included_item_ids_json: Option<String>,
    pub themes_json: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateReadingProgressRequest {
    pub mode: Option<String>,
    pub scroll_ratio: Option<f64>,
    pub anchor: Option<String>,
    pub read_at: Option<i64>,
}

fn sanitize_pagination(page: Option<i64>, limit: Option<i64>) -> (i64, i64) {
    let limit = limit.unwrap_or(20).clamp(MIN_LIMIT, MAX_LIMIT);
    let page = page.unwrap_or(1).max(1);
    (limit, (page - 1) * limit)
}

fn is_valid_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

fn is_valid_audio_url(url: &str) -> bool {
    is_valid_url(url) || url.starts_with("/audio/")
}

fn has_internal_auth(headers: &HeaderMap, state: &AppState) -> bool {
    headers
        .get("X-NEXUS-KEY")
        .and_then(|value| value.to_str().ok())
        == Some(state.api_key.as_str())
}

fn validate_label(
    value: &str,
    allowed: &[&str],
    field: &str,
) -> Result<(), axum::response::Response> {
    if allowed.contains(&value) {
        return Ok(());
    }

    Err((
        StatusCode::BAD_REQUEST,
        Json(json!({
            "error": format!("Invalid {} '{}'", field, value),
            "allowed": allowed
        })),
    )
        .into_response())
}

fn validate_optional_url(
    url: &Option<String>,
    field: &str,
) -> Result<(), axum::response::Response> {
    if let Some(url) = url.as_deref().filter(|url| !url.is_empty()) {
        if url.len() > MAX_URL_LENGTH {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(
                    json!({ "error": format!("{} exceeds {} characters", field, MAX_URL_LENGTH) }),
                ),
            )
                .into_response());
        }
        if !is_valid_url(url) {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": format!("{} must start with http:// or https://", field) })),
            )
                .into_response());
        }
    }
    Ok(())
}

fn validate_optional_audio_url(url: &Option<String>) -> Result<(), axum::response::Response> {
    if let Some(url) = url.as_deref().filter(|url| !url.is_empty()) {
        if url.len() > MAX_URL_LENGTH {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": format!("audio_url exceeds {} characters", MAX_URL_LENGTH)
                })),
            )
                .into_response());
        }
        if !is_valid_audio_url(url) {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": "audio_url must start with http://, https://, or /audio/"
                })),
            )
                .into_response());
        }
    }
    Ok(())
}

fn has_text(value: &Option<String>) -> bool {
    value.as_deref().is_some_and(|text| !text.trim().is_empty())
}

fn prefer_non_empty(incoming: Option<String>, existing: Option<String>) -> Option<String> {
    incoming
        .filter(|value| !value.trim().is_empty())
        .or(existing)
}

#[derive(Clone, Copy)]
enum MathTextMode {
    Markdown,
    Plain,
}

fn normalize_math_for_markdown(text: &str) -> String {
    normalize_math_fragments(text, MathTextMode::Markdown)
}

fn normalize_math_for_plain_text(text: &str) -> String {
    normalize_math_fragments(text, MathTextMode::Plain)
}

fn remove_orphan_dollar_markers(text: &str) -> String {
    let chars = text.chars().collect::<Vec<_>>();
    let mut cleaned = String::with_capacity(text.len());
    let mut index = 0usize;

    while index < chars.len() {
        if chars[index] == '$' {
            let mut end = index + 1;
            let mut digit_count = 0usize;
            while end < chars.len() && digit_count < 2 && chars[end].is_ascii_digit() {
                end += 1;
                digit_count += 1;
            }

            if digit_count > 0 {
                let next = chars.get(end).copied();
                let next_is_money =
                    next.is_some_and(|ch| ch.is_ascii_digit() || ch == ',' || ch == '.');
                if !next_is_money {
                    let previous = cleaned.chars().last();
                    let previous_is_inline =
                        previous.is_some_and(|ch| !ch.is_whitespace() && ch != '$');
                    let previous_is_boundary = previous
                        .map(|ch| !ch.is_alphanumeric() && ch != '$')
                        .unwrap_or(true);
                    let mut next_significant = end;
                    while chars
                        .get(next_significant)
                        .is_some_and(|ch| ch.is_whitespace())
                    {
                        next_significant += 1;
                    }
                    let followed_by_terminal = chars
                        .get(next_significant)
                        .map(|ch| is_terminal_after_dollar_marker(*ch))
                        .unwrap_or(true);

                    if previous_is_inline || (previous_is_boundary && followed_by_terminal) {
                        index = end;
                        continue;
                    }
                }
            }
        }

        cleaned.push(chars[index]);
        index += 1;
    }

    cleaned
}

fn is_terminal_after_dollar_marker(ch: char) -> bool {
    matches!(
        ch,
        ')' | ']'
            | '}'
            | '。'
            | '！'
            | '？'
            | '；'
            | '，'
            | '、'
            | ','
            | '.'
            | ';'
            | ':'
            | '!'
            | '?'
    )
}

fn normalize_math_fragments(text: &str, mode: MathTextMode) -> String {
    let chars = text.chars().collect::<Vec<_>>();
    let mut normalized = String::with_capacity(text.len());
    let mut index = 0usize;

    while index < chars.len() {
        if chars[index] == '`' {
            let mut end = index + 1;
            while end < chars.len() && chars[end] != '`' {
                end += 1;
            }
            if end < chars.len() {
                normalized.extend(chars[index..=end].iter());
                index = end + 1;
                continue;
            }
        }

        if chars[index] == '$' {
            let is_block = chars.get(index + 1) == Some(&'$');
            let offset = if is_block { 2 } else { 1 };
            let start = index + offset;
            if let Some(end) = find_matching_dollar(&chars, start, is_block) {
                let fragment = chars[start..end].iter().collect::<String>();
                if is_block || looks_like_math_fragment(&fragment) {
                    let cleaned = cleanup_latex_expression(&fragment);
                    if !cleaned.is_empty() {
                        match mode {
                            MathTextMode::Markdown => {
                                normalized.push('`');
                                normalized.push_str(&cleaned);
                                normalized.push('`');
                            }
                            MathTextMode::Plain => normalized.push_str(&cleaned),
                        }
                        index = end + offset;
                        continue;
                    }
                }
            }
        }

        normalized.push(chars[index]);
        index += 1;
    }

    normalized
}

fn find_matching_dollar(chars: &[char], start: usize, is_block: bool) -> Option<usize> {
    let mut index = start;
    while index < chars.len() {
        if chars[index] == '\\' {
            index += 2;
            continue;
        }
        if chars[index] == '$' {
            if is_block {
                if chars.get(index + 1) == Some(&'$') {
                    return Some(index);
                }
            } else {
                return Some(index);
            }
        }
        index += 1;
    }
    None
}

fn looks_like_math_fragment(fragment: &str) -> bool {
    let trimmed = fragment.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.contains('\\')
        || trimmed.contains('{')
        || trimmed.contains('}')
        || trimmed.contains('^')
        || trimmed.contains('_')
    {
        return true;
    }
    if trimmed.chars().all(|ch| ch.is_ascii_alphanumeric()) && trimmed.len() <= 4 {
        return true;
    }
    trimmed
        .chars()
        .any(|ch| matches!(ch, '=' | '<' | '>' | '≤' | '≥' | '≈' | '≲' | '≳'))
}

fn cleanup_latex_expression(fragment: &str) -> String {
    let mut normalized = replace_latex_structures(fragment.trim());
    for (from, to) in [
        ("\\left", ""),
        ("\\right", ""),
        ("\\,", " "),
        ("\\;", " "),
        ("\\:", " "),
        ("\\!", ""),
        ("\\cdot", "·"),
        ("\\times", "×"),
        ("\\approx", "≈"),
        ("\\lesssim", "≲"),
        ("\\gtrsim", "≳"),
        ("\\leq", "≤"),
        ("\\le", "≤"),
        ("\\geq", "≥"),
        ("\\ge", "≥"),
        ("\\neq", "≠"),
        ("\\sim", "~"),
        ("\\pi", "π"),
        ("\\alpha", "α"),
        ("\\beta", "β"),
        ("\\gamma", "γ"),
        ("\\delta", "δ"),
        ("\\theta", "θ"),
        ("\\lambda", "λ"),
        ("\\mu", "μ"),
        ("\\sigma", "σ"),
        ("\\phi", "φ"),
        ("\\omega", "ω"),
        ("\\sin", "sin"),
        ("\\cos", "cos"),
        ("\\tan", "tan"),
        ("\\log", "log"),
        ("\\ln", "ln"),
        ("\\exp", "exp"),
        ("\\min", "min"),
        ("\\max", "max"),
    ] {
        normalized = normalized.replace(from, to);
    }

    let chars = normalized.chars().collect::<Vec<_>>();
    let mut cleaned = String::with_capacity(normalized.len());
    let mut index = 0usize;

    while index < chars.len() {
        if matches_command(&chars, index, "\\operatorname")
            || matches_command(&chars, index, "\\text")
            || matches_command(&chars, index, "\\mathrm")
            || matches_command(&chars, index, "\\mathbb")
            || matches_command(&chars, index, "\\mathbf")
            || matches_command(&chars, index, "\\mathcal")
        {
            let command_len = command_length_at(&chars, index);
            if let Some((group, next_index)) = parse_braced_group(&chars, index + command_len) {
                cleaned.push_str(&cleanup_latex_expression(&group));
                index = next_index;
                continue;
            }
        }

        if chars[index] == '\\' {
            let mut next = index + 1;
            while next < chars.len() && chars[next].is_ascii_alphabetic() {
                next += 1;
            }
            if next > index + 1 {
                cleaned.extend(chars[index + 1..next].iter());
                index = next;
                continue;
            }
            index += 1;
            continue;
        }

        match chars[index] {
            '{' => cleaned.push('('),
            '}' => cleaned.push(')'),
            _ => cleaned.push(chars[index]),
        }
        index += 1;
    }

    cleaned
        .replace('\n', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

fn replace_latex_structures(fragment: &str) -> String {
    let chars = fragment.chars().collect::<Vec<_>>();
    let mut normalized = String::with_capacity(fragment.len());
    let mut index = 0usize;

    while index < chars.len() {
        if matches_command(&chars, index, "\\frac") {
            let start = index + "\\frac".chars().count();
            if let Some((numerator, next_index)) = parse_braced_group(&chars, start) {
                if let Some((denominator, final_index)) = parse_braced_group(&chars, next_index) {
                    normalized.push_str(&cleanup_latex_expression(&numerator));
                    normalized.push_str(" / ");
                    normalized.push_str(&cleanup_latex_expression(&denominator));
                    index = final_index;
                    continue;
                }
            }
        }

        if matches_command(&chars, index, "\\sqrt") {
            let start = index + "\\sqrt".chars().count();
            if let Some((body, next_index)) = parse_braced_group(&chars, start) {
                normalized.push_str("sqrt(");
                normalized.push_str(&cleanup_latex_expression(&body));
                normalized.push(')');
                index = next_index;
                continue;
            }
        }

        normalized.push(chars[index]);
        index += 1;
    }

    normalized
}

fn parse_braced_group(chars: &[char], start: usize) -> Option<(String, usize)> {
    if chars.get(start) != Some(&'{') {
        return None;
    }

    let mut depth = 0usize;
    let mut index = start;
    let mut group = String::new();

    while index < chars.len() {
        let ch = chars[index];
        match ch {
            '{' => {
                depth += 1;
                if depth > 1 {
                    group.push(ch);
                }
            }
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some((group, index + 1));
                }
                group.push(ch);
            }
            _ => group.push(ch),
        }
        index += 1;
    }

    None
}

fn matches_command(chars: &[char], start: usize, command: &str) -> bool {
    let command_chars = command.chars().collect::<Vec<_>>();
    chars.get(start..start + command_chars.len()) == Some(command_chars.as_slice())
}

fn command_length_at(chars: &[char], start: usize) -> usize {
    let mut index = start;
    if chars.get(index) == Some(&'\\') {
        index += 1;
    }
    while chars.get(index).is_some_and(|ch| ch.is_ascii_alphabetic()) {
        index += 1;
    }
    index - start
}

fn normalize_block_text(text: &str, mode: MathTextMode) -> String {
    let normalized = match mode {
        MathTextMode::Markdown => normalize_math_for_markdown(text),
        MathTextMode::Plain => normalize_math_for_plain_text(text),
    };

    remove_orphan_dollar_markers(&normalized)
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .replace("\n\n\n", "\n\n")
        .trim()
        .to_string()
}

fn normalize_key_points_json(value: &Option<String>) -> Option<String> {
    let raw = value.as_deref()?.trim();
    if raw.is_empty() {
        return None;
    }

    match serde_json::from_str::<Vec<String>>(raw) {
        Ok(points) => {
            let points = points
                .into_iter()
                .map(|point| normalize_block_text(&point, MathTextMode::Plain))
                .filter(|point| !point.trim().is_empty())
                .collect::<Vec<_>>();
            serde_json::to_string(&points).ok()
        }
        Err(_) => Some(raw.to_string()),
    }
}

fn normalize_feed_item_content(content: &FeedItemContentPayload) -> FeedItemContentPayload {
    FeedItemContentPayload {
        original_html: content.original_html.clone(),
        reader_markdown: content.reader_markdown.clone(),
        plain_text: content.plain_text.clone(),
        compressed_markdown: content
            .compressed_markdown
            .as_deref()
            .map(|value| normalize_block_text(value, MathTextMode::Markdown)),
        audio_script: content
            .audio_script
            .as_deref()
            .map(|value| normalize_block_text(value, MathTextMode::Plain)),
        key_points_json: normalize_key_points_json(&content.key_points_json),
    }
}

fn normalize_feed_item_content_response(content: FeedItemContent) -> FeedItemContent {
    FeedItemContent {
        compressed_markdown: content
            .compressed_markdown
            .as_deref()
            .map(|value| normalize_block_text(value, MathTextMode::Markdown)),
        audio_script: content
            .audio_script
            .as_deref()
            .map(|value| normalize_block_text(value, MathTextMode::Plain)),
        key_points_json: normalize_key_points_json(&content.key_points_json),
        ..content
    }
}

fn normalize_weekly_digest(digest: WeeklyDigest) -> WeeklyDigest {
    WeeklyDigest {
        digest_markdown: digest
            .digest_markdown
            .as_deref()
            .map(|value| normalize_block_text(value, MathTextMode::Markdown)),
        audio_script: digest
            .audio_script
            .as_deref()
            .map(|value| normalize_block_text(value, MathTextMode::Plain)),
        ..digest
    }
}

fn validate_content(
    content: &Option<FeedItemContentPayload>,
) -> Result<(), axum::response::Response> {
    let Some(content) = content else {
        return Ok(());
    };

    for (field, value) in [
        ("original_html", &content.original_html),
        ("reader_markdown", &content.reader_markdown),
        ("plain_text", &content.plain_text),
        ("compressed_markdown", &content.compressed_markdown),
        ("audio_script", &content.audio_script),
        ("key_points_json", &content.key_points_json),
    ] {
        if value
            .as_ref()
            .is_some_and(|body| body.len() > MAX_BODY_LENGTH)
        {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(
                    json!({ "error": format!("{} exceeds {} characters", field, MAX_BODY_LENGTH) }),
                ),
            )
                .into_response());
        }
    }

    Ok(())
}

fn validate_feed_item(payload: &CreateFeedItemRequest) -> Result<(), axum::response::Response> {
    if payload.title.trim().is_empty() || payload.title.len() > MAX_TITLE_LENGTH {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": format!("title must be 1-{} characters", MAX_TITLE_LENGTH) })),
        )
            .into_response());
    }

    let product_line = payload.product_line.as_deref().unwrap_or("curated_feed");
    validate_label(product_line, &["radio", "curated_feed"], "product_line")?;
    validate_label(
        &payload.item_type,
        &[
            "article",
            "compressed_article",
            "audio_episode",
            "weekly_digest",
        ],
        "item_type",
    )?;
    validate_label(&payload.primary_mode, &["read", "listen"], "primary_mode")?;
    validate_optional_url(&payload.source_url, "source_url")?;
    validate_optional_url(&payload.original_url, "original_url")?;
    validate_optional_url(&payload.canonical_url, "canonical_url")?;
    validate_optional_audio_url(&payload.audio_url)?;
    if payload.duration_sec.is_some_and(|duration| duration < 0) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "duration_sec must be non-negative" })),
        )
            .into_response());
    }
    validate_content(&payload.content)?;

    Ok(())
}

async fn upsert_content(
    db: &SqlitePool,
    item_id: &str,
    content: &FeedItemContentPayload,
    now: i64,
) -> Result<(), sqlx::Error> {
    let content = normalize_feed_item_content(content);
    sqlx::query(
        r#"
        INSERT INTO feed_item_contents (
            item_id, original_html, reader_markdown, plain_text, compressed_markdown,
            audio_script, key_points_json, created_at, updated_at
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(item_id) DO UPDATE SET
            original_html = excluded.original_html,
            reader_markdown = excluded.reader_markdown,
            plain_text = excluded.plain_text,
            compressed_markdown = excluded.compressed_markdown,
            audio_script = excluded.audio_script,
            key_points_json = excluded.key_points_json,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(item_id)
    .bind(&content.original_html)
    .bind(&content.reader_markdown)
    .bind(&content.plain_text)
    .bind(&content.compressed_markdown)
    .bind(&content.audio_script)
    .bind(&content.key_points_json)
    .bind(now)
    .bind(now)
    .execute(db)
    .await?;

    Ok(())
}

pub async fn list_feed_items(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(pagination): Query<FeedPagination>,
) -> impl IntoResponse {
    let requested_user = headers
        .get("x-user-id")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let (limit, offset) = sanitize_pagination(pagination.page, pagination.limit);
    let (query_limit, query_offset) = if requested_user.is_some() {
        (
            personalization::recommended_candidate_limit(limit, offset),
            0,
        )
    } else {
        (limit, offset)
    };
    let product_line = pagination
        .product_line
        .unwrap_or_else(|| "curated_feed".to_string());
    let item_type = pagination.item_type;
    let primary_mode = pagination.primary_mode;

    let result = match (item_type, primary_mode) {
        (Some(item_type), Some(primary_mode)) => {
            sqlx::query_as::<_, FeedItem>(
                r#"
                SELECT * FROM feed_items
                WHERE product_line = ? AND item_type = ? AND primary_mode = ? AND status = 'published'
                ORDER BY publish_time DESC, created_at DESC
                LIMIT ? OFFSET ?
                "#,
            )
            .bind(product_line)
            .bind(item_type)
            .bind(primary_mode)
            .bind(query_limit)
            .bind(query_offset)
            .fetch_all(&state.db)
            .await
        }
        (Some(item_type), None) => {
            sqlx::query_as::<_, FeedItem>(
                r#"
                SELECT * FROM feed_items
                WHERE product_line = ? AND item_type = ? AND status = 'published'
                ORDER BY publish_time DESC, created_at DESC
                LIMIT ? OFFSET ?
                "#,
            )
            .bind(product_line)
            .bind(item_type)
            .bind(query_limit)
            .bind(query_offset)
            .fetch_all(&state.db)
            .await
        }
        (None, Some(primary_mode)) => {
            sqlx::query_as::<_, FeedItem>(
                r#"
                SELECT * FROM feed_items
                WHERE product_line = ? AND primary_mode = ? AND status = 'published'
                ORDER BY publish_time DESC, created_at DESC
                LIMIT ? OFFSET ?
                "#,
            )
            .bind(product_line)
            .bind(primary_mode)
            .bind(query_limit)
            .bind(query_offset)
            .fetch_all(&state.db)
            .await
        }
        (None, None) => {
            sqlx::query_as::<_, FeedItem>(
                r#"
                SELECT * FROM feed_items
                WHERE product_line = ? AND status = 'published'
                ORDER BY publish_time DESC, created_at DESC
                LIMIT ? OFFSET ?
                "#,
            )
            .bind(product_line)
            .bind(query_limit)
            .bind(query_offset)
            .fetch_all(&state.db)
            .await
        }
    };

    match result {
        Ok(items) => {
            let items = if let Some(user_id) = requested_user.as_deref() {
                let fallback_items = items.clone();
                match personalization::personalize_feed_items(&state, user_id, items).await {
                    Ok(items) => items,
                    Err(_) => fallback_items,
                }
            } else {
                items
            };
            let items = if requested_user.is_some() {
                items
                    .into_iter()
                    .skip(offset as usize)
                    .take(limit as usize)
                    .collect::<Vec<_>>()
            } else {
                items
            };
            Json(items).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub async fn get_feed_item(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let result = sqlx::query_as::<_, FeedItem>("SELECT * FROM feed_items WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.db)
        .await;

    match result {
        Ok(Some(item)) => Json(item).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub async fn get_feed_item_why(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let Some(user_id) = headers
        .get("x-user-id")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };

    let result = sqlx::query_as::<_, FeedItem>("SELECT * FROM feed_items WHERE id = ?")
        .bind(&id)
        .fetch_optional(&state.db)
        .await;

    match result {
        Ok(Some(item)) => match personalization::explain_feed_item(&state, user_id, &item).await {
            Ok(response) => Json(response).into_response(),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        },
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub async fn get_feed_item_content(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let result =
        sqlx::query_as::<_, FeedItemContent>("SELECT * FROM feed_item_contents WHERE item_id = ?")
            .bind(id)
            .fetch_optional(&state.db)
            .await;

    match result {
        Ok(Some(content)) => Json(normalize_feed_item_content_response(content)).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub async fn create_feed_item(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateFeedItemRequest>,
) -> impl IntoResponse {
    if !has_internal_auth(&headers, &state) {
        return (StatusCode::UNAUTHORIZED, "Invalid API Key").into_response();
    }
    if let Err(response) = validate_feed_item(&payload) {
        return response;
    }

    let id = payload
        .id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let now = chrono::Utc::now().timestamp();
    let product_line = payload
        .product_line
        .clone()
        .unwrap_or_else(|| "curated_feed".to_string());
    let status = payload
        .status
        .clone()
        .unwrap_or_else(|| "published".to_string());
    let has_audio = payload.has_audio.unwrap_or_else(|| {
        payload
            .audio_url
            .as_ref()
            .is_some_and(|url| !url.is_empty())
    });
    let clear_audio = payload.clear_audio.unwrap_or(false);

    let result = sqlx::query(
        r#"
        INSERT OR IGNORE INTO feed_items (
            id, product_line, item_type, primary_mode, title, subtitle, source_name,
            source_url, original_url, canonical_url, content_hash, publish_time,
            created_at, updated_at, has_audio, audio_url, reading_time_min,
            duration_sec, quality_score, tags, status
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&id)
    .bind(&product_line)
    .bind(&payload.item_type)
    .bind(&payload.primary_mode)
    .bind(&payload.title)
    .bind(&payload.subtitle)
    .bind(&payload.source_name)
    .bind(&payload.source_url)
    .bind(&payload.original_url)
    .bind(&payload.canonical_url)
    .bind(&payload.content_hash)
    .bind(payload.publish_time)
    .bind(now)
    .bind(now)
    .bind(has_audio)
    .bind(&payload.audio_url)
    .bind(payload.reading_time_min)
    .bind(payload.duration_sec)
    .bind(payload.quality_score)
    .bind(&payload.tags)
    .bind(&status)
    .execute(&state.db)
    .await;

    match result {
        Ok(result) if result.rows_affected() > 0 => {
            if let Some(content) = &payload.content {
                if let Err(e) = upsert_content(&state.db, &id, content, now).await {
                    tracing::error!("Failed to insert feed item content {}: {}", id, e);
                    return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
                }
            }
            Json(json!({ "id": id, "status": "created" })).into_response()
        }
        Ok(_) => {
            let existing_id = if let Some(original_url) = payload.original_url.as_deref() {
                sqlx::query_scalar::<_, String>(
                    "SELECT id FROM feed_items WHERE original_url = ? OR id = ? LIMIT 1",
                )
                .bind(original_url)
                .bind(&id)
                .fetch_optional(&state.db)
                .await
                .ok()
                .flatten()
            } else {
                sqlx::query_scalar::<_, String>("SELECT id FROM feed_items WHERE id = ? LIMIT 1")
                    .bind(&id)
                    .fetch_optional(&state.db)
                    .await
                    .ok()
                    .flatten()
            };

            let Some(existing_id) = existing_id else {
                return Json(json!({
                    "id": "skipped",
                    "status": "skipped_duplicate"
                }))
                .into_response();
            };

            let update_result = sqlx::query(
                r#"
                UPDATE feed_items
                SET title = ?,
                    subtitle = COALESCE(NULLIF(?, ''), subtitle),
                    source_name = COALESCE(NULLIF(?, ''), source_name),
                    source_url = COALESCE(NULLIF(?, ''), source_url),
                    original_url = COALESCE(NULLIF(?, ''), original_url),
                    canonical_url = COALESCE(NULLIF(?, ''), canonical_url),
                    content_hash = COALESCE(NULLIF(?, ''), content_hash),
                    publish_time = COALESCE(?, publish_time),
                    updated_at = ?,
                    has_audio = CASE WHEN ? THEN 0 WHEN ? THEN 1 ELSE has_audio END,
                    audio_url = CASE WHEN ? THEN NULL ELSE COALESCE(NULLIF(?, ''), audio_url) END,
                    reading_time_min = COALESCE(?, reading_time_min),
                    duration_sec = CASE WHEN ? THEN NULL ELSE COALESCE(?, duration_sec) END,
                    quality_score = COALESCE(?, quality_score),
                    tags = COALESCE(NULLIF(?, ''), tags),
                    status = COALESCE(NULLIF(?, ''), status)
                WHERE id = ?
                "#,
            )
            .bind(&payload.title)
            .bind(&payload.subtitle)
            .bind(&payload.source_name)
            .bind(&payload.source_url)
            .bind(&payload.original_url)
            .bind(&payload.canonical_url)
            .bind(&payload.content_hash)
            .bind(payload.publish_time)
            .bind(now)
            .bind(clear_audio)
            .bind(has_audio)
            .bind(clear_audio)
            .bind(&payload.audio_url)
            .bind(payload.reading_time_min)
            .bind(clear_audio)
            .bind(payload.duration_sec)
            .bind(payload.quality_score)
            .bind(&payload.tags)
            .bind(&status)
            .bind(&existing_id)
            .execute(&state.db)
            .await;

            if let Err(e) = update_result {
                tracing::error!("Feed item update failed {}: {}", existing_id, e);
                return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
            }

            if let Some(content) = &payload.content {
                if let Err(e) = upsert_content(&state.db, &existing_id, content, now).await {
                    tracing::error!("Failed to upsert feed item content {}: {}", existing_id, e);
                    return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
                }
            }

            Json(json!({
                "id": existing_id,
                "status": "updated"
            }))
            .into_response()
        }
        Err(e) => {
            tracing::error!("Feed item insert failed: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}

pub async fn list_weekly_digests(State(state): State<AppState>) -> impl IntoResponse {
    let result = sqlx::query_as::<_, WeeklyDigest>(
        "SELECT * FROM weekly_digests WHERE status = 'published' ORDER BY week_start DESC LIMIT 52",
    )
    .fetch_all(&state.db)
    .await;

    match result {
        Ok(digests) => Json(
            digests
                .into_iter()
                .map(normalize_weekly_digest)
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub async fn get_weekly_digest(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let result = sqlx::query_as::<_, WeeklyDigest>("SELECT * FROM weekly_digests WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.db)
        .await;

    match result {
        Ok(Some(digest)) => Json(normalize_weekly_digest(digest)).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub async fn create_weekly_digest(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateWeeklyDigestRequest>,
) -> impl IntoResponse {
    if !has_internal_auth(&headers, &state) {
        return (StatusCode::UNAUTHORIZED, "Invalid API Key").into_response();
    }
    if payload.title.trim().is_empty() || payload.title.len() > MAX_TITLE_LENGTH {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": format!("title must be 1-{} characters", MAX_TITLE_LENGTH) })),
        )
            .into_response();
    }
    if payload.week_end < payload.week_start {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "week_end must be greater than or equal to week_start" })),
        )
            .into_response();
    }
    if let Err(response) = validate_optional_audio_url(&payload.audio_url) {
        return response;
    }
    if payload.duration_sec.is_some_and(|duration| duration < 0) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "duration_sec must be non-negative" })),
        )
            .into_response();
    }

    let digest_id = payload
        .id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let now = chrono::Utc::now().timestamp();
    let status = payload
        .status
        .clone()
        .unwrap_or_else(|| "published".to_string());
    let existing = sqlx::query_as::<_, WeeklyDigest>(
        r#"
        SELECT * FROM weekly_digests
        WHERE id = ? OR (week_start = ? AND week_end = ?)
        LIMIT 1
        "#,
    )
    .bind(&digest_id)
    .bind(payload.week_start)
    .bind(payload.week_end)
    .fetch_optional(&state.db)
    .await;

    match existing {
        Ok(Some(digest)) => {
            let next_digest_markdown = prefer_non_empty(
                payload.digest_markdown.clone(),
                digest.digest_markdown.clone(),
            )
            .map(|value| normalize_block_text(&value, MathTextMode::Markdown));
            let next_audio_script =
                prefer_non_empty(payload.audio_script.clone(), digest.audio_script.clone())
                    .map(|value| normalize_block_text(&value, MathTextMode::Plain));
            let next_audio_url =
                prefer_non_empty(payload.audio_url.clone(), digest.audio_url.clone());
            let next_duration_sec = payload.duration_sec.or(digest.duration_sec);
            let next_included_item_ids_json = prefer_non_empty(
                payload.included_item_ids_json.clone(),
                digest.included_item_ids_json.clone(),
            );
            let next_themes_json =
                prefer_non_empty(payload.themes_json.clone(), digest.themes_json.clone());
            let next_status = payload
                .status
                .clone()
                .or(digest.status.clone())
                .unwrap_or_else(|| "published".to_string());
            let has_audio = has_text(&next_audio_url);

            if let Err(e) = sqlx::query(
                r#"
                UPDATE weekly_digests
                SET title = ?, digest_markdown = ?, audio_script = ?, audio_url = ?,
                    duration_sec = ?, included_item_ids_json = ?, themes_json = ?, status = ?
                WHERE id = ?
                "#,
            )
            .bind(&payload.title)
            .bind(&next_digest_markdown)
            .bind(&next_audio_script)
            .bind(&next_audio_url)
            .bind(next_duration_sec)
            .bind(&next_included_item_ids_json)
            .bind(&next_themes_json)
            .bind(&next_status)
            .bind(&digest.id)
            .execute(&state.db)
            .await
            {
                return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
            }

            if let Some(feed_item_id) = digest.feed_item_id.as_deref() {
                if let Err(e) = sqlx::query(
                    r#"
                    UPDATE feed_items
                    SET title = ?, updated_at = ?, has_audio = ?, audio_url = ?,
                        duration_sec = ?, status = ?
                    WHERE id = ?
                    "#,
                )
                .bind(&payload.title)
                .bind(now)
                .bind(has_audio)
                .bind(&next_audio_url)
                .bind(next_duration_sec)
                .bind(&next_status)
                .bind(feed_item_id)
                .execute(&state.db)
                .await
                {
                    return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
                }
            }

            return Json(json!({
                "id": digest.id,
                "feed_item_id": digest.feed_item_id,
                "status": "updated"
            }))
            .into_response();
        }
        Ok(None) => {}
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }

    let feed_item_id = payload
        .feed_item_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let has_audio = payload
        .audio_url
        .as_ref()
        .is_some_and(|url| !url.is_empty());

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };

    let feed_result = sqlx::query(
        r#"
        INSERT OR IGNORE INTO feed_items (
            id, product_line, item_type, primary_mode, title, subtitle, source_name,
            source_url, original_url, canonical_url, content_hash, publish_time,
            created_at, updated_at, has_audio, audio_url, reading_time_min,
            duration_sec, quality_score, tags, status
        )
        VALUES (?, 'curated_feed', 'weekly_digest', 'listen', ?, ?, 'FreshLoop', NULL,
            NULL, NULL, NULL, ?, ?, ?, ?, ?, NULL, ?, NULL, NULL, ?)
        "#,
    )
    .bind(&feed_item_id)
    .bind(&payload.title)
    .bind(Some(format!(
        "{} - {}",
        payload.week_start, payload.week_end
    )))
    .bind(payload.week_end)
    .bind(now)
    .bind(now)
    .bind(has_audio)
    .bind(&payload.audio_url)
    .bind(payload.duration_sec)
    .bind(&status)
    .execute(&mut *tx)
    .await;

    if let Err(e) = feed_result {
        let _ = tx.rollback().await;
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }

    let digest_result = sqlx::query(
        r#"
        INSERT OR IGNORE INTO weekly_digests (
            id, feed_item_id, week_start, week_end, title, digest_markdown,
            audio_script, audio_url, duration_sec, included_item_ids_json, themes_json,
            created_at, status
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&digest_id)
    .bind(&feed_item_id)
    .bind(payload.week_start)
    .bind(payload.week_end)
    .bind(&payload.title)
    .bind(
        &payload
            .digest_markdown
            .as_deref()
            .map(|value| normalize_block_text(value, MathTextMode::Markdown)),
    )
    .bind(
        &payload
            .audio_script
            .as_deref()
            .map(|value| normalize_block_text(value, MathTextMode::Plain)),
    )
    .bind(&payload.audio_url)
    .bind(payload.duration_sec)
    .bind(&payload.included_item_ids_json)
    .bind(&payload.themes_json)
    .bind(now)
    .bind(&status)
    .execute(&mut *tx)
    .await;

    match digest_result {
        Ok(result) => {
            if let Err(e) = tx.commit().await {
                return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
            }
            let status = if result.rows_affected() > 0 {
                "created"
            } else {
                "skipped_duplicate"
            };
            Json(json!({ "id": digest_id, "feed_item_id": feed_item_id, "status": status }))
                .into_response()
        }
        Err(e) => {
            let _ = tx.rollback().await;
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pagination_is_clamped_to_supported_range() {
        assert_eq!(sanitize_pagination(Some(-4), Some(500)), (100, 0));
        assert_eq!(sanitize_pagination(Some(3), Some(10)), (10, 20));
        assert_eq!(sanitize_pagination(None, Some(0)), (1, 0));
    }

    #[test]
    fn audio_url_accepts_remote_and_local_audio_paths() {
        assert!(is_valid_audio_url(
            "https://news.hackerlife.fun/audio/a.mp3"
        ));
        assert!(is_valid_audio_url("http://localhost:8899/audio/a.mp3"));
        assert!(is_valid_audio_url("/audio/a.mp3"));
        assert!(!is_valid_audio_url("/api/items"));
        assert!(!is_valid_audio_url("file:///tmp/a.mp3"));
    }

    #[test]
    fn feed_item_validation_allows_internal_audio_urls() {
        let payload = CreateFeedItemRequest {
            id: None,
            product_line: Some("curated_feed".to_string()),
            item_type: "article".to_string(),
            primary_mode: "read".to_string(),
            title: "A useful article".to_string(),
            subtitle: None,
            source_name: None,
            source_url: Some("https://example.com/feed".to_string()),
            original_url: Some("https://example.com/post".to_string()),
            canonical_url: Some("https://example.com/post".to_string()),
            content_hash: None,
            publish_time: None,
            has_audio: Some(true),
            audio_url: Some("/audio/test.mp3".to_string()),
            clear_audio: None,
            duration_sec: Some(42),
            reading_time_min: Some(3),
            quality_score: Some(8),
            tags: None,
            status: Some("published".to_string()),
            content: None,
        };

        assert!(validate_feed_item(&payload).is_ok());
    }

    #[test]
    fn normalizes_latex_math_without_touching_currency_text() {
        let markdown = "拉普拉斯极限 $e \\approx 0.6627$；标题保留 StubZero: $148,337 RCE。";
        let normalized_markdown = normalize_block_text(markdown, MathTextMode::Markdown);
        assert!(normalized_markdown.contains("`e ≈ 0.6627`"));
        assert!(normalized_markdown.contains("StubZero: $148,337 RCE"));
        assert!(!normalized_markdown.contains("$e"));

        let plain = normalize_block_text(markdown, MathTextMode::Plain);
        assert!(plain.contains("e ≈ 0.6627"));
        assert!(plain.contains("StubZero: $148,337 RCE"));
        assert!(!plain.contains("`e"));
    }

    #[test]
    fn normalizes_content_payload_math_fragments() {
        let content = FeedItemContentPayload {
            original_html: None,
            reader_markdown: None,
            plain_text: None,
            compressed_markdown: Some("公式 $M = E - e \\sin E$".to_string()),
            audio_script: Some("收听版讲 $e \\lesssim 0.6627$".to_string()),
            key_points_json: Some(
                serde_json::to_string(&vec!["边界 $e$".to_string()]).unwrap_or_default(),
            ),
        };

        let normalized = normalize_feed_item_content(&content);
        assert_eq!(
            normalized.compressed_markdown.as_deref(),
            Some("公式 `M = E - e sin E`")
        );
        assert_eq!(
            normalized.audio_script.as_deref(),
            Some("收听版讲 e ≲ 0.6627")
        );
        assert_eq!(normalized.key_points_json.as_deref(), Some("[\"边界 e\"]"));
    }

    #[test]
    fn removes_orphan_capture_markers_but_keeps_money() {
        let text = "这里$1有残片，括号（$2）。价格 $148,337、$1.99 和 $1 per month 保留。";
        let normalized = normalize_block_text(text, MathTextMode::Markdown);

        assert!(normalized.contains("这里有残片"));
        assert!(normalized.contains("括号（）。"));
        assert!(normalized.contains("$148,337"));
        assert!(normalized.contains("$1.99"));
        assert!(normalized.contains("$1 per month"));
        assert!(!normalized.contains("这里$1"));
    }
}

pub async fn update_reading_progress(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(payload): Json<UpdateReadingProgressRequest>,
) -> impl IntoResponse {
    let user_id = headers
        .get("x-user-id")
        .and_then(|value| value.to_str().ok());
    let Some(user_id) = user_id else {
        return StatusCode::OK.into_response();
    };

    let mode = payload.mode.unwrap_or_else(|| "original".to_string());
    if !["original", "compressed"].contains(&mode.as_str()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "mode must be original or compressed" })),
        )
            .into_response();
    }

    let scroll_ratio = payload.scroll_ratio.map(|ratio| ratio.clamp(0.0, 1.0));
    let now = chrono::Utc::now().timestamp();

    let result = sqlx::query(
        r#"
        INSERT OR REPLACE INTO feed_reading_progress (
            user_id, item_id, mode, scroll_ratio, anchor, updated_at, read_at
        )
        VALUES (?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(user_id)
    .bind(&id)
    .bind(mode)
    .bind(scroll_ratio)
    .bind(&payload.anchor)
    .bind(now)
    .bind(payload.read_at)
    .execute(&state.db)
    .await;

    match result {
        Ok(_) => StatusCode::OK.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}
