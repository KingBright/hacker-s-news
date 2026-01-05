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
            created_at INTEGER,
            rating INTEGER,
            tags TEXT,
            is_deleted BOOLEAN DEFAULT 0,
            status TEXT DEFAULT 'published'
        );
        CREATE TABLE IF NOT EXISTS source_items (
            id TEXT PRIMARY KEY,
            url TEXT UNIQUE NOT NULL,
            category TEXT NOT NULL,
            created_at INTEGER
        );
        CREATE TABLE IF NOT EXISTS item_sources (
            id TEXT PRIMARY KEY,
            item_id TEXT NOT NULL,
            source_url TEXT NOT NULL,
            source_title TEXT,
            source_summary TEXT,
            created_at INTEGER
        );
        "#
    )
    .execute(&pool)
    .await?;

    // Attempt migrations for existing database
    // We ignore errors if columns already exist
    let _ = sqlx::query("ALTER TABLE items ADD COLUMN rating INTEGER").execute(&pool).await;
    let _ = sqlx::query("ALTER TABLE items ADD COLUMN tags TEXT").execute(&pool).await;
    let _ = sqlx::query("ALTER TABLE items ADD COLUMN is_deleted BOOLEAN DEFAULT 0").execute(&pool).await;
    let _ = sqlx::query("ALTER TABLE items ADD COLUMN duration_sec INTEGER").execute(&pool).await;
    let _ = sqlx::query("ALTER TABLE items ADD COLUMN status TEXT DEFAULT 'published'").execute(&pool).await; // New status column

    Ok(pool)
}
