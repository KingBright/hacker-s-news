use sqlx::migrate::MigrateDatabase;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::{Pool, Sqlite};
use std::env;
use std::time::Duration;

pub type DbPool = Pool<Sqlite>;

pub async fn init_db() -> Result<DbPool, sqlx::Error> {
    let home_dir = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let default_db_path = format!("{}/.freshloop/data/freshloop.db", home_dir);

    // Ensure parent dir exists
    if let Some(parent) = std::path::Path::new(&default_db_path).parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent).map_err(|e| sqlx::Error::Configuration(e.into()))?;
        }
    }

    let database_url =
        env::var("DATABASE_URL").unwrap_or_else(|_| format!("sqlite:{}", default_db_path));

    // Create database file if not exists
    if !Sqlite::database_exists(&database_url)
        .await
        .unwrap_or(false)
    {
        Sqlite::create_database(&database_url).await?;
    }

    // Configure connection pool with proper settings
    let pool = SqlitePoolOptions::new()
        .max_connections(10)              // Maximum concurrent connections
        .min_connections(2)               // Minimum idle connections
        .acquire_timeout(Duration::from_secs(30))  // Timeout for acquiring connection
        .idle_timeout(Some(Duration::from_secs(600))) // Close idle connections after 10 mins
        .max_lifetime(Some(Duration::from_secs(3600))) // Connection lifetime 1 hour
        .connect(&database_url)
        .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS items (
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
            status TEXT DEFAULT 'published',
            category TEXT
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
        CREATE TABLE IF NOT EXISTS user_history (
            user_id TEXT NOT NULL,
            item_id TEXT NOT NULL,
            played_at INTEGER,
            PRIMARY KEY (user_id, item_id)
        );
        CREATE TABLE IF NOT EXISTS users (
            id TEXT PRIMARY KEY,
            username TEXT UNIQUE NOT NULL,
            password_hash TEXT NOT NULL,
            created_at INTEGER
        );
        "#,
    )
    .execute(&pool)
    .await?;

    // Attempt migrations for existing database
    // We ignore errors if columns already exist
    let _ = sqlx::query("ALTER TABLE items ADD COLUMN rating INTEGER")
        .execute(&pool)
        .await;
    let _ = sqlx::query("ALTER TABLE items ADD COLUMN tags TEXT")
        .execute(&pool)
        .await;
    let _ = sqlx::query("ALTER TABLE items ADD COLUMN is_deleted BOOLEAN DEFAULT 0")
        .execute(&pool)
        .await;
    let _ = sqlx::query("ALTER TABLE items ADD COLUMN duration_sec INTEGER")
        .execute(&pool)
        .await;
    let _ = sqlx::query("ALTER TABLE items ADD COLUMN status TEXT DEFAULT 'published'")
        .execute(&pool)
        .await;
    let _ = sqlx::query("ALTER TABLE items ADD COLUMN category TEXT")
        .execute(&pool)
        .await; // New category column

    // Add unique index on original_url for deduplication (idempotent)
    // First, clean up existing data to prevent index creation failures
    // 1. Remove duplicate original_urls (keep the newest by publish_time)
    let _ = sqlx::query(
        r#"
        DELETE FROM items WHERE id NOT IN (
            SELECT id FROM (
                SELECT id, ROW_NUMBER() OVER (
                    PARTITION BY original_url ORDER BY publish_time DESC, created_at DESC
                ) as rn
                FROM items
                WHERE original_url IS NOT NULL AND original_url != ''
            ) WHERE rn = 1
        ) AND original_url IS NOT NULL AND original_url != ''
        "#
    )
    .execute(&pool)
    .await;

    // 2. Create the unique index (IF NOT EXISTS makes this idempotent)
    let _ = sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_items_original_url ON items(original_url)"
    )
    .execute(&pool)
    .await;

    Ok(pool)
}
