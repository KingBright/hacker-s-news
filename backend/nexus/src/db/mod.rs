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
        .max_connections(10) // Maximum concurrent connections
        .min_connections(2) // Minimum idle connections
        .acquire_timeout(Duration::from_secs(30)) // Timeout for acquiring connection
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
        CREATE TABLE IF NOT EXISTS feed_items (
            id TEXT PRIMARY KEY,
            product_line TEXT NOT NULL,
            item_type TEXT NOT NULL,
            primary_mode TEXT NOT NULL,
            title TEXT NOT NULL,
            subtitle TEXT,
            source_name TEXT,
            source_url TEXT,
            original_url TEXT,
            canonical_url TEXT,
            content_hash TEXT,
            publish_time INTEGER,
            created_at INTEGER,
            updated_at INTEGER,
            has_audio BOOLEAN DEFAULT 0,
            audio_url TEXT,
            duration_sec INTEGER,
            reading_time_min INTEGER,
            quality_score INTEGER,
            tags TEXT,
            status TEXT DEFAULT 'published'
        );
        CREATE TABLE IF NOT EXISTS feed_item_contents (
            item_id TEXT PRIMARY KEY,
            original_html TEXT,
            reader_markdown TEXT,
            plain_text TEXT,
            compressed_markdown TEXT,
            audio_script TEXT,
            key_points_json TEXT,
            created_at INTEGER,
            updated_at INTEGER
        );
        CREATE TABLE IF NOT EXISTS feed_reading_progress (
            user_id TEXT NOT NULL,
            item_id TEXT NOT NULL,
            mode TEXT NOT NULL,
            scroll_ratio REAL,
            anchor TEXT,
            updated_at INTEGER,
            read_at INTEGER,
            PRIMARY KEY (user_id, item_id, mode)
        );
        CREATE TABLE IF NOT EXISTS weekly_digests (
            id TEXT PRIMARY KEY,
            feed_item_id TEXT,
            week_start INTEGER NOT NULL,
            week_end INTEGER NOT NULL,
            title TEXT NOT NULL,
            digest_markdown TEXT,
            audio_script TEXT,
            audio_url TEXT,
            duration_sec INTEGER,
            included_item_ids_json TEXT,
            themes_json TEXT,
            created_at INTEGER,
            status TEXT DEFAULT 'published'
        );
        CREATE TABLE IF NOT EXISTS loop_posts (
            id TEXT PRIMARY KEY,
            user_id TEXT NOT NULL,
            post_type TEXT NOT NULL,
            feedback_mode TEXT DEFAULT 'balance',
            title TEXT,
            body TEXT NOT NULL,
            visibility TEXT NOT NULL DEFAULT 'private',
            source_ref TEXT,
            memory_entry_id TEXT,
            preference_status TEXT DEFAULT 'pending',
            preference_extracted_at INTEGER,
            preference_error TEXT,
            created_at INTEGER,
            updated_at INTEGER,
            status TEXT DEFAULT 'published'
        );
        CREATE TABLE IF NOT EXISTS loop_post_references (
            id TEXT PRIMARY KEY,
            post_id TEXT NOT NULL,
            source_type TEXT NOT NULL,
            source_id TEXT,
            source_url TEXT,
            title TEXT,
            quote_text TEXT,
            start_ms INTEGER,
            end_ms INTEGER,
            created_at INTEGER
        );
        CREATE TABLE IF NOT EXISTS agent_jobs (
            id TEXT PRIMARY KEY,
            job_type TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'queued',
            priority INTEGER DEFAULT 0,
            run_after INTEGER,
            lease_owner TEXT,
            lease_token TEXT,
            lease_expires_at INTEGER,
            attempt_count INTEGER DEFAULT 0,
            max_attempts INTEGER DEFAULT 3,
            context_json TEXT,
            input_json TEXT,
            artifact_json TEXT,
            result_ref TEXT,
            last_error TEXT,
            created_at INTEGER,
            updated_at INTEGER,
            completed_at INTEGER
        );
        CREATE TABLE IF NOT EXISTS agent_job_events (
            id TEXT PRIMARY KEY,
            job_id TEXT NOT NULL,
            event_type TEXT NOT NULL,
            actor TEXT,
            message TEXT,
            payload_json TEXT,
            created_at INTEGER
        );
        CREATE TABLE IF NOT EXISTS voice_jobs (
            id TEXT PRIMARY KEY,
            target_type TEXT NOT NULL,
            target_id TEXT NOT NULL,
            product_line TEXT NOT NULL,
            voice_kind TEXT NOT NULL,
            text TEXT NOT NULL,
            file_prefix TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'queued',
            priority INTEGER DEFAULT 0,
            run_after INTEGER,
            lease_owner TEXT,
            lease_token TEXT,
            lease_expires_at INTEGER,
            attempt_count INTEGER DEFAULT 0,
            max_attempts INTEGER DEFAULT 5,
            audio_url TEXT,
            duration_sec INTEGER,
            last_error TEXT,
            created_at INTEGER,
            updated_at INTEGER,
            completed_at INTEGER
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
    let _ = sqlx::query("ALTER TABLE feed_items ADD COLUMN duration_sec INTEGER")
        .execute(&pool)
        .await;
    let _ = sqlx::query("ALTER TABLE feed_item_contents ADD COLUMN audio_script TEXT")
        .execute(&pool)
        .await;
    let _ = sqlx::query("ALTER TABLE weekly_digests ADD COLUMN duration_sec INTEGER")
        .execute(&pool)
        .await;

    // Never delete published history merely because two programs cite one source.
    repair_radio_program_sources(&pool).await?;
    let _ = sqlx::query("CREATE INDEX IF NOT EXISTS idx_items_original_url ON items(original_url)")
        .execute(&pool)
        .await;
    let _ = sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_items_publish_queue ON items(publish_time ASC, created_at ASC, id ASC)",
    )
    .execute(&pool)
    .await;
    let _ = sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_user_history_user_item ON user_history(user_id, item_id)",
    )
    .execute(&pool)
    .await;

    let _ = sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_feed_items_original_url ON feed_items(original_url) WHERE original_url IS NOT NULL AND original_url != ''"
    )
    .execute(&pool)
    .await;
    let _ = sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_feed_items_product_time ON feed_items(product_line, publish_time DESC)"
    )
    .execute(&pool)
    .await;
    let _ = sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_feed_items_type_time ON feed_items(item_type, publish_time DESC)"
    )
    .execute(&pool)
    .await;
    let _ = sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_weekly_digests_range ON weekly_digests(week_start DESC, week_end DESC)"
    )
    .execute(&pool)
    .await;
    let _ = sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_weekly_digests_unique_range ON weekly_digests(week_start, week_end)"
    )
    .execute(&pool)
    .await;
    let _ = sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_loop_posts_user_time ON loop_posts(user_id, created_at DESC)"
    )
    .execute(&pool)
    .await;
    let _ = sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_loop_posts_status ON loop_posts(status, created_at DESC)",
    )
    .execute(&pool)
    .await;
    let _ =
        sqlx::query("ALTER TABLE loop_posts ADD COLUMN preference_status TEXT DEFAULT 'pending'")
            .execute(&pool)
            .await;
    let _ = sqlx::query("ALTER TABLE loop_posts ADD COLUMN preference_extracted_at INTEGER")
        .execute(&pool)
        .await;
    let _ = sqlx::query("ALTER TABLE loop_posts ADD COLUMN preference_error TEXT")
        .execute(&pool)
        .await;
    let _ = sqlx::query("ALTER TABLE loop_posts ADD COLUMN feedback_mode TEXT DEFAULT 'balance'")
        .execute(&pool)
        .await;
    let _ = sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_loop_post_references_post ON loop_post_references(post_id)",
    )
    .execute(&pool)
    .await;
    let _ = sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_loop_posts_preference_status ON loop_posts(preference_status, created_at ASC)"
    )
    .execute(&pool)
    .await;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_agent_jobs_result_ref ON agent_jobs(result_ref)")
        .execute(&pool)
        .await?;
    let _ = sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_agent_jobs_status_run ON agent_jobs(status, run_after, priority DESC, created_at ASC)"
    )
    .execute(&pool)
    .await;
    let _ = sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_agent_jobs_lease ON agent_jobs(lease_token, lease_expires_at)"
    )
    .execute(&pool)
    .await;
    let _ = sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_agent_job_events_job ON agent_job_events(job_id, created_at ASC)"
    )
    .execute(&pool)
    .await;
    let _ = sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_voice_jobs_status_run ON voice_jobs(status, run_after, priority DESC, created_at ASC)"
    )
    .execute(&pool)
    .await;
    let _ = sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_voice_jobs_target ON voice_jobs(target_type, target_id)",
    )
    .execute(&pool)
    .await;

    Ok(pool)
}

async fn repair_radio_program_sources(pool: &DbPool) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    // Repair only missing category items whose source was taken by a remastered
    // program. Their original IDs, audio and timestamps are retained in job audit.
    sqlx::query(r#"
        CREATE TEMP TABLE radio_program_recovery AS
        SELECT substr(j.result_ref,12) AS id,json_extract(j.artifact_json,'$.title') AS title,json_extract(j.artifact_json,'$.script') AS summary,
          COALESCE(json_extract(j.artifact_json,'$.original_url'),json_extract(j.artifact_json,'$.sources[0].url')) AS original_url,
          (SELECT audio_url FROM voice_jobs v WHERE v.target_id=substr(j.result_ref,12) AND v.target_type='radio_item' AND v.status='completed' ORDER BY v.completed_at DESC LIMIT 1) AS audio_url,
          COALESCE(json_extract(j.artifact_json,'$.publish_time'),j.completed_at) AS publish_time,j.completed_at AS created_at,
          (SELECT duration_sec FROM voice_jobs v WHERE v.target_id=substr(j.result_ref,12) AND v.target_type='radio_item' AND v.status='completed' ORDER BY v.completed_at DESC LIMIT 1) AS duration_sec,
          'published' AS status,json_extract(j.artifact_json,'$.category') AS category,json_extract(j.artifact_json,'$.tags') AS tags
        FROM agent_jobs j WHERE j.job_type='radio_episode' AND j.status='completed' AND json_valid(j.artifact_json)
          AND j.result_ref LIKE 'radio_item:%'
          AND NOT EXISTS(SELECT 1 FROM items WHERE id=substr(j.result_ref,12))
          AND EXISTS(SELECT 1 FROM items p WHERE p.original_url=COALESCE(json_extract(j.artifact_json,'$.original_url'),json_extract(j.artifact_json,'$.sources[0].url'))
            AND EXISTS(SELECT 1 FROM json_each(CASE WHEN json_valid(p.tags) THEN p.tags ELSE '[]' END) WHERE value='radio:program'))
    "#).execute(&mut *tx).await?;
    // Snapshot candidates first, then release the URL before restoring its owner.
    // Fresh databases enforce UNIQUE(original_url); older installations may not.
    sqlx::query("UPDATE items SET original_url=NULL WHERE EXISTS(SELECT 1 FROM json_each(CASE WHEN json_valid(items.tags) THEN items.tags ELSE '[]' END) WHERE value='radio:program')").execute(&mut *tx).await?;
    sqlx::query("INSERT INTO items (id,title,summary,original_url,audio_url,publish_time,created_at,duration_sec,status,category,tags) SELECT id,title,summary,original_url,audio_url,publish_time,created_at,duration_sec,status,category,tags FROM radio_program_recovery")
        .execute(&mut *tx).await?;
    sqlx::query("DROP TABLE radio_program_recovery")
        .execute(&mut *tx)
        .await?;
    tx.commit().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn program_repair_restores_originals_with_unique_urls_and_is_idempotent() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::raw_sql(r#"
            CREATE TABLE items (id TEXT PRIMARY KEY, title TEXT, summary TEXT, original_url TEXT UNIQUE,
                audio_url TEXT, publish_time INTEGER, created_at INTEGER, duration_sec INTEGER,
                status TEXT, category TEXT, tags TEXT);
            CREATE TABLE agent_jobs (job_type TEXT, status TEXT, result_ref TEXT, artifact_json TEXT, completed_at INTEGER);
            CREATE TABLE voice_jobs (target_id TEXT, target_type TEXT, status TEXT, audio_url TEXT, duration_sec INTEGER, completed_at INTEGER);
            CREATE TABLE item_sources (item_id TEXT, source_url TEXT);
            INSERT INTO items (id,title,original_url,tags) VALUES
                ('program','节目','https://example.com/story','["radio:program"]'),
                ('untouched','历史节目','https://example.com/older','[]');
            INSERT INTO agent_jobs VALUES ('radio_episode','completed','radio_item:original',
                '{"title":"原节目","script":"原文稿","category":"AI前沿","tags":["radio:edition:morning"],"publish_time":123,"sources":[{"url":"https://example.com/story"}]}',124);
            INSERT INTO voice_jobs VALUES ('original','radio_item','completed','/audio/original.mp3',88,125);
            INSERT INTO item_sources VALUES ('original','https://example.com/story'),('program','https://example.com/story');
        "#).execute(&pool).await.unwrap();

        for _ in 0..2 {
            repair_radio_program_sources(&pool).await.unwrap();
            let original: (String,String,String,i64,i64,String) = sqlx::query_as(
                "SELECT title,summary,audio_url,publish_time,duration_sec,tags FROM items WHERE id='original'")
                .fetch_one(&pool).await.unwrap();
            assert_eq!(
                original,
                (
                    "原节目".into(),
                    "原文稿".into(),
                    "/audio/original.mp3".into(),
                    123,
                    88,
                    "[\"radio:edition:morning\"]".into()
                )
            );
            let program_url: Option<String> =
                sqlx::query_scalar("SELECT original_url FROM items WHERE id='program'")
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert!(program_url.is_none());
            assert_eq!(
                sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM items")
                    .fetch_one(&pool)
                    .await
                    .unwrap(),
                3
            );
            assert_eq!(
                sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM item_sources")
                    .fetch_one(&pool)
                    .await
                    .unwrap(),
                2
            );
            assert_eq!(
                sqlx::query_scalar::<_, String>(
                    "SELECT original_url FROM items WHERE id='untouched'"
                )
                .fetch_one(&pool)
                .await
                .unwrap(),
                "https://example.com/older"
            );
        }
    }
}
