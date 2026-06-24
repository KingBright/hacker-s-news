use crate::{db::DbPool, personalization, AppState};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::FromRow;
use uuid::Uuid;

// Input validation constants
const MAX_TITLE_LENGTH: usize = 500;
const MAX_SUMMARY_LENGTH: usize = 10000;
const MAX_URL_LENGTH: usize = 2048;
const MIN_LIMIT: i64 = 1;
const MAX_LIMIT: i64 = 100;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
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

const ITEM_COLUMNS_WITH_ALIAS: &str = r#"
    i.id, i.title, i.summary, i.original_url, i.cover_image_url, i.audio_url,
    i.publish_time, i.created_at, i.rating, i.tags, i.is_deleted, i.duration_sec, i.status, i.category
"#;

const USER_VISIBLE_ORDER: &str = r#"
    ORDER BY (i.publish_time IS NULL) ASC, i.publish_time DESC,
             (i.created_at IS NULL) ASC, i.created_at DESC,
             i.id DESC
"#;

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

async fn fetch_items_for_request(
    db: &DbPool,
    user_id: Option<&str>,
    category_filter: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<Item>, sqlx::Error> {
    match (user_id, category_filter) {
        (Some(uid), Some(cat)) => {
            sqlx::query_as::<_, Item>(&format!(
                r#"
                SELECT {ITEM_COLUMNS_WITH_ALIAS}
                FROM items i
                LEFT JOIN user_history uh ON i.id = uh.item_id AND uh.user_id = ?
                WHERE (i.is_deleted = 0 OR i.is_deleted IS NULL)
                  AND uh.item_id IS NULL
                  AND i.category = ?
                {USER_VISIBLE_ORDER}
                LIMIT ? OFFSET ?
                "#
            ))
            .bind(uid)
            .bind(cat)
            .bind(limit)
            .bind(offset)
            .fetch_all(db)
            .await
        }
        (Some(uid), None) => {
            sqlx::query_as::<_, Item>(&format!(
                r#"
                SELECT {ITEM_COLUMNS_WITH_ALIAS}
                FROM items i
                LEFT JOIN user_history uh ON i.id = uh.item_id AND uh.user_id = ?
                WHERE (i.is_deleted = 0 OR i.is_deleted IS NULL)
                  AND uh.item_id IS NULL
                {USER_VISIBLE_ORDER}
                LIMIT ? OFFSET ?
                "#
            ))
            .bind(uid)
            .bind(limit)
            .bind(offset)
            .fetch_all(db)
            .await
        }
        (None, Some(cat)) => {
            sqlx::query_as::<_, Item>(
                r#"
                SELECT id, title, summary, original_url, cover_image_url, audio_url,
                       publish_time, created_at, rating, tags, is_deleted, duration_sec, status, category
                FROM items
                WHERE (is_deleted = 0 OR is_deleted IS NULL) AND category = ?
                ORDER BY publish_time DESC, created_at DESC, id DESC
                LIMIT ? OFFSET ?
                "#,
            )
            .bind(cat)
            .bind(limit)
            .bind(offset)
            .fetch_all(db)
            .await
        }
        (None, None) => {
            sqlx::query_as::<_, Item>(
                r#"
                SELECT id, title, summary, original_url, cover_image_url, audio_url,
                       publish_time, created_at, rating, tags, is_deleted, duration_sec, status, category
                FROM items
                WHERE is_deleted = 0 OR is_deleted IS NULL
                ORDER BY publish_time DESC, created_at DESC, id DESC
                LIMIT ? OFFSET ?
                "#,
            )
            .bind(limit)
            .bind(offset)
            .fetch_all(db)
            .await
        }
    }
}

fn sort_items_newest_first(items: &mut [Item]) {
    items.sort_by(|left, right| {
        right
            .publish_time
            .unwrap_or_default()
            .cmp(&left.publish_time.unwrap_or_default())
            .then_with(|| {
                right
                    .created_at
                    .unwrap_or_default()
                    .cmp(&left.created_at.unwrap_or_default())
            })
            .then_with(|| right.id.cmp(&left.id))
    });
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
    let (query_limit, query_offset) = if user_id.is_some() {
        (
            personalization::recommended_candidate_limit(limit, offset),
            0,
        )
    } else {
        (limit, offset)
    };

    let items = fetch_items_for_request(
        &state.db,
        user_id,
        category_filter,
        query_limit,
        query_offset,
    )
    .await;

    match items {
        Ok(items) => {
            let items = if let Some(user_id) = user_id {
                let fallback_items = items.clone();
                let mut items =
                    match personalization::personalize_radio_items(&state, user_id, items).await {
                        Ok(items) => items,
                        Err(_) => fallback_items,
                    };
                sort_items_newest_first(&mut items);
                items
            } else {
                items
            };
            let items = if user_id.is_some() {
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

pub async fn get_item_why(
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

    let result = sqlx::query_as::<_, Item>(
        r#"
        SELECT id, title, summary, original_url, cover_image_url, audio_url,
               publish_time, created_at, rating, tags, is_deleted, duration_sec, status, category
        FROM items
        WHERE id = ?
        "#,
    )
    .bind(&id)
    .fetch_optional(&state.db)
    .await;

    match result {
        Ok(Some(item)) => match personalization::explain_radio_item(&state, user_id, &item).await {
            Ok(response) => Json(response).into_response(),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        },
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
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
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": format!("Title must be 1-{} characters", MAX_TITLE_LENGTH)
            })),
        )
            .into_response();
    }

    if let Some(ref url) = payload.original_url {
        if url.len() > MAX_URL_LENGTH {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": format!("URL exceeds maximum length of {}", MAX_URL_LENGTH)
                })),
            )
                .into_response();
        }
        if !url.is_empty() && !is_valid_url(url) {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": "Invalid URL format (must start with http:// or https://)"
                })),
            )
                .into_response();
        }
    }

    if let Some(ref summary) = payload.summary {
        if summary.len() > MAX_SUMMARY_LENGTH {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": format!("Summary exceeds maximum length of {}", MAX_SUMMARY_LENGTH)
                })),
            )
                .into_response();
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

            // Backfill source_items to keep legacy dedup table in sync with items.original_url.
            if let Some(original_url) = payload.original_url.as_deref().filter(|u| !u.is_empty()) {
                let source_id = uuid::Uuid::new_v4().to_string();
                let category = payload
                    .category
                    .as_deref()
                    .unwrap_or("Uncategorized")
                    .to_string();
                if let Err(e) = sqlx::query(
                    "INSERT OR IGNORE INTO source_items (id, url, category, created_at) VALUES (?, ?, ?, ?)"
                )
                .bind(&source_id)
                .bind(original_url)
                .bind(category)
                .bind(created_at)
                .execute(&state.db)
                .await
                {
                    tracing::warn!("Failed to sync source_items for item {}: {}", id, e);
                }
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
            let clean_name = Path::new(&filename)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("audio.wav");
            let target_name = format!("{}_{}", id, clean_name);
            let target_path = Path::new(&state.audio_dir).join(&target_name);

            file_path = Some(target_path.to_string_lossy().to_string());

            let mut file = match tokio::fs::File::create(&target_path).await {
                Ok(f) => f,
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("Failed to create file: {}", e),
                    )
                        .into_response()
                }
            };

            while let Ok(Some(chunk)) = field.chunk().await {
                if let Err(e) = file.write_all(&chunk).await {
                    let _ = tokio::fs::remove_file(&target_path).await;
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("Failed to write file: {}", e),
                    )
                        .into_response();
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

            // Backfill source_items to keep legacy dedup table in sync with items.original_url.
            if let Some(original_url) = item_req.original_url.as_deref().filter(|u| !u.is_empty()) {
                let source_id = uuid::Uuid::new_v4().to_string();
                let category = item_req
                    .category
                    .as_deref()
                    .unwrap_or("Uncategorized")
                    .to_string();
                if let Err(e) = sqlx::query(
                    "INSERT OR IGNORE INTO source_items (id, url, category, created_at) VALUES (?, ?, ?, ?)"
                )
                .bind(&source_id)
                .bind(original_url)
                .bind(category)
                .bind(created_at)
                .execute(&state.db)
                .await
                {
                    tracing::warn!("Failed to sync source_items for item {}: {}", id, e);
                }
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
        }
        Err(e) => {
            tracing::error!("DB Insert Failed: {}", e);
            if let Some(path) = file_path {
                let _ = tokio::fs::remove_file(path).await;
            }
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("DB Insert Failed: {}", e),
            )
                .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn test_pool() -> DbPool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();

        sqlx::query(
            r#"
            CREATE TABLE items (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                summary TEXT,
                original_url TEXT UNIQUE,
                cover_image_url TEXT,
                audio_url TEXT,
                publish_time INTEGER,
                created_at INTEGER,
                rating INTEGER,
                tags TEXT,
                is_deleted BOOLEAN DEFAULT 0,
                duration_sec INTEGER,
                status TEXT DEFAULT 'published',
                category TEXT
            );
            CREATE TABLE user_history (
                user_id TEXT NOT NULL,
                item_id TEXT NOT NULL,
                played_at INTEGER,
                PRIMARY KEY (user_id, item_id)
            );
            "#,
        )
        .execute(&pool)
        .await
        .unwrap();

        pool
    }

    async fn insert_item(pool: &DbPool, id: &str, publish_time: i64, category: &str) {
        sqlx::query(
            r#"
            INSERT INTO items (
                id, title, summary, original_url, cover_image_url, audio_url,
                publish_time, created_at, rating, tags, is_deleted, duration_sec, status, category
            )
            VALUES (?, ?, NULL, ?, NULL, NULL, ?, ?, NULL, NULL, 0, NULL, 'published', ?)
            "#,
        )
        .bind(id)
        .bind(format!("item {id}"))
        .bind(format!("https://example.com/{id}"))
        .bind(publish_time)
        .bind(publish_time)
        .bind(category)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn mark_played(pool: &DbPool, user_id: &str, item_id: &str) {
        sqlx::query("INSERT INTO user_history (user_id, item_id, played_at) VALUES (?, ?, ?)")
            .bind(user_id)
            .bind(item_id)
            .bind(999_i64)
            .execute(pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn user_listing_filters_played_before_limit_and_returns_newest_first() {
        let pool = test_pool().await;
        insert_item(&pool, "oldest-unplayed", 100, "radio").await;
        insert_item(&pool, "played-middle", 200, "radio").await;
        insert_item(&pool, "middle-unplayed", 300, "radio").await;
        insert_item(&pool, "newest-unplayed", 500, "radio").await;
        insert_item(&pool, "played-newest", 600, "radio").await;

        mark_played(&pool, "user-1", "played-middle").await;
        mark_played(&pool, "user-1", "played-newest").await;

        let items = fetch_items_for_request(&pool, Some("user-1"), None, 3, 0)
            .await
            .unwrap();
        let ids = items.into_iter().map(|item| item.id).collect::<Vec<_>>();

        assert_eq!(
            ids,
            vec!["newest-unplayed", "middle-unplayed", "oldest-unplayed"]
        );
    }

    #[tokio::test]
    async fn anonymous_listing_stays_recent_first_for_admin_and_ingestion_callers() {
        let pool = test_pool().await;
        insert_item(&pool, "old", 100, "radio").await;
        insert_item(&pool, "new", 300, "radio").await;
        insert_item(&pool, "middle", 200, "radio").await;

        let items = fetch_items_for_request(&pool, None, None, 3, 0)
            .await
            .unwrap();
        let ids = items.into_iter().map(|item| item.id).collect::<Vec<_>>();

        assert_eq!(ids, vec!["new", "middle", "old"]);
    }

    #[test]
    fn newest_first_sort_restores_chronology_after_personalization() {
        let item = |id: &str, publish_time: i64| Item {
            id: id.to_string(),
            title: id.to_string(),
            summary: None,
            original_url: None,
            cover_image_url: None,
            audio_url: None,
            publish_time: Some(publish_time),
            created_at: Some(publish_time),
            rating: None,
            tags: None,
            is_deleted: Some(false),
            duration_sec: None,
            status: Some("published".to_string()),
            category: Some("radio".to_string()),
        };
        let mut items = vec![item("old", 100), item("new", 300), item("middle", 200)];

        sort_items_newest_first(&mut items);
        let ids = items.into_iter().map(|item| item.id).collect::<Vec<_>>();

        assert_eq!(ids, vec!["new", "middle", "old"]);
    }
}
