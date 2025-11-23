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
}

#[derive(Deserialize)]
pub struct CreateItemRequest {
    pub title: String,
    pub summary: Option<String>,
    pub original_url: Option<String>,
    pub cover_image_url: Option<String>,
    pub audio_url: Option<String>,
    pub publish_time: Option<i64>,
}

#[derive(Deserialize)]
pub struct Pagination {
    pub page: Option<i64>,
    pub limit: Option<i64>,
}

pub async fn list_items(
    State(state): State<AppState>,
    Query(pagination): Query<Pagination>,
) -> impl IntoResponse {
    let limit = pagination.limit.unwrap_or(20);
    let offset = (pagination.page.unwrap_or(1) - 1) * limit;

    let items = sqlx::query_as::<_, Item>(
        "SELECT * FROM items ORDER BY publish_time DESC LIMIT ? OFFSET ?",
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.db)
    .await;

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

    let id = Uuid::new_v4().to_string();
    let created_at = chrono::Utc::now().timestamp();

    let result = sqlx::query(
        r#"
        INSERT INTO items (id, title, summary, original_url, cover_image_url, audio_url, publish_time, created_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&id)
    .bind(&payload.title)
    .bind(&payload.summary)
    .bind(&payload.original_url)
    .bind(&payload.cover_image_url)
    .bind(&payload.audio_url)
    .bind(payload.publish_time)
    .bind(created_at)
    .execute(&state.db)
    .await;

    match result {
        Ok(_) => Json(json!({ "id": id })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}
