use axum::{
    routing::{get, post},
    Router,
};
use std::net::SocketAddr;
use tokio::fs;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod db;
mod routes;

use db::DbPool;

#[derive(Clone)]
pub struct AppState {
    pub db: DbPool,
    pub api_key: String,
    pub audio_dir: String,
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

    // Ensure audio directory exists
    let audio_dir = std::env::var("AUDIO_DIR").unwrap_or_else(|_| "audio".to_string());
    fs::create_dir_all(&audio_dir).await.expect("Failed to create audio dir");

    let api_key = std::env::var("NEXUS_KEY").unwrap_or_else(|_| "my-secret-key-123".to_string());

    let state = AppState {
        db: db_pool,
        api_key,
        audio_dir: audio_dir.clone(),
    };

    let app = Router::new()
        .route("/api/items", get(routes::items::list_items))
        .route("/api/internal/items", post(routes::items::create_item))
        .route("/api/internal/upload", post(routes::upload::upload_audio))
        .nest_service("/audio", ServeDir::new(audio_dir))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));
    tracing::info!("listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
