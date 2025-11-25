use sqlx::sqlite::SqlitePool;
use sqlx::migrate::MigrateDatabase;
use sqlx::{Pool, Sqlite};
use std::env;

pub type DbPool = Pool<Sqlite>;

pub async fn init_db() -> Result<DbPool, sqlx::Error> {
    let database_url = env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:freshloop.db".to_string());

    // Create database file if not exists
    if !Sqlite::database_exists(&database_url).await.unwrap_or(false) {
        Sqlite::create_database(&database_url).await?;
    }

    let pool = SqlitePool::connect(&database_url).await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS items (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            summary TEXT,
            original_url TEXT,
            cover_image_url TEXT,
            audio_url TEXT,
            publish_time INTEGER,
            created_at INTEGER
        );
        "#
    )
    .execute(&pool)
    .await?;

    Ok(pool)
}
