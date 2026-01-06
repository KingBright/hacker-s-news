use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json},
};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use crate::AppState;

const DEFAULT_USER: &str = "default";

#[derive(Serialize, FromRow)]
pub struct HistoryItem {
    pub item_id: String,
    pub played_at: Option<i64>,
}

pub async fn get_history(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    // Optional: Check Auth
    let user_id = headers.get("x-user-id").and_then(|v| v.to_str().ok());
    
    // If no user_id (Guest), return empty history
    let user_id = match user_id {
        Some(id) => id,
        None => return Json(Vec::<HistoryItem>::new()).into_response(),
    };
    
    let history = sqlx::query_as::<_, HistoryItem>(
        "SELECT item_id, played_at FROM user_history WHERE user_id = ? ORDER BY played_at DESC"
    )
    .bind(user_id)
    .fetch_all(&state.db)
    .await;

    match history {
        Ok(items) => Json(items).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[derive(Deserialize)]
pub struct MarkPlayedRequest {
    pub item_id: String,
}

pub async fn mark_played(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<MarkPlayedRequest>,
) -> impl IntoResponse {
    let user_id = headers.get("x-user-id").and_then(|v| v.to_str().ok());

    // If no user_id, fail silently (Guest mode doesn't save)
    let user_id = match user_id {
        Some(id) => id,
        None => return StatusCode::OK.into_response(),
    };

    let now = chrono::Utc::now().timestamp();

    let result = sqlx::query(
        "INSERT OR REPLACE INTO user_history (user_id, item_id, played_at) VALUES (?, ?, ?)"
    )
    .bind(user_id)
    .bind(&payload.item_id)
    .bind(now)
    .execute(&state.db)
    .await;

    match result {
        Ok(_) => StatusCode::OK.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub async fn unmark_played(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<MarkPlayedRequest>,
) -> impl IntoResponse {
    let user_id = headers.get("x-user-id").and_then(|v| v.to_str().ok());

    let user_id = match user_id {
        Some(id) => id,
        None => return StatusCode::OK.into_response(),
    };

    let result = sqlx::query(
        "DELETE FROM user_history WHERE user_id = ? AND item_id = ?"
    )
    .bind(user_id)
    .bind(&payload.item_id)
    .execute(&state.db)
    .await;

    match result {
        Ok(_) => StatusCode::OK.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}
