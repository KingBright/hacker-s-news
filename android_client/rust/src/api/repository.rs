use rusqlite::{params, Connection};
use std::sync::Mutex;
use crate::models::Item;
use flutter_rust_bridge::frb;

#[frb(opaque)]
pub struct Repository {
    conn: Mutex<Connection>,
}

impl Repository {
    #[frb(sync)]
    pub fn new(db_path: String) -> anyhow::Result<Self> {
        let conn = Connection::open(&db_path)?;
        
        conn.execute(
            "CREATE TABLE IF NOT EXISTS queue (
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
                is_deleted INTEGER,
                duration_sec INTEGER,
                status TEXT,
                category TEXT
            )",
            [],
        )?;
        
        conn.execute(
            "CREATE TABLE IF NOT EXISTS played_history (
                id TEXT PRIMARY KEY,
                played_at INTEGER NOT NULL
            )",
            [],
        )?;
        
        Ok(Self { conn: Mutex::new(conn) })
    }

    pub fn insert_queue(&self, items: Vec<Item>) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "INSERT OR REPLACE INTO queue (
                id, title, summary, original_url, cover_image_url, audio_url,
                publish_time, created_at, rating, tags, is_deleted, duration_sec, status, category
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        )?;
        
        for item in items {
            stmt.execute(params![
                item.id,
                item.title,
                item.summary,
                item.original_url,
                item.cover_image_url,
                item.audio_url,
                item.publish_time,
                item.created_at,
                item.rating,
                item.tags,
                item.is_deleted.map(|d| if d { 1 } else { 0 }),
                item.duration_sec,
                item.status,
                item.category,
            ])?;
        }
        
        Ok(())
    }

    pub fn get_queue(&self) -> anyhow::Result<Vec<Item>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, title, summary, original_url, cover_image_url, audio_url,
             publish_time, created_at, rating, tags, is_deleted, duration_sec, status, category
             FROM queue ORDER BY publish_time DESC"
        )?;
        
        let item_iter = stmt.query_map([], |row| {
            let is_deleted_int: Option<i32> = row.get(10)?;
            Ok(Item {
                id: row.get(0)?,
                title: row.get(1)?,
                summary: row.get(2)?,
                original_url: row.get(3)?,
                cover_image_url: row.get(4)?,
                audio_url: row.get(5)?,
                publish_time: row.get(6)?,
                created_at: row.get(7)?,
                rating: row.get(8)?,
                tags: row.get(9)?,
                is_deleted: is_deleted_int.map(|d| d != 0),
                duration_sec: row.get(11)?,
                status: row.get(12)?,
                category: row.get(13)?,
            })
        })?;
        
        let mut items = Vec::new();
        for item in item_iter {
            items.push(item?);
        }
        Ok(items)
    }

    pub fn mark_played(&self, id: String) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO played_history (id, played_at) VALUES (?, strftime('%s', 'now'))",
            params![id],
        )?;
        Ok(())
    }
    
    pub fn get_played_ids(&self) -> anyhow::Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT id FROM played_history")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        let mut ids = Vec::new();
        for row in rows {
            ids.push(row?);
        }
        Ok(ids)
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_repository() {
        let repo = Repository::new(":memory:".to_string()).unwrap();
        let item = Item {
            id: "1".to_string(),
            title: "Test".to_string(),
            summary: None,
            original_url: None,
            cover_image_url: None,
            audio_url: None,
            publish_time: Some(100),
            created_at: Some(100),
            rating: None,
            tags: None,
            is_deleted: Some(false),
            duration_sec: None,
            status: None,
            category: None,
        };
        repo.insert_queue(vec![item]).unwrap();
        let q = repo.get_queue().unwrap();
        assert_eq!(q.len(), 1);
        assert_eq!(q[0].id, "1");
        repo.mark_played("1".to_string()).unwrap();
        let played = repo.get_played_ids().unwrap();
        assert_eq!(played.len(), 1);
        assert_eq!(played[0], "1");
    }
}
