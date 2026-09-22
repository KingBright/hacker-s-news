//! External-agent inputs. No scoring, summarization, clustering or LLM calls.
use super::{
    fetch_feed_entries, fetch_url_bytes, parse_opml_sources, ContentSource, FeedFetchOptions,
    FetchedEntry, ProductLine,
};
use crate::core::config::Config;
use anyhow::{anyhow, Result};
use futures::{stream, StreamExt};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{path::Path, sync::Arc};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceRecord {
    pub id: String,
    pub source: ContentSource,
    pub entry: FetchedEntry,
    pub fetched_at: i64,
    pub article_fetched_at: Option<i64>,
    pub original_html: Option<String>,
    pub reader_markdown: Option<String>,
    pub content_hash: Option<String>,
    pub media: Vec<MediaLink>,
    pub fetch_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaLink {
    pub kind: String,
    pub url: String,
    pub description: String,
    pub inspected: bool,
}

#[derive(Default, Serialize)]
pub struct IngestReport {
    pub feeds_ok: usize,
    pub entries_cached: usize,
    pub errors: Vec<String>,
}

pub struct SourceCache {
    db: sled::Db,
    pub refresh_lock: tokio::sync::Mutex<()>,
    article_lock: tokio::sync::Mutex<()>,
}

impl SourceCache {
    pub fn open(path: &Path) -> Result<Self> {
        Ok(Self {
            db: sled::open(path)?,
            refresh_lock: tokio::sync::Mutex::new(()),
            article_lock: tokio::sync::Mutex::new(()),
        })
    }

    pub fn get(&self, id: &str) -> Result<Option<SourceRecord>> {
        self.db
            .get(id)?
            .map(|v| serde_json::from_slice(&v).map_err(Into::into))
            .transpose()
    }

    fn put(&self, record: &SourceRecord) -> Result<()> {
        self.db
            .insert(record.id.as_bytes(), serde_json::to_vec(record)?)?;
        Ok(())
    }

    pub fn list(
        &self,
        product: Option<&str>,
        since: i64,
        offset: usize,
        limit: usize,
    ) -> Result<serde_json::Value> {
        let mut records = Vec::new();
        for row in self.db.iter() {
            let (_, value) = row?;
            let r: SourceRecord = serde_json::from_slice(&value)?;
            let line = match r.source.product_line {
                ProductLine::Radio => "radio",
                ProductLine::CuratedFeed => "reading",
            };
            if r.fetched_at < since || product.is_some_and(|p| p != line) {
                continue;
            }
            records.push((
                r.fetched_at,
                r.id.clone(),
                serde_json::json!({
                    "id": r.id, "product_line": line, "source_group": r.source.source_group,
                    "tags": r.source.tags, "title": r.entry.title, "url": r.entry.link,
                    "source_name": r.entry.source_name, "published_at": r.entry.pub_date,
                    "fetched_at": r.fetched_at, "article_fetched_at": r.article_fetched_at,
                    "fetch_error": r.fetch_error, "media_count": r.media.len()
                }),
            ));
        }
        records.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        let total = records.len();
        Ok(
            serde_json::json!({"total":total,"offset":offset,"items":records.into_iter().skip(offset).take(limit.clamp(1,100)).map(|r|r.2).collect::<Vec<_>>() }),
        )
    }

    fn cache_entry(&self, source: ContentSource, entry: FetchedEntry) -> Result<()> {
        let key = format!("{:?}:{}", source.product_line, entry.link);
        let id = hex::encode(Sha256::digest(key.as_bytes()));
        // Rediscovery never erases a successfully downloaded article or changes first-seen time.
        let old = self.get(&id)?;
        let mut media = discover_media(&entry.description, &entry.link);
        media.extend(entry.media.clone());
        let mut seen = std::collections::HashSet::new();
        media.retain(|m| seen.insert(m.url.clone()));
        let mut r = old.unwrap_or(SourceRecord {
            id,
            source: source.clone(),
            entry: entry.clone(),
            fetched_at: chrono::Utc::now().timestamp(),
            article_fetched_at: None,
            original_html: None,
            reader_markdown: None,
            content_hash: None,
            media,
            fetch_error: None,
        });
        r.source = source;
        r.entry = entry;
        self.put(&r)
    }

    pub async fn refresh(&self, config: &Config) -> Result<IngestReport> {
        let _guard = self
            .refresh_lock
            .try_lock()
            .map_err(|_| anyhow!("source refresh already running"))?;
        let options = FeedFetchOptions::new(config.http_proxy.clone()).with_prefer_proxy(
            config
                .curated_feed
                .as_ref()
                .and_then(|f| f.prefer_proxy)
                .unwrap_or(false),
        );
        let mut report = IngestReport::default();
        let mut sources = Vec::new();
        if let Some(feed) = config.curated_feed.as_ref().filter(|f| f.enabled) {
            for configured in feed.feeds.iter().flatten() {
                let group = configured
                    .source_group
                    .as_deref()
                    .or(feed.source_group.as_deref());
                if configured.kind.as_deref() == Some("opml") || configured.url.ends_with(".opml") {
                    match fetch_url_bytes(&configured.url, &options).await {
                        Ok(bytes) => match parse_opml_sources(&bytes, group) {
                            Ok(mut expanded) => {
                                for s in &mut expanded {
                                    s.tags = configured.tags.clone().unwrap_or_default();
                                }
                                sources.extend(expanded);
                            }
                            Err(e) => report.errors.push(format!("{}: {e}", configured.url)),
                        },
                        Err(e) => report.errors.push(format!("{}: {e}", configured.url)),
                    }
                } else {
                    let mut source = ContentSource::new(
                        &configured.url,
                        configured.name.as_deref().unwrap_or(&configured.url),
                        &configured.url,
                        ProductLine::CuratedFeed,
                    );
                    source.source_group = group.map(str::to_owned);
                    source.tags = configured.tags.clone().unwrap_or_default();
                    sources.push(source);
                }
            }
        }
        // Reading owns overlapping subscriptions, matching the self-driven pipeline.
        for url in config.rss_feeds.iter().flatten() {
            if !sources.iter().any(|s| &s.url == url) {
                sources.push(ContentSource::new(url, url, url, ProductLine::Radio));
            }
        }
        let mut seen = std::collections::HashSet::new();
        sources.retain(|s| seen.insert(s.url.clone()));
        let results = stream::iter(sources)
            .map(|source| {
                let options = options.clone();
                async move {
                    let result = fetch_feed_entries(&source.url, &options).await;
                    (source, result)
                }
            })
            .buffer_unordered(8)
            .collect::<Vec<_>>()
            .await;
        let cutoff = chrono::Utc::now() - chrono::Duration::days(14);
        for (source, result) in results {
            match result {
                Ok(entries) => {
                    report.feeds_ok += 1;
                    for entry in entries.into_iter().take(100) {
                        if entry
                            .pub_date
                            .as_deref()
                            .and_then(|d| chrono::DateTime::parse_from_rfc3339(d).ok())
                            .is_some_and(|d| d < cutoff)
                        {
                            continue;
                        }
                        self.cache_entry(source.clone(), entry)?;
                        report.entries_cached += 1;
                    }
                }
                Err(e) => report.errors.push(format!("{}: {e}", source.url)),
            }
        }
        // Bounded retention: stale inputs are not an ever-growing article archive.
        for row in self.db.iter() {
            let (key, value) = row?;
            let r: SourceRecord = serde_json::from_slice(&value)?;
            if r.fetched_at < cutoff.timestamp() {
                let media = self.db.open_tree("media")?;
                for entry in media.scan_prefix(format!("{}:", r.id)) {
                    let (key, _) = entry?;
                    media.remove(key)?;
                }
                self.db.remove(key)?;
            }
        }
        self.db.flush_async().await?;
        Ok(report)
    }

    pub async fn fetch_media(
        &self,
        id: &str,
        index: usize,
        options: &FeedFetchOptions,
    ) -> Result<Vec<u8>> {
        let record = self.get(id)?.ok_or_else(|| anyhow!("unknown source id"))?;
        let media = record
            .media
            .get(index)
            .ok_or_else(|| anyhow!("unknown media index"))?;
        let tree = self.db.open_tree("media")?;
        let key = format!(
            "{}:{}",
            id,
            hex::encode(Sha256::digest(media.url.as_bytes()))
        );
        if let Some(bytes) = tree.get(key.as_bytes())? {
            return Ok(bytes.to_vec());
        }
        // Use the same status checks, timeout, proxy policy and 8 MiB bound as pages.
        // Larger recordings stay as source links for agent streaming/transcript tools.
        let bytes = fetch_url_bytes(&media.url, options).await?;
        tree.insert(key.as_bytes(), bytes.as_ref())?;
        tree.flush_async().await?;
        Ok(bytes.to_vec())
    }

    pub async fn fetch_article(
        &self,
        id: &str,
        options: &FeedFetchOptions,
    ) -> Result<SourceRecord> {
        let _guard = self.article_lock.lock().await;
        let mut r = self.get(id)?.ok_or_else(|| anyhow!("unknown source id"))?;
        if r.article_fetched_at.is_some() {
            return Ok(r);
        }
        match fetch_url_bytes(&r.entry.link, options).await {
            Ok(bytes) => {
                let html = std::str::from_utf8(&bytes)
                    .map_err(|_| anyhow!("non-UTF8 source; inspect original with agent tools"))?
                    .to_string();
                r.media = discover_media(&html, &r.entry.link);
                r.media.extend(r.entry.media.clone());
                let mut seen = std::collections::HashSet::new();
                r.media.retain(|m| seen.insert(m.url.clone()));
                r.reader_markdown = Some(html2md::parse_html(&html));
                r.content_hash = Some(hex::encode(Sha256::digest(&bytes)));
                r.original_html = Some(html);
                r.article_fetched_at = Some(chrono::Utc::now().timestamp());
                r.fetch_error = None;
            }
            Err(e) => {
                r.fetch_error = Some(e.to_string());
            }
        }
        self.put(&r)?;
        self.db.flush_async().await?;
        Ok(r)
    }
}

// Discovery only: alt text is metadata, never evidence that a model inspected media.
pub fn discover_media(html: &str, base: &str) -> Vec<MediaLink> {
    let Ok(base) = reqwest::Url::parse(base) else {
        return Vec::new();
    };
    let tags = regex::Regex::new(r"(?is)<(img|audio|video|source|track)\b([^>]*)>").unwrap();
    let attrs = regex::Regex::new(r#"(?i)([\w-]+)\s*=\s*["']([^"']*)["']"#).unwrap();
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();
    for tag in tags.captures_iter(html).take(200) {
        let values = attrs
            .captures_iter(&tag[2])
            .map(|a| (a[1].to_lowercase(), a[2].to_string()))
            .collect::<std::collections::HashMap<_, _>>();
        let Some(src) = values.get("src").or_else(|| values.get("data-src")) else {
            continue;
        };
        let Ok(url) = base.join(src) else {
            continue;
        };
        if !matches!(url.scheme(), "http" | "https") || !seen.insert(url.to_string()) {
            continue;
        }
        result.push(MediaLink {
            kind: tag[1].to_lowercase(),
            url: url.to_string(),
            description: values.get("alt").cloned().unwrap_or_default(),
            inspected: false,
        });
    }
    result
}

pub async fn run(cache: Arc<SourceCache>, config: Arc<Config>) {
    let mut ticks = tokio::time::interval(std::time::Duration::from_secs(
        config.interval_min.unwrap_or(30).max(5) * 60,
    ));
    loop {
        ticks.tick().await;
        match cache.refresh(&config).await {
            Ok(report) => log::info!(
                "External source cache: {} feeds, {} entries, {} errors",
                report.feeds_ok,
                report.entries_cached,
                report.errors.len()
            ),
            Err(e) => log::warn!("External source cache: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn media_keeps_original_links_without_claiming_inspection() {
        let media = discover_media(
            r#"<img src="/plot.png" alt="Chart"><audio src="clip.mp3"></audio><img src="data:image/png;base64,x"><img src="/plot.png">"#,
            "https://example.org/post/",
        );
        assert_eq!(media.len(), 2);
        assert_eq!(media[0].url, "https://example.org/plot.png");
        assert_eq!(media[1].url, "https://example.org/post/clip.mp3");
        assert!(!media[0].inspected);
    }
    #[test]
    fn rediscovery_preserves_article_and_first_seen() {
        let db = sled::Config::new().temporary(true).open().unwrap();
        let cache = SourceCache {
            db,
            refresh_lock: tokio::sync::Mutex::new(()),
            article_lock: tokio::sync::Mutex::new(()),
        };
        let source = ContentSource::new("x", "X", "https://example.org/rss", ProductLine::Radio);
        let entry = FetchedEntry {
            media: vec![],
            title: "Article".into(),
            link: "https://example.org/a".into(),
            description: "teaser".into(),
            pub_date: None,
            source_name: None,
        };
        cache.cache_entry(source.clone(), entry.clone()).unwrap();
        let list = cache.list(None, 0, 0, 10).unwrap();
        let id = list["items"][0]["id"].as_str().unwrap();
        let mut r = cache.get(id).unwrap().unwrap();
        r.article_fetched_at = Some(100);
        r.fetched_at = 1;
        r.original_html = Some("complete".into());
        cache.put(&r).unwrap();
        cache.cache_entry(source, entry).unwrap();
        let r = cache.get(id).unwrap().unwrap();
        assert_eq!(r.original_html.as_deref(), Some("complete"));
        assert_eq!(r.fetched_at, 1);
        assert_eq!(cache.list(Some("reading"), 0, 0, 10).unwrap()["total"], 0);
    }
    #[tokio::test]
    async fn media_is_reused_after_origin_goes_offline() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/image.png", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 2048];
            let _ = socket.read(&mut request).await;
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nDATA")
                .await
                .unwrap();
        });
        let cache = SourceCache {
            db: sled::Config::new().temporary(true).open().unwrap(),
            refresh_lock: tokio::sync::Mutex::new(()),
            article_lock: tokio::sync::Mutex::new(()),
        };
        cache
            .put(&SourceRecord {
                id: "fixture".into(),
                source: ContentSource::new("f", "F", "https://example.org/rss", ProductLine::Radio),
                entry: FetchedEntry {
                    title: "T".into(),
                    link: "https://example.org/a".into(),
                    description: "".into(),
                    pub_date: None,
                    source_name: None,
                    media: vec![],
                },
                fetched_at: 1,
                article_fetched_at: None,
                original_html: None,
                reader_markdown: None,
                content_hash: None,
                fetch_error: None,
                media: vec![MediaLink {
                    url,
                    kind: "image".into(),
                    description: "".into(),
                    inspected: false,
                }],
            })
            .unwrap();
        assert_eq!(
            cache
                .fetch_media("fixture", 0, &FeedFetchOptions::default())
                .await
                .unwrap(),
            b"DATA"
        );
        server.await.unwrap();
        assert_eq!(
            cache
                .fetch_media("fixture", 0, &FeedFetchOptions::default())
                .await
                .unwrap(),
            b"DATA"
        );
        assert!(cache
            .fetch_media("fixture", 1, &FeedFetchOptions::default())
            .await
            .is_err());
    }
}
