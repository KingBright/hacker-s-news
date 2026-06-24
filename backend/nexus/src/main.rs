use axum::{
    extract::DefaultBodyLimit,
    http::{header, HeaderName, HeaderValue, Method, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::fs;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod db;
mod personalization;
mod routes;

use db::DbPool;

#[derive(Clone)]
pub struct AppState {
    pub db: DbPool,
    pub api_key: String,
    pub audio_dir: String,
    pub memory_store: Arc<loop_memory::RedbMemoryStore>,
}

fn build_cors_layer() -> CorsLayer {
    let origins = std::env::var("CORS_ALLOWED_ORIGINS")
        .ok()
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|origin| !origin.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .filter(|origins| !origins.is_empty())
        .unwrap_or_else(|| {
            vec![
                "http://localhost:3000".to_string(),
                "http://127.0.0.1:3000".to_string(),
                "http://localhost:8899".to_string(),
                "http://127.0.0.1:8899".to_string(),
            ]
        });

    let origins = origins
        .iter()
        .filter_map(|origin| origin.parse::<HeaderValue>().ok())
        .collect::<Vec<_>>();

    CorsLayer::new()
        .allow_origin(origins)
        .allow_methods([Method::GET, Method::POST, Method::PATCH, Method::DELETE])
        .allow_headers([
            header::CONTENT_TYPE,
            header::AUTHORIZATION,
            HeaderName::from_static("x-api-key"),
            HeaderName::from_static("x-nexus-key"),
            HeaderName::from_static("x-user-id"),
        ])
}

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "nexus=debug,tower_http=debug".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let db_pool = db::init_db().await.expect("Failed to initialize DB");
    let memory_db_path = std::env::var("MEMORY_DB_PATH").unwrap_or_else(|_| {
        let home_dir = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        format!("{}/.freshloop/data/loop_memory.redb", home_dir)
    });
    let memory_store = Arc::new(
        loop_memory::RedbMemoryStore::new(&memory_db_path)
            .expect("Failed to initialize memory store"),
    );

    // Ensure audio directory exists
    let audio_dir = std::env::var("AUDIO_DIR").unwrap_or_else(|_| "audio".to_string());
    fs::create_dir_all(&audio_dir)
        .await
        .expect("Failed to create audio dir");

    let api_key = std::env::var("NEXUS_KEY").expect("NEXUS_KEY environment variable must be set for security. Please configure it before starting the server.");

    let state = AppState {
        db: db_pool,
        api_key,
        audio_dir: audio_dir.clone(),
        memory_store,
    };

    let static_dir = std::env::var("STATIC_DIR").unwrap_or_else(|_| "dist/frontend".to_string());
    let static_index = format!("{}/index.html", static_dir);
    let admin_index = format!("{}/admin.html", static_dir);
    let feed_index = format!("{}/feed.html", static_dir);
    let loop_index = format!("{}/loop.html", static_dir);
    let focus_index = format!("{}/focus.html", static_dir);

    let app = Router::new()
        .route("/api/health", get(|| async { "ok" }))
        .route("/api/items", get(routes::items::list_items))
        .route("/api/items/{id}/why", get(routes::items::get_item_why))
        .route("/api/feed/items", get(routes::feed::list_feed_items))
        .route("/api/feed/items/{id}", get(routes::feed::get_feed_item))
        .route(
            "/api/feed/items/{id}/why",
            get(routes::feed::get_feed_item_why),
        )
        .route(
            "/api/feed/items/{id}/content",
            get(routes::feed::get_feed_item_content),
        )
        .route(
            "/api/feed/items/{id}/progress",
            post(routes::feed::update_reading_progress),
        )
        .route("/api/feed/weeklies", get(routes::feed::list_weekly_digests))
        .route(
            "/api/feed/weeklies/{id}",
            get(routes::feed::get_weekly_digest),
        )
        .route(
            "/api/memory/entries",
            get(routes::memory::list_memory_entries).post(routes::memory::create_memory_entry),
        )
        .route(
            "/api/memory/entries/{id}",
            axum::routing::delete(routes::memory::delete_memory_entry),
        )
        .route("/api/memory/search", get(routes::memory::search_memory))
        .route(
            "/api/memory/profile",
            get(routes::memory::get_memory_profile),
        )
        .route("/api/focus", get(routes::memory::get_focus_summary))
        .route(
            "/api/loop/posts",
            get(routes::loops::list_loop_posts).post(routes::loops::create_loop_post),
        )
        .route(
            "/api/loop/posts/{id}",
            get(routes::loops::get_loop_post)
                .patch(routes::loops::update_loop_post)
                .delete(routes::loops::delete_loop_post),
        )
        .route("/api/internal/items", post(routes::items::create_item))
        .route(
            "/api/internal/feed/items",
            post(routes::feed::create_feed_item),
        )
        .route(
            "/api/internal/feed/weeklies",
            post(routes::feed::create_weekly_digest),
        )
        .route(
            "/api/internal/loop/posts/pending-preferences",
            get(routes::loops::list_pending_loop_posts_internal),
        )
        .route(
            "/api/internal/loop/posts/{id}/preference-result",
            post(routes::loops::update_loop_post_preference_result_internal),
        )
        .route(
            "/api/internal/memory/entries",
            post(routes::memory::create_memory_entry_internal),
        )
        .route(
            "/api/internal/memory/profile/{user_id}",
            get(routes::memory::get_memory_profile_internal),
        )
        .route(
            "/api/internal/items/multipart",
            post(routes::items::create_item_multipart),
        )
        .route(
            "/api/internal/items/pending",
            get(routes::internal_api::list_pending_items),
        )
        .route(
            "/api/internal/items/{id}/complete",
            post(routes::internal_api::complete_item),
        )
        .route("/api/internal/upload", post(routes::upload::upload_audio))
        .route(
            "/api/internal/dedup/check",
            post(routes::dedup::check_files),
        )
        .route("/api/internal/dedup/mark", post(routes::dedup::mark_file))
        .route(
            "/api/internal/items/{id}/sources",
            post(routes::internal_api::push_sources),
        )
        .route(
            "/api/items/{id}/sources",
            get(routes::internal_api::get_sources),
        )
        .route(
            "/api/history",
            get(routes::history::get_history)
                .post(routes::history::mark_played)
                .delete(routes::history::unmark_played),
        )
        .route("/api/auth/login", post(routes::auth::login))
        .route(
            "/api/admin/users",
            post(routes::auth::create_user).get(routes::auth::list_users),
        )
        .route(
            "/api/admin/items/{id}",
            axum::routing::patch(routes::admin::update_item),
        )
        .route(
            "/api/admin/items/{id}/regenerate",
            post(routes::admin::regenerate_item),
        )
        .route("/api/admin/export", get(routes::admin::export_items))
        .route(
            "/admin",
            get(move || async move {
                match tokio::fs::read_to_string(&admin_index).await {
                    Ok(html) => axum::response::Html(html).into_response(),
                    Err(_) => StatusCode::NOT_FOUND.into_response(),
                }
            }),
        )
        .route(
            "/feed",
            get({
                let feed_index = feed_index.clone();
                move || async move {
                    match tokio::fs::read_to_string(&feed_index).await {
                        Ok(html) => axum::response::Html(html).into_response(),
                        Err(_) => StatusCode::NOT_FOUND.into_response(),
                    }
                }
            }),
        )
        .route(
            "/feed/",
            get(move || async move {
                match tokio::fs::read_to_string(&feed_index).await {
                    Ok(html) => axum::response::Html(html).into_response(),
                    Err(_) => StatusCode::NOT_FOUND.into_response(),
                }
            }),
        )
        .route(
            "/loop",
            get({
                let loop_index = loop_index.clone();
                move || async move {
                    match tokio::fs::read_to_string(&loop_index).await {
                        Ok(html) => axum::response::Html(html).into_response(),
                        Err(_) => StatusCode::NOT_FOUND.into_response(),
                    }
                }
            }),
        )
        .route(
            "/loop/",
            get({
                let loop_index = loop_index.clone();
                move || async move {
                    match tokio::fs::read_to_string(&loop_index).await {
                        Ok(html) => axum::response::Html(html).into_response(),
                        Err(_) => StatusCode::NOT_FOUND.into_response(),
                    }
                }
            }),
        )
        .route(
            "/focus",
            get({
                let focus_index = focus_index.clone();
                move || async move {
                    match tokio::fs::read_to_string(&focus_index).await {
                        Ok(html) => axum::response::Html(html).into_response(),
                        Err(_) => StatusCode::NOT_FOUND.into_response(),
                    }
                }
            }),
        )
        .route(
            "/focus/",
            get(move || async move {
                match tokio::fs::read_to_string(&focus_index).await {
                    Ok(html) => axum::response::Html(html).into_response(),
                    Err(_) => StatusCode::NOT_FOUND.into_response(),
                }
            }),
        )
        .nest_service("/audio", ServeDir::new(audio_dir))
        .layer(build_cors_layer())
        .with_state(state)
        .fallback_service(
            ServeDir::new(&static_dir)
                .not_found_service(tower_http::services::ServeFile::new(static_index)),
        )
        .layer(DefaultBodyLimit::max(100 * 1024 * 1024));

    let port = std::env::var("PORT")
        .unwrap_or_else(|_| "8899".to_string())
        .parse::<u16>()
        .unwrap_or(8899);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
