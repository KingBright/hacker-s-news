use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json},
};
use serde::Deserialize;
use serde_json::json;
use crate::AppState;
use crate::routes::items::Item;

pub async fn list_pending_items(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    // Check Auth
    let api_key = headers.get("X-NEXUS-KEY").and_then(|v| v.to_str().ok());
    if api_key != Some(&state.api_key) {
        return (StatusCode::UNAUTHORIZED, "Invalid API Key").into_response();
    }

    let items = sqlx::query_as::<_, Item>(
        "SELECT id, title, summary, original_url, cover_image_url, audio_url, publish_time, created_at, rating, tags, is_deleted, duration_sec, status FROM items WHERE status = 'pending_regen'",
    )
    .fetch_all(&state.db)
    .await;

    match items {
        Ok(items) => Json(items).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[derive(Deserialize)]
pub struct CompleteItemRequest {
    pub audio_url: String,
    pub summary: String,
    pub duration_sec: Option<i64>,
    pub publish_time: i64,
}

pub async fn complete_item(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(payload): Json<CompleteItemRequest>,
) -> impl IntoResponse {
    // Check Auth
    let api_key = headers.get("X-NEXUS-KEY").and_then(|v| v.to_str().ok());
    if api_key != Some(&state.api_key) {
        return (StatusCode::UNAUTHORIZED, "Invalid API Key").into_response();
    }

    let result = sqlx::query(
        "UPDATE items SET audio_url = ?, summary = ?, duration_sec = ?, publish_time = ?, status = 'published' WHERE id = ?"
    )
    .bind(&payload.audio_url)
    .bind(&payload.summary)
    .bind(payload.duration_sec)
    .bind(payload.publish_time)
    .bind(id)
    .execute(&state.db)
    .await;

    match result {
        Ok(_) => StatusCode::OK.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

// === Source Links API ===

#[derive(Deserialize)]
pub struct SourceItem {
    pub url: String,
    pub title: String,
    pub summary: String,
}

#[derive(Deserialize)]
pub struct PushSourcesRequest {
    pub sources: Vec<SourceItem>,
}

pub async fn push_sources(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(item_id): Path<String>,
    Json(payload): Json<PushSourcesRequest>,
) -> impl IntoResponse {
    // Check Auth
    let api_key = headers.get("X-NEXUS-KEY").and_then(|v| v.to_str().ok());
    if api_key != Some(&state.api_key) {
        return (StatusCode::UNAUTHORIZED, "Invalid API Key").into_response();
    }

    let now = chrono::Utc::now().timestamp();
    
    for source in payload.sources {
        let id = uuid::Uuid::new_v4().to_string();
        let _ = sqlx::query(
            "INSERT OR IGNORE INTO item_sources (id, item_id, source_url, source_title, source_summary, created_at) VALUES (?, ?, ?, ?, ?, ?)"
        )
        .bind(&id)
        .bind(&item_id)
        .bind(&source.url)
        .bind(&source.title)
        .bind(&source.summary)
        .bind(now)
        .execute(&state.db)
        .await;
    }

    StatusCode::OK.into_response()
}

#[derive(serde::Serialize, sqlx::FromRow)]
pub struct ItemSource {
    pub id: String,
    pub item_id: String,
    pub source_url: String,
    pub source_title: Option<String>,
    pub source_summary: Option<String>,
    pub created_at: Option<i64>,
}

pub async fn get_sources(
    State(state): State<AppState>,
    Path(item_id): Path<String>,
) -> impl IntoResponse {
    let sources = sqlx::query_as::<_, ItemSource>(
        "SELECT id, item_id, source_url, source_title, source_summary, created_at FROM item_sources WHERE item_id = ? ORDER BY created_at ASC"
    )
    .bind(&item_id)
    .fetch_all(&state.db)
    .await;

    match sources {
        Ok(sources) => Json(sources).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}
