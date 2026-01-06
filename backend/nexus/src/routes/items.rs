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
}

pub async fn list_items(
    State(state): State<AppState>,
    Query(pagination): Query<Pagination>,
) -> impl IntoResponse {
    let limit = pagination.limit.unwrap_or(20);
    let offset = (pagination.page.unwrap_or(1) - 1) * limit;

    let items = sqlx::query_as::<_, Item>(
        "SELECT id, title, summary, original_url, cover_image_url, audio_url, publish_time, created_at, rating, tags, is_deleted, duration_sec, status, category FROM items WHERE is_deleted = 0 OR is_deleted IS NULL ORDER BY publish_time DESC LIMIT ? OFFSET ?",
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
        INSERT INTO items (id, title, summary, original_url, cover_image_url, audio_url, publish_time, created_at, duration_sec, category)
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

    // Handle Sources
    if let Ok(_) = result {
        if let Some(sources) = &payload.sources {
            for source in sources {
                let source_id = uuid::Uuid::new_v4().to_string();
                let now = chrono::Utc::now().timestamp();
                let _ = sqlx::query("INSERT INTO item_sources (id, item_id, source_url, source_title, source_summary, created_at) VALUES (?, ?, ?, ?, ?, ?)")
                    .bind(&source_id)
                    .bind(&id)
                    .bind(&source.url)
                    .bind(&source.title)
                    .bind(&source.summary)
                    .bind(now)
                    .execute(&state.db)
                    .await;
            }
        }
    }

    match result {
        Ok(_) => Json(json!({ "id": id, "status": "created" })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
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

    let result = sqlx::query(
        r#"
        INSERT INTO items (id, title, summary, original_url, cover_image_url, audio_url, publish_time, created_at, duration_sec, category)
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
        Ok(_) => {
             // Handle sources
            if let Some(sources) = &item_req.sources {
                for source in sources {
                    let source_id = uuid::Uuid::new_v4().to_string();
                    let now = chrono::Utc::now().timestamp();
                    let _ = sqlx::query("INSERT INTO item_sources (id, item_id, source_url, source_title, source_summary, created_at) VALUES (?, ?, ?, ?, ?, ?)")
                        .bind(&source_id)
                        .bind(&id)
                        .bind(&source.url)
                        .bind(&source.title)
                        .bind(&source.summary)
                        .bind(now)
                        .execute(&state.db)
                        .await;
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
