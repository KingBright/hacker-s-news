use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;
use sqlx::FromRow;
use crate::AppState;

// Input validation constants
const MAX_TITLE_LENGTH: usize = 500;
const MAX_SUMMARY_LENGTH: usize = 10000;
const MAX_URL_LENGTH: usize = 2048;
const MIN_LIMIT: i64 = 1;
const MAX_LIMIT: i64 = 100;

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct Item {
    pub id: String,
    pub title: String,
    pub summary: Option<String>,
    pub original_url: Option<String>,
    pub cover_image_url: Option<String>,
    pub audio_url: Option<String>,
    pub publish_time: Option<i64>,
    pub created_at: Option<i64>,
    pub rating: Option<i32>,
    pub tags: Option<String>,
    pub is_deleted: Option<bool>,
    pub duration_sec: Option<i64>,
    pub status: Option<String>,
    pub category: Option<String>,
}

#[derive(Deserialize)]
pub struct CreateSourceItem {
    pub url: String,
    pub title: String,
    pub summary: String,
}

#[derive(Deserialize)]
pub struct CreateItemRequest {
    pub title: String,
    pub summary: Option<String>,
    pub original_url: Option<String>,
    pub cover_image_url: Option<String>,
    pub audio_url: Option<String>,
    pub publish_time: Option<i64>,
    pub duration_sec: Option<i64>,
    pub sources: Option<Vec<CreateSourceItem>>,
    pub category: Option<String>,
}

#[derive(Deserialize)]
pub struct Pagination {
    pub page: Option<i64>,
    pub limit: Option<i64>,
    pub category: Option<String>,
}

/// Validate URL format (basic check)
fn is_valid_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

/// Sanitize pagination parameters
fn sanitize_pagination(page: Option<i64>, limit: Option<i64>) -> (i64, i64) {
    let limit = limit.unwrap_or(20).clamp(MIN_LIMIT, MAX_LIMIT);
    let page = page.unwrap_or(1).max(1);
    let offset = (page - 1) * limit;
    (limit, offset)
}

pub async fn list_items(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(pagination): Query<Pagination>,
) -> impl IntoResponse {
    let (limit, offset) = sanitize_pagination(pagination.page, pagination.limit);
    let category_filter = pagination.category.as_deref();

    // Extract user_id from header for filtering listened items
    let user_id = headers.get("x-user-id").and_then(|v| v.to_str().ok());

    let items = match (user_id, category_filter) {
        (Some(uid), Some(cat)) => {
            sqlx::query_as::<_, Item>(
                r#"
                SELECT i.id, i.title, i.summary, i.original_url, i.cover_image_url, i.audio_url,
                       i.publish_time, i.created_at, i.rating, i.tags, i.is_deleted, i.duration_sec, i.status, i.category
                FROM items i
                LEFT JOIN user_history uh ON i.id = uh.item_id AND uh.user_id = ?
                WHERE (i.is_deleted = 0 OR i.is_deleted IS NULL) AND uh.item_id IS NULL AND i.category = ?
                ORDER BY i.publish_time DESC
                LIMIT ? OFFSET ?
                "#,
            )
            .bind(uid)
            .bind(cat)
            .bind(limit)
            .bind(offset)
            .fetch_all(&state.db)
            .await
        }
        (Some(uid), None) => {
            sqlx::query_as::<_, Item>(
                r#"
                SELECT i.id, i.title, i.summary, i.original_url, i.cover_image_url, i.audio_url,
                       i.publish_time, i.created_at, i.rating, i.tags, i.is_deleted, i.duration_sec, i.status, i.category
                FROM items i
                LEFT JOIN user_history uh ON i.id = uh.item_id AND uh.user_id = ?
                WHERE (i.is_deleted = 0 OR i.is_deleted IS NULL) AND uh.item_id IS NULL
                ORDER BY i.publish_time DESC
                LIMIT ? OFFSET ?
                "#,
            )
            .bind(uid)
            .bind(limit)
            .bind(offset)
            .fetch_all(&state.db)
            .await
        }
        (None, Some(cat)) => {
            sqlx::query_as::<_, Item>(
                "SELECT id, title, summary, original_url, cover_image_url, audio_url, publish_time, created_at, rating, tags, is_deleted, duration_sec, status, category FROM items WHERE (is_deleted = 0 OR is_deleted IS NULL) AND category = ? ORDER BY publish_time DESC LIMIT ? OFFSET ?",
            )
            .bind(cat)
            .bind(limit)
            .bind(offset)
            .fetch_all(&state.db)
            .await
        }
        (None, None) => {
            sqlx::query_as::<_, Item>(
                "SELECT id, title, summary, original_url, cover_image_url, audio_url, publish_time, created_at, rating, tags, is_deleted, duration_sec, status, category FROM items WHERE is_deleted = 0 OR is_deleted IS NULL ORDER BY publish_time DESC LIMIT ? OFFSET ?",
            )
            .bind(limit)
            .bind(offset)
            .fetch_all(&state.db)
            .await
        }
    };

    match items {
        Ok(items) => Json(items).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub async fn create_item(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateItemRequest>,
) -> impl IntoResponse {
    // Check Auth
    let api_key = headers.get("X-NEXUS-KEY").and_then(|v| v.to_str().ok());
    if api_key != Some(&state.api_key) {
        return (StatusCode::UNAUTHORIZED, "Invalid API Key").into_response();
    }

    // === INPUT VALIDATION ===
    if payload.title.is_empty() || payload.title.len() > MAX_TITLE_LENGTH {
        return (StatusCode::BAD_REQUEST, Json(json!({
            "error": format!("Title must be 1-{} characters", MAX_TITLE_LENGTH)
        }))).into_response();
    }

    if let Some(ref url) = payload.original_url {
        if url.len() > MAX_URL_LENGTH {
            return (StatusCode::BAD_REQUEST, Json(json!({
                "error": format!("URL exceeds maximum length of {}", MAX_URL_LENGTH)
            }))).into_response();
        }
        if !url.is_empty() && !is_valid_url(url) {
            return (StatusCode::BAD_REQUEST, Json(json!({
                "error": "Invalid URL format (must start with http:// or https://)"
            }))).into_response();
        }
    }

    if let Some(ref summary) = payload.summary {
        if summary.len() > MAX_SUMMARY_LENGTH {
            return (StatusCode::BAD_REQUEST, Json(json!({
                "error": format!("Summary exceeds maximum length of {}", MAX_SUMMARY_LENGTH)
            }))).into_response();
        }
    }

    let id = Uuid::new_v4().to_string();
    let created_at = chrono::Utc::now().timestamp();

    // === INGESTION FILTERING ===

    // 1. Stale Check: Reject items older than 7 days
    if let Some(pub_time) = payload.publish_time {
        let now = created_at;
        if pub_time < now - 7 * 24 * 3600 {
            return Json(json!({ "id": "skipped", "status": "skipped_stale", "reason": "Older than 7 days" })).into_response();
        }
    }

    // 2. Use INSERT OR IGNORE for atomic race-condition-safe deduplication
    // First try to insert with the unique constraint handling
    let insert_result = sqlx::query(
        r#"
        INSERT OR IGNORE INTO items (id, title, summary, original_url, cover_image_url, audio_url, publish_time, created_at, duration_sec, category)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&id)
    .bind(&payload.title)
    .bind(&payload.summary)
    .bind(&payload.original_url)
    .bind(&payload.cover_image_url)
    .bind(&payload.audio_url)
    .bind(&payload.publish_time)
    .bind(created_at)
    .bind(payload.duration_sec)
    .bind(&payload.category)
    .execute(&state.db)
    .await;

    match insert_result {
        Ok(result) => {
            // Check if the insert actually happened (rows_affected > 0)
            if result.rows_affected() == 0 {
                // Insert was ignored due to duplicate URL or other constraint
                return Json(json!({ "id": "skipped", "status": "skipped_dupe" })).into_response();
            }

            // Handle Sources - log errors but don't fail the request
            if let Some(sources) = &payload.sources {
                for source in sources {
                    let source_id = uuid::Uuid::new_v4().to_string();
                    let now = chrono::Utc::now().timestamp();
                    if let Err(e) = sqlx::query(
                        "INSERT INTO item_sources (id, item_id, source_url, source_title, source_summary, created_at) VALUES (?, ?, ?, ?, ?, ?)"
                    )
                    .bind(&source_id)
                    .bind(&id)
                    .bind(&source.url)
                    .bind(&source.title)
                    .bind(&source.summary)
                    .bind(now)
                    .execute(&state.db)
                    .await
                    {
                        tracing::warn!("Failed to insert source for item {}: {}", id, e);
                    }
                }
            }

            Json(json!({ "id": id, "status": "created" })).into_response()
        }
        Err(e) => {
            tracing::error!("DB Insert Failed: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}

pub async fn create_item_multipart(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut multipart: axum::extract::Multipart,
) -> impl IntoResponse {
    use std::path::Path;
    use tokio::io::AsyncWriteExt;

    // Check Auth
    let api_key = headers.get("X-NEXUS-KEY").and_then(|v| v.to_str().ok());
    if api_key != Some(&state.api_key) {
        return (StatusCode::UNAUTHORIZED, "Invalid API Key").into_response();
    }

    let mut payload: Option<CreateItemRequest> = None;
    let mut file_path: Option<String> = None;
    let mut audio_url: Option<String> = None;
    let mut saved_file = false;

    // Generate ID
    let id = Uuid::new_v4().to_string();
    let created_at = chrono::Utc::now().timestamp();

    while let Ok(Some(mut field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();

        if name == "payload" {
            if let Ok(data) = field.text().await {
                if let Ok(parsed) = serde_json::from_str::<CreateItemRequest>(&data) {
                    payload = Some(parsed);
                }
            }
        } else if name == "file" {
            let filename = field.file_name().unwrap_or("audio.wav").to_string();
            let clean_name = Path::new(&filename).file_name().and_then(|n| n.to_str()).unwrap_or("audio.wav");
            let target_name = format!("{}_{}", id, clean_name);
            let target_path = Path::new(&state.audio_dir).join(&target_name);

            file_path = Some(target_path.to_string_lossy().to_string());

            let mut file = match tokio::fs::File::create(&target_path).await {
                Ok(f) => f,
                Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to create file: {}", e)).into_response(),
            };

            while let Ok(Some(chunk)) = field.chunk().await {
                if let Err(e) = file.write_all(&chunk).await {
                    let _ = tokio::fs::remove_file(&target_path).await;
                    return (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to write file: {}", e)).into_response();
                }
            }
            saved_file = true;
            audio_url = Some(format!("/audio/{}", target_name));
        }
    }

    let Some(mut item_req) = payload else {
         if let Some(path) = file_path {
             let _ = tokio::fs::remove_file(path).await;
         }
         return (StatusCode::BAD_REQUEST, "Missing payload").into_response();
    };

    if saved_file {
        item_req.audio_url = audio_url;
    }

    // Use INSERT OR IGNORE for atomic deduplication
    let result = sqlx::query(
        r#"
        INSERT OR IGNORE INTO items (id, title, summary, original_url, cover_image_url, audio_url, publish_time, created_at, duration_sec, category)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&id)
    .bind(&item_req.title)
    .bind(&item_req.summary)
    .bind(&item_req.original_url)
    .bind(&item_req.cover_image_url)
    .bind(&item_req.audio_url)
    .bind(&item_req.publish_time)
    .bind(created_at)
    .bind(item_req.duration_sec)
    .bind(&item_req.category)
    .execute(&state.db)
    .await;

    match result {
        Ok(result) => {
            // Check if insert was successful
            if result.rows_affected() == 0 {
                // Cleanup file if insert was skipped
                if let Some(path) = file_path {
                    let _ = tokio::fs::remove_file(path).await;
                }
                return Json(json!({ "id": "skipped", "status": "skipped_dupe" })).into_response();
            }

            // Handle sources
            if let Some(sources) = &item_req.sources {
                for source in sources {
                    let source_id = uuid::Uuid::new_v4().to_string();
                    let now = chrono::Utc::now().timestamp();
                    if let Err(e) = sqlx::query(
                        "INSERT INTO item_sources (id, item_id, source_url, source_title, source_summary, created_at) VALUES (?, ?, ?, ?, ?, ?)"
                    )
                    .bind(&source_id)
                    .bind(&id)
                    .bind(&source.url)
                    .bind(&source.title)
                    .bind(&source.summary)
                    .bind(now)
                    .execute(&state.db)
                    .await
                    {
                        tracing::warn!("Failed to insert source for item {}: {}", id, e);
                    }
                }
            }
            Json(json!({ "id": id, "status": "created" })).into_response()
        },
        Err(e) => {
            tracing::error!("DB Insert Failed: {}", e);
            if let Some(path) = file_path {
                let _ = tokio::fs::remove_file(path).await;
            }
            (StatusCode::INTERNAL_SERVER_ERROR, format!("DB Insert Failed: {}", e)).into_response()
        }
    }
}
