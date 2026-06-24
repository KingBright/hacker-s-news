use crate::AppState;
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json},
};
use loop_memory::{MemoryEntry, MemoryStore, MemoryType, Provenance};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

const DEFAULT_LIST_LIMIT: i64 = 40;
const MAX_LIST_LIMIT: i64 = 100;
const MAX_TITLE_LENGTH: usize = 500;
const MAX_BODY_LENGTH: usize = 50_000;
const MAX_QUOTE_LENGTH: usize = 20_000;
const MAX_URL_LENGTH: usize = 2048;
const MAX_SOURCE_ID_LENGTH: usize = 512;
const MAX_SOURCE_REF_LENGTH: usize = 1024;
const MAX_REFERENCES_PER_POST: usize = 20;

const ALLOWED_POST_TYPES: &[&str] = &[
    "thought",
    "quote_comment",
    "excerpt",
    "reflection",
    "observation",
];
const ALLOWED_VISIBILITY: &[&str] = &["private", "unlisted", "public"];
const ALLOWED_STATUS: &[&str] = &["published", "archived", "deleted"];
const ALLOWED_PREFERENCE_STATUS: &[&str] = &["pending", "processed", "skipped", "failed"];
const ALLOWED_FEEDBACK_MODES: &[&str] = &["balance", "boost", "reduce", "observe"];
const ALLOWED_SOURCE_TYPES: &[&str] = &[
    "article",
    "daily_brief",
    "weekly_digest",
    "radio_item",
    "audio_offset",
    "external_url",
    "loop_post",
];

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct LoopPost {
    pub id: String,
    pub user_id: String,
    pub post_type: String,
    pub feedback_mode: Option<String>,
    pub title: Option<String>,
    pub body: String,
    pub visibility: String,
    pub source_ref: Option<String>,
    pub memory_entry_id: Option<String>,
    pub preference_status: Option<String>,
    pub preference_extracted_at: Option<i64>,
    pub preference_error: Option<String>,
    pub created_at: Option<i64>,
    pub updated_at: Option<i64>,
    pub status: Option<String>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct LoopPostReference {
    pub id: String,
    pub post_id: String,
    pub source_type: String,
    pub source_id: Option<String>,
    pub source_url: Option<String>,
    pub title: Option<String>,
    pub quote_text: Option<String>,
    pub start_ms: Option<i64>,
    pub end_ms: Option<i64>,
    pub created_at: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct LoopPostResponse {
    #[serde(flatten)]
    pub post: LoopPost,
    pub references: Vec<LoopPostReference>,
}

#[derive(Debug, Serialize)]
pub struct InternalLoopPostResponse {
    #[serde(flatten)]
    pub post: LoopPost,
    pub references: Vec<LoopPostReference>,
}

#[derive(Debug, Deserialize)]
pub struct CreateLoopPostReferenceRequest {
    pub source_type: String,
    pub source_id: Option<String>,
    pub source_url: Option<String>,
    pub title: Option<String>,
    pub quote_text: Option<String>,
    pub start_ms: Option<i64>,
    pub end_ms: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct CreateLoopPostRequest {
    pub post_type: Option<String>,
    pub feedback_mode: Option<String>,
    pub title: Option<String>,
    pub body: String,
    pub visibility: Option<String>,
    pub source_ref: Option<String>,
    pub references: Option<Vec<CreateLoopPostReferenceRequest>>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateLoopPostRequest {
    pub feedback_mode: Option<String>,
    pub title: Option<String>,
    pub body: Option<String>,
    pub visibility: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LoopPostListQuery {
    pub limit: Option<i64>,
    pub include_deleted: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct InternalPendingLoopPostQuery {
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct InternalPreferenceResultRequest {
    pub status: String,
    pub error: Option<String>,
}

fn user_id_from_headers(headers: &HeaderMap) -> Result<String, StatusCode> {
    let Some(user_id) = headers
        .get("x-user-id")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Err(StatusCode::UNAUTHORIZED);
    };
    Ok(user_id.to_string())
}

fn user_namespace(user_id: &str) -> String {
    format!("user:{}", user_id)
}

fn has_internal_auth(headers: &HeaderMap, state: &AppState) -> bool {
    headers
        .get("X-NEXUS-KEY")
        .and_then(|value| value.to_str().ok())
        == Some(state.api_key.as_str())
}

fn sanitize_limit(limit: Option<i64>) -> i64 {
    limit.unwrap_or(DEFAULT_LIST_LIMIT).clamp(1, MAX_LIST_LIMIT)
}

fn normalize_optional_text(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn validate_label(value: &str, allowed: &[&str], field: &str) -> Result<(), String> {
    if allowed.contains(&value) {
        Ok(())
    } else {
        Err(format!("invalid {} '{}'", field, value))
    }
}

fn is_valid_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

fn validate_reference(reference: &CreateLoopPostReferenceRequest) -> Result<(), String> {
    validate_label(&reference.source_type, ALLOWED_SOURCE_TYPES, "source_type")?;
    if reference
        .source_id
        .as_deref()
        .is_some_and(|source_id| source_id.chars().count() > MAX_SOURCE_ID_LENGTH)
    {
        return Err(format!(
            "source_id exceeds {} characters",
            MAX_SOURCE_ID_LENGTH
        ));
    }
    if let Some(url) = reference
        .source_url
        .as_deref()
        .filter(|url| !url.is_empty())
    {
        if url.len() > MAX_URL_LENGTH {
            return Err(format!("source_url exceeds {} characters", MAX_URL_LENGTH));
        }
        if !is_valid_url(url) {
            return Err("source_url must start with http:// or https://".to_string());
        }
    }
    if reference
        .title
        .as_deref()
        .is_some_and(|title| title.chars().count() > MAX_TITLE_LENGTH)
    {
        return Err(format!(
            "reference title exceeds {} characters",
            MAX_TITLE_LENGTH
        ));
    }
    if reference
        .quote_text
        .as_deref()
        .is_some_and(|quote| quote.chars().count() > MAX_QUOTE_LENGTH)
    {
        return Err(format!(
            "quote_text exceeds {} characters",
            MAX_QUOTE_LENGTH
        ));
    }
    if let (Some(start), Some(end)) = (reference.start_ms, reference.end_ms) {
        if end < start {
            return Err("end_ms must be greater than or equal to start_ms".to_string());
        }
    }
    Ok(())
}

fn validate_create_payload(payload: &CreateLoopPostRequest) -> Result<(), String> {
    let post_type = payload.post_type.as_deref().unwrap_or("thought");
    validate_label(post_type, ALLOWED_POST_TYPES, "post_type")?;
    let feedback_mode = payload.feedback_mode.as_deref().unwrap_or("balance");
    validate_label(feedback_mode, ALLOWED_FEEDBACK_MODES, "feedback_mode")?;

    let visibility = payload.visibility.as_deref().unwrap_or("private");
    validate_label(visibility, ALLOWED_VISIBILITY, "visibility")?;

    let body = payload.body.trim();
    if body.is_empty() {
        return Err("body is required".to_string());
    }
    if body.chars().count() > MAX_BODY_LENGTH {
        return Err(format!("body exceeds {} characters", MAX_BODY_LENGTH));
    }
    if payload
        .title
        .as_deref()
        .is_some_and(|title| title.chars().count() > MAX_TITLE_LENGTH)
    {
        return Err(format!("title exceeds {} characters", MAX_TITLE_LENGTH));
    }
    if payload
        .source_ref
        .as_deref()
        .is_some_and(|source_ref| source_ref.chars().count() > MAX_SOURCE_REF_LENGTH)
    {
        return Err(format!(
            "source_ref exceeds {} characters",
            MAX_SOURCE_REF_LENGTH
        ));
    }
    if let Some(references) = &payload.references {
        if references.len() > MAX_REFERENCES_PER_POST {
            return Err(format!(
                "references exceeds {} items",
                MAX_REFERENCES_PER_POST
            ));
        }
        for reference in references {
            validate_reference(reference)?;
        }
    }
    Ok(())
}

fn validate_update_payload(payload: &UpdateLoopPostRequest) -> Result<(), String> {
    if let Some(feedback_mode) = payload.feedback_mode.as_deref() {
        validate_label(feedback_mode, ALLOWED_FEEDBACK_MODES, "feedback_mode")?;
    }
    if let Some(visibility) = payload.visibility.as_deref() {
        validate_label(visibility, ALLOWED_VISIBILITY, "visibility")?;
    }
    if let Some(status) = payload.status.as_deref() {
        validate_label(status, ALLOWED_STATUS, "status")?;
    }
    if let Some(body) = payload.body.as_deref() {
        if body.trim().is_empty() {
            return Err("body cannot be empty".to_string());
        }
        if body.chars().count() > MAX_BODY_LENGTH {
            return Err(format!("body exceeds {} characters", MAX_BODY_LENGTH));
        }
    }
    if payload
        .title
        .as_deref()
        .is_some_and(|title| title.chars().count() > MAX_TITLE_LENGTH)
    {
        return Err(format!("title exceeds {} characters", MAX_TITLE_LENGTH));
    }
    Ok(())
}

fn build_memory_content(post: &LoopPost, references: &[LoopPostReference]) -> String {
    let mut content = format!(
        "[Loop Post]\nType: {}\nFeedback Mode: {}\nVisibility: {}\nBody:\n{}",
        post.post_type,
        post.feedback_mode.as_deref().unwrap_or("balance"),
        post.visibility,
        post.body
    );

    if let Some(title) = post.title.as_deref().filter(|title| !title.is_empty()) {
        content.insert_str(0, &format!("Title: {}\n", title));
    }
    if !references.is_empty() {
        content.push_str("\n\nReferences:");
        for reference in references {
            content.push_str(&format!("\n- {}", reference.source_type));
            if let Some(title) = reference.title.as_deref().filter(|title| !title.is_empty()) {
                content.push_str(&format!(": {}", title));
            }
            if let Some(source_id) = reference
                .source_id
                .as_deref()
                .filter(|source_id| !source_id.is_empty())
            {
                content.push_str(&format!(" ({})", source_id));
            }
            if let Some(quote) = reference
                .quote_text
                .as_deref()
                .filter(|quote| !quote.is_empty())
            {
                content.push_str(&format!("\n  Quote: {}", quote));
            }
        }
    }
    content
}

async fn fetch_references(
    state: &AppState,
    post_id: &str,
) -> Result<Vec<LoopPostReference>, sqlx::Error> {
    sqlx::query_as::<_, LoopPostReference>(
        r#"
        SELECT id, post_id, source_type, source_id, source_url, title, quote_text,
               start_ms, end_ms, created_at
        FROM loop_post_references
        WHERE post_id = ?
        ORDER BY created_at ASC
        "#,
    )
    .bind(post_id)
    .fetch_all(&state.db)
    .await
}

async fn fetch_user_post(
    state: &AppState,
    user_id: &str,
    post_id: &str,
) -> Result<Option<LoopPost>, sqlx::Error> {
    sqlx::query_as::<_, LoopPost>(
        r#"
        SELECT id, user_id, post_type, feedback_mode, title, body, visibility, source_ref,
               memory_entry_id, preference_status, preference_extracted_at, preference_error,
               created_at, updated_at, status
        FROM loop_posts
        WHERE id = ? AND user_id = ?
        "#,
    )
    .bind(post_id)
    .bind(user_id)
    .fetch_optional(&state.db)
    .await
}

async fn fetch_post_by_id(
    state: &AppState,
    post_id: &str,
) -> Result<Option<LoopPost>, sqlx::Error> {
    sqlx::query_as::<_, LoopPost>(
        r#"
        SELECT id, user_id, post_type, feedback_mode, title, body, visibility, source_ref,
               memory_entry_id, preference_status, preference_extracted_at, preference_error,
               created_at, updated_at, status
        FROM loop_posts
        WHERE id = ?
        "#,
    )
    .bind(post_id)
    .fetch_optional(&state.db)
    .await
}

async fn store_memory_for_post(
    state: &AppState,
    post: &LoopPost,
    references: &[LoopPostReference],
) -> Option<String> {
    let now = chrono::Utc::now().timestamp().max(0) as u64;
    let memory_id = Uuid::new_v4().to_string();
    let mut entry = MemoryEntry::new(
        memory_id.clone(),
        MemoryType::UserExpression,
        build_memory_content(post, references),
        now,
        None,
    );
    entry.namespace = Some(user_namespace(&post.user_id));
    entry.provenance = Provenance::UserExplicit;
    entry.source_ref = Some(format!("loop_post:{}", post.id));
    entry
        .metadata
        .insert("post_type".to_string(), post.post_type.clone());
    entry.metadata.insert(
        "feedback_mode".to_string(),
        post.feedback_mode
            .clone()
            .unwrap_or_else(|| "balance".to_string()),
    );
    entry
        .metadata
        .insert("visibility".to_string(), post.visibility.clone());
    entry
        .metadata
        .insert("reference_count".to_string(), references.len().to_string());
    if let Some(source_ref) = &post.source_ref {
        entry
            .metadata
            .insert("source_ref".to_string(), source_ref.clone());
    }

    match state.memory_store.store(entry).await {
        Ok(_) => Some(memory_id),
        Err(e) => {
            tracing::warn!(
                "failed to store loop post memory for post {}: {}",
                post.id,
                e
            );
            None
        }
    }
}

pub async fn create_loop_post(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateLoopPostRequest>,
) -> impl IntoResponse {
    let user_id = match user_id_from_headers(&headers) {
        Ok(user_id) => user_id,
        Err(status) => return status.into_response(),
    };
    if let Err(e) = validate_create_payload(&payload) {
        return (StatusCode::BAD_REQUEST, e).into_response();
    }

    let post_id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp();
    let post_type = payload.post_type.unwrap_or_else(|| "thought".to_string());
    let feedback_mode = payload
        .feedback_mode
        .unwrap_or_else(|| "balance".to_string());
    let visibility = payload.visibility.unwrap_or_else(|| "private".to_string());
    let title = normalize_optional_text(payload.title);
    let body = payload.body.trim().to_string();
    let source_ref = normalize_optional_text(payload.source_ref);

    let result = sqlx::query(
        r#"
        INSERT INTO loop_posts (
            id, user_id, post_type, feedback_mode, title, body, visibility, source_ref,
            memory_entry_id, preference_status, preference_extracted_at, preference_error,
            created_at, updated_at, status
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, NULL, 'pending', NULL, NULL, ?, ?, 'published')
        "#,
    )
    .bind(&post_id)
    .bind(&user_id)
    .bind(&post_type)
    .bind(&feedback_mode)
    .bind(&title)
    .bind(&body)
    .bind(&visibility)
    .bind(&source_ref)
    .bind(now)
    .bind(now)
    .execute(&state.db)
    .await;

    if let Err(e) = result {
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }

    let references = payload.references.unwrap_or_default();
    for reference in references {
        let reference_id = Uuid::new_v4().to_string();
        let result = sqlx::query(
            r#"
            INSERT INTO loop_post_references (
                id, post_id, source_type, source_id, source_url, title, quote_text,
                start_ms, end_ms, created_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(reference_id)
        .bind(&post_id)
        .bind(reference.source_type)
        .bind(normalize_optional_text(reference.source_id))
        .bind(normalize_optional_text(reference.source_url))
        .bind(normalize_optional_text(reference.title))
        .bind(normalize_optional_text(reference.quote_text))
        .bind(reference.start_ms)
        .bind(reference.end_ms)
        .bind(now)
        .execute(&state.db)
        .await;

        if let Err(e) = result {
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    }

    let mut post = match fetch_user_post(&state, &user_id, &post_id).await {
        Ok(Some(post)) => post,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    let references = match fetch_references(&state, &post_id).await {
        Ok(references) => references,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };

    if let Some(memory_id) = store_memory_for_post(&state, &post, &references).await {
        let update_result = sqlx::query("UPDATE loop_posts SET memory_entry_id = ? WHERE id = ?")
            .bind(&memory_id)
            .bind(&post_id)
            .execute(&state.db)
            .await;
        if update_result.is_ok() {
            post.memory_entry_id = Some(memory_id);
        }
    }

    (
        StatusCode::CREATED,
        Json(LoopPostResponse { post, references }),
    )
        .into_response()
}

pub async fn list_loop_posts(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<LoopPostListQuery>,
) -> impl IntoResponse {
    let user_id = match user_id_from_headers(&headers) {
        Ok(user_id) => user_id,
        Err(status) => return status.into_response(),
    };
    let limit = sanitize_limit(query.limit);
    let include_deleted = query.include_deleted.unwrap_or(false);

    let posts = if include_deleted {
        sqlx::query_as::<_, LoopPost>(
            r#"
            SELECT id, user_id, post_type, feedback_mode, title, body, visibility, source_ref,
                   memory_entry_id, preference_status, preference_extracted_at, preference_error,
                   created_at, updated_at, status
            FROM loop_posts
            WHERE user_id = ?
            ORDER BY created_at DESC
            LIMIT ?
            "#,
        )
        .bind(&user_id)
        .bind(limit)
        .fetch_all(&state.db)
        .await
    } else {
        sqlx::query_as::<_, LoopPost>(
            r#"
            SELECT id, user_id, post_type, feedback_mode, title, body, visibility, source_ref,
                   memory_entry_id, preference_status, preference_extracted_at, preference_error,
                   created_at, updated_at, status
            FROM loop_posts
            WHERE user_id = ? AND status != 'deleted'
            ORDER BY created_at DESC
            LIMIT ?
            "#,
        )
        .bind(&user_id)
        .bind(limit)
        .fetch_all(&state.db)
        .await
    };

    let posts = match posts {
        Ok(posts) => posts,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };

    let mut responses = Vec::with_capacity(posts.len());
    for post in posts {
        let references = match fetch_references(&state, &post.id).await {
            Ok(references) => references,
            Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        };
        responses.push(LoopPostResponse { post, references });
    }

    Json(responses).into_response()
}

pub async fn get_loop_post(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let user_id = match user_id_from_headers(&headers) {
        Ok(user_id) => user_id,
        Err(status) => return status.into_response(),
    };
    let post = match fetch_user_post(&state, &user_id, &id).await {
        Ok(Some(post)) if post.status.as_deref() != Some("deleted") => post,
        Ok(_) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    let references = match fetch_references(&state, &id).await {
        Ok(references) => references,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    Json(LoopPostResponse { post, references }).into_response()
}

pub async fn update_loop_post(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(payload): Json<UpdateLoopPostRequest>,
) -> impl IntoResponse {
    let user_id = match user_id_from_headers(&headers) {
        Ok(user_id) => user_id,
        Err(status) => return status.into_response(),
    };
    if let Err(e) = validate_update_payload(&payload) {
        return (StatusCode::BAD_REQUEST, e).into_response();
    }

    let existing = match fetch_user_post(&state, &user_id, &id).await {
        Ok(Some(post)) if post.status.as_deref() != Some("deleted") => post,
        Ok(_) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };

    let title = payload
        .title
        .map(Some)
        .unwrap_or_else(|| existing.title.clone())
        .and_then(|title| normalize_optional_text(Some(title)));
    let feedback_mode = payload.feedback_mode.unwrap_or_else(|| {
        existing
            .feedback_mode
            .clone()
            .unwrap_or_else(|| "balance".to_string())
    });
    let body = payload
        .body
        .map(|body| body.trim().to_string())
        .unwrap_or_else(|| existing.body.clone());
    let visibility = payload.visibility.unwrap_or(existing.visibility);
    let status = payload
        .status
        .unwrap_or_else(|| existing.status.unwrap_or_else(|| "published".to_string()));
    let now = chrono::Utc::now().timestamp();

    let result = sqlx::query(
        r#"
        UPDATE loop_posts
        SET feedback_mode = ?, title = ?, body = ?, visibility = ?, status = ?,
            preference_status = CASE WHEN ? = 'deleted' THEN preference_status ELSE 'pending' END,
            preference_error = NULL,
            updated_at = ?
        WHERE id = ? AND user_id = ?
        "#,
    )
    .bind(&feedback_mode)
    .bind(&title)
    .bind(&body)
    .bind(&visibility)
    .bind(&status)
    .bind(&status)
    .bind(now)
    .bind(&id)
    .bind(&user_id)
    .execute(&state.db)
    .await;

    if let Err(e) = result {
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }

    let mut post = match fetch_user_post(&state, &user_id, &id).await {
        Ok(Some(post)) => post,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    let references = match fetch_references(&state, &id).await {
        Ok(references) => references,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };

    if status == "deleted" {
        if let Some(memory_id) = post.memory_entry_id.clone() {
            let _ = state.memory_store.delete(&memory_id).await;
        }
    } else if let Some(memory_id) = store_memory_for_post(&state, &post, &references).await {
        if let Some(old_memory_id) = post.memory_entry_id.clone() {
            let _ = state.memory_store.delete(&old_memory_id).await;
        }
        let update_result = sqlx::query("UPDATE loop_posts SET memory_entry_id = ? WHERE id = ?")
            .bind(&memory_id)
            .bind(&id)
            .execute(&state.db)
            .await;
        if update_result.is_ok() {
            post.memory_entry_id = Some(memory_id);
        }
    }

    Json(LoopPostResponse { post, references }).into_response()
}

pub async fn delete_loop_post(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let user_id = match user_id_from_headers(&headers) {
        Ok(user_id) => user_id,
        Err(status) => return status.into_response(),
    };

    let post = match fetch_user_post(&state, &user_id, &id).await {
        Ok(Some(post)) => post,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };

    let now = chrono::Utc::now().timestamp();
    let result = sqlx::query(
        "UPDATE loop_posts SET status = 'deleted', updated_at = ? WHERE id = ? AND user_id = ?",
    )
    .bind(now)
    .bind(&id)
    .bind(&user_id)
    .execute(&state.db)
    .await;
    if let Err(e) = result {
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }

    if let Some(memory_id) = post.memory_entry_id {
        let _ = state.memory_store.delete(&memory_id).await;
    }

    StatusCode::NO_CONTENT.into_response()
}

pub async fn list_pending_loop_posts_internal(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<InternalPendingLoopPostQuery>,
) -> impl IntoResponse {
    if !has_internal_auth(&headers, &state) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let limit = sanitize_limit(query.limit);
    let posts = match sqlx::query_as::<_, LoopPost>(
        r#"
        SELECT id, user_id, post_type, feedback_mode, title, body, visibility, source_ref,
               memory_entry_id, preference_status, preference_extracted_at, preference_error,
               created_at, updated_at, status
        FROM loop_posts
        WHERE status = 'published'
          AND COALESCE(preference_status, 'pending') IN ('pending', 'failed')
        ORDER BY created_at ASC
        LIMIT ?
        "#,
    )
    .bind(limit)
    .fetch_all(&state.db)
    .await
    {
        Ok(posts) => posts,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };

    let mut responses = Vec::with_capacity(posts.len());
    for post in posts {
        let references = match fetch_references(&state, &post.id).await {
            Ok(references) => references,
            Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        };
        responses.push(InternalLoopPostResponse { post, references });
    }

    Json(responses).into_response()
}

pub async fn update_loop_post_preference_result_internal(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(payload): Json<InternalPreferenceResultRequest>,
) -> impl IntoResponse {
    if !has_internal_auth(&headers, &state) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if let Err(e) = validate_label(&payload.status, ALLOWED_PREFERENCE_STATUS, "status") {
        return (StatusCode::BAD_REQUEST, e).into_response();
    }

    let now = chrono::Utc::now().timestamp();
    let error = normalize_optional_text(payload.error);
    let extracted_at = if payload.status == "processed" || payload.status == "skipped" {
        Some(now)
    } else {
        None
    };

    let result = sqlx::query(
        r#"
        UPDATE loop_posts
        SET preference_status = ?, preference_extracted_at = ?, preference_error = ?, updated_at = ?
        WHERE id = ?
        "#,
    )
    .bind(&payload.status)
    .bind(extracted_at)
    .bind(&error)
    .bind(now)
    .bind(&id)
    .execute(&state.db)
    .await;

    match result {
        Ok(result) if result.rows_affected() == 0 => StatusCode::NOT_FOUND.into_response(),
        Ok(_) => match fetch_post_by_id(&state, &id).await {
            Ok(Some(post)) => Json(post).into_response(),
            Ok(None) => StatusCode::NOT_FOUND.into_response(),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        },
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_payload() -> CreateLoopPostRequest {
        CreateLoopPostRequest {
            post_type: Some("quote_comment".to_string()),
            feedback_mode: Some("boost".to_string()),
            title: Some("Memory product idea".to_string()),
            body: "I like the feedback loop, not the generic tutorial.".to_string(),
            visibility: Some("private".to_string()),
            source_ref: Some("feed_item:123".to_string()),
            references: Some(vec![CreateLoopPostReferenceRequest {
                source_type: "article".to_string(),
                source_id: Some("123".to_string()),
                source_url: Some("https://example.com/post".to_string()),
                title: Some("Original".to_string()),
                quote_text: Some("Feedback should become a personal feed.".to_string()),
                start_ms: None,
                end_ms: None,
            }]),
        }
    }

    #[test]
    fn create_payload_accepts_quote_comment_reference() {
        assert!(validate_create_payload(&valid_payload()).is_ok());
    }

    #[test]
    fn create_payload_rejects_empty_body() {
        let mut payload = valid_payload();
        payload.body = "  ".to_string();
        assert_eq!(
            validate_create_payload(&payload).unwrap_err(),
            "body is required"
        );
    }

    #[test]
    fn create_payload_rejects_invalid_visibility() {
        let mut payload = valid_payload();
        payload.visibility = Some("friends".to_string());
        assert!(validate_create_payload(&payload)
            .unwrap_err()
            .contains("invalid visibility"));
    }

    #[test]
    fn reference_rejects_backwards_audio_range() {
        let mut payload = valid_payload();
        let refs = payload.references.as_mut().unwrap();
        refs[0].start_ms = Some(10);
        refs[0].end_ms = Some(5);
        assert_eq!(
            validate_create_payload(&payload).unwrap_err(),
            "end_ms must be greater than or equal to start_ms"
        );
    }

    #[test]
    fn create_payload_rejects_too_many_references() {
        let mut payload = valid_payload();
        let reference = CreateLoopPostReferenceRequest {
            source_type: "article".to_string(),
            source_id: Some("123".to_string()),
            source_url: Some("https://example.com/post".to_string()),
            title: Some("Original".to_string()),
            quote_text: Some("Feedback should become a personal feed.".to_string()),
            start_ms: None,
            end_ms: None,
        };
        payload.references = Some(
            (0..=MAX_REFERENCES_PER_POST)
                .map(|_| CreateLoopPostReferenceRequest {
                    source_type: reference.source_type.clone(),
                    source_id: reference.source_id.clone(),
                    source_url: reference.source_url.clone(),
                    title: reference.title.clone(),
                    quote_text: reference.quote_text.clone(),
                    start_ms: reference.start_ms,
                    end_ms: reference.end_ms,
                })
                .collect(),
        );
        assert!(validate_create_payload(&payload)
            .unwrap_err()
            .contains("references exceeds"));
    }

    #[test]
    fn memory_content_preserves_body_and_references() {
        let post = LoopPost {
            id: "post_1".to_string(),
            user_id: "user_1".to_string(),
            post_type: "quote_comment".to_string(),
            feedback_mode: Some("reduce".to_string()),
            title: Some("A title".to_string()),
            body: "This point matters for Loop.".to_string(),
            visibility: "private".to_string(),
            source_ref: None,
            memory_entry_id: None,
            preference_status: Some("pending".to_string()),
            preference_extracted_at: None,
            preference_error: None,
            created_at: Some(1),
            updated_at: Some(1),
            status: Some("published".to_string()),
        };
        let references = vec![LoopPostReference {
            id: "ref_1".to_string(),
            post_id: "post_1".to_string(),
            source_type: "article".to_string(),
            source_id: Some("article_1".to_string()),
            source_url: None,
            title: Some("Article".to_string()),
            quote_text: Some("Original quote".to_string()),
            start_ms: None,
            end_ms: None,
            created_at: Some(1),
        }];

        let content = build_memory_content(&post, &references);
        assert!(content.contains("Feedback Mode: reduce"));
        assert!(content.contains("This point matters for Loop."));
        assert!(content.contains("Article"));
        assert!(content.contains("Original quote"));
    }

    #[test]
    fn update_payload_rejects_deleted_typo() {
        let payload = UpdateLoopPostRequest {
            feedback_mode: None,
            title: None,
            body: None,
            visibility: None,
            status: Some("delete".to_string()),
        };
        assert!(validate_update_payload(&payload)
            .unwrap_err()
            .contains("invalid status"));
    }

    #[test]
    fn create_payload_rejects_invalid_feedback_mode() {
        let mut payload = valid_payload();
        payload.feedback_mode = Some("mute".to_string());
        assert!(validate_create_payload(&payload)
            .unwrap_err()
            .contains("invalid feedback_mode"));
    }
}
