use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json},
};
use serde::{Deserialize, Serialize};
use crate::AppState;
use uuid::Uuid;

#[derive(Deserialize)]
pub struct CheckRequest {
    pub urls: Vec<String>,
}

#[derive(Serialize)]
pub struct CheckResponse {
    pub existing_urls: Vec<String>,
}

#[derive(Deserialize)]
pub struct MarkRequest {
    pub url: String,
    pub category: String,
}

pub async fn check_files(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CheckRequest>,
) -> impl IntoResponse {
    // Check Auth
    let api_key = headers.get("X-NEXUS-KEY").and_then(|v| v.to_str().ok());
    if api_key != Some(&state.api_key) {
        return (StatusCode::UNAUTHORIZED, "Invalid API Key").into_response();
    }

    if payload.urls.is_empty() {
        return Json(CheckResponse { existing_urls: vec![] }).into_response();
    }

    // Query both source_items (legacy dedup table) and items.original_url (authoritative content table).
    let placeholders: Vec<String> = payload.urls.iter().map(|_| "?".to_string()).collect();
    let query = format!(
        r#"
        SELECT url FROM source_items WHERE url IN ({})
        UNION
        SELECT original_url AS url
        FROM items
        WHERE original_url IS NOT NULL
          AND original_url != ''
          AND original_url IN ({})
        "#,
        placeholders.join(","),
        placeholders.join(",")
    );

    let mut query_builder = sqlx::query_as::<_, (String,)>(&query);
    for url in &payload.urls {
        query_builder = query_builder.bind(url);
    }
    for url in &payload.urls {
        query_builder = query_builder.bind(url);
    }

    let result = query_builder.fetch_all(&state.db).await;

    match result {
        Ok(rows) => {
            let existing_urls = rows.into_iter().map(|(url,)| url).collect();
            Json(CheckResponse { existing_urls }).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub async fn mark_file(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<MarkRequest>,
) -> impl IntoResponse {
    // Check Auth
    let api_key = headers.get("X-NEXUS-KEY").and_then(|v| v.to_str().ok());
    if api_key != Some(&state.api_key) {
        return (StatusCode::UNAUTHORIZED, "Invalid API Key").into_response();
    }

    let id = Uuid::new_v4().to_string();
    let created_at = chrono::Utc::now().timestamp();

    let result = sqlx::query(
        "INSERT OR IGNORE INTO source_items (id, url, category, created_at) VALUES (?, ?, ?, ?)",
    )
    .bind(id)
    .bind(&payload.url)
    .bind(&payload.category)
    .bind(created_at)
    .execute(&state.db)
    .await;

    match result {
        Ok(_) => StatusCode::OK.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}
