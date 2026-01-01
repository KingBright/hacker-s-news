use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json},
};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use crate::AppState;
use crate::routes::items::Item;

#[derive(Deserialize)]
pub struct UpdateItemRequest {
    pub rating: Option<i32>,
    pub tags: Option<String>,
    pub is_deleted: Option<bool>,
}

pub async fn update_item(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(payload): Json<UpdateItemRequest>,
) -> impl IntoResponse {
    // Check Auth
    let api_key = headers.get("X-NEXUS-KEY").and_then(|v| v.to_str().ok());
    if api_key != Some(&state.api_key) {
        return (StatusCode::UNAUTHORIZED, "Invalid API Key").into_response();
    }

    // Build dynamic update query
    let mut query = "UPDATE items SET ".to_string();
    let mut updates = Vec::new();
    
    if payload.rating.is_some() {
        updates.push("rating = ?");
    }
    if payload.tags.is_some() {
        updates.push("tags = ?");
    }
    if payload.is_deleted.is_some() {
        updates.push("is_deleted = ?");
    }
    
    if updates.is_empty() {
         return (StatusCode::BAD_REQUEST, "No fields to update").into_response();
    }
    
    query.push_str(&updates.join(", "));
    query.push_str(" WHERE id = ?");
    
    let mut sql = sqlx::query(&query);
    
    if let Some(rating) = payload.rating {
        sql = sql.bind(rating);
    }
    if let Some(tags) = &payload.tags {
        sql = sql.bind(tags);
    }
    if let Some(is_deleted) = payload.is_deleted {
        sql = sql.bind(is_deleted);
    }
    
    sql = sql.bind(id);

    match sql.execute(&state.db).await {
        Ok(_) => StatusCode::OK.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub async fn export_items(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    // Check Auth
    let api_key = headers.get("X-NEXUS-KEY").and_then(|v| v.to_str().ok());
    if api_key != Some(&state.api_key) {
        return (StatusCode::UNAUTHORIZED, "Invalid API Key").into_response();
    }

    // Export items where rating is set or tags are present
    let items = sqlx::query_as::<_, Item>(
        "SELECT * FROM items WHERE is_deleted = 0 AND (rating IS NOT NULL OR tags IS NOT NULL) ORDER BY created_at DESC",
    )
    .fetch_all(&state.db)
    .await;

    match items {
        Ok(items) => Json(items).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}
