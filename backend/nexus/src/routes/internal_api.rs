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
