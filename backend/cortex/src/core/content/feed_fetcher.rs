use super::model::FetchedEntry;
use anyhow::{anyhow, Result};
use std::collections::HashSet;
use std::fs;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
const PROXY_DOMAIN_CACHE: &str = ".freshloop/cache/feed_proxy_domains.json";
static PROXY_DOMAINS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

#[derive(Debug, Clone, Default)]
pub struct FeedFetchOptions {
    pub proxy_url: Option<String>,
    pub prefer_proxy: bool,
}

impl FeedFetchOptions {
    pub fn new(proxy_url: Option<String>) -> Self {
        Self {
            proxy_url,
            prefer_proxy: false,
        }
    }

    pub fn with_prefer_proxy(mut self, prefer_proxy: bool) -> Self {
        self.prefer_proxy = prefer_proxy;
        self
    }
}

pub async fn fetch_feed_entries(
    url: &str,
    options: &FeedFetchOptions,
) -> Result<Vec<FetchedEntry>> {
    let content = fetch_url_bytes(url, options).await?;
    parse_feed_entries(&content)
}

pub async fn fetch_url_bytes(url: &str, options: &FeedFetchOptions) -> Result<bytes::Bytes> {
    let cached_proxy_domain = is_cached_proxy_domain(url);
    if options.prefer_proxy || cached_proxy_domain {
        if let Some(proxy_url) = options.proxy_url.as_deref() {
            match fetch_bytes(url, Some(proxy_url)).await {
                Ok(bytes) => {
                    remember_proxy_domain(url);
                    return Ok(bytes);
                }
                Err(proxy_err) => {
                    log::info!(
                        "Proxy feed fetch failed for {}, retrying direct: {}",
                        url,
                        proxy_err
                    );
                    match fetch_bytes(url, None).await {
                        Ok(bytes) => {
                            if cached_proxy_domain {
                                forget_proxy_domain(url);
                            }
                            return Ok(bytes);
                        }
                        Err(direct_err) => {
                            return Err(anyhow!(
                                "proxy fetch failed: {}; direct fallback failed: {}",
                                proxy_err,
                                direct_err
                            ));
                        }
                    }
                }
            }
        }
    }

    match fetch_bytes(url, None).await {
        Ok(bytes) => Ok(bytes),
        Err(direct_err) => {
            let Some(proxy_url) = options.proxy_url.as_deref() else {
                return Err(direct_err);
            };
            log::info!(
                "Direct feed fetch failed for {}, retrying with proxy: {}",
                url,
                proxy_url
            );
            match fetch_bytes(url, Some(proxy_url)).await {
                Ok(bytes) => {
                    remember_proxy_domain(url);
                    Ok(bytes)
                }
                Err(proxy_err) => Err(anyhow!(
                    "direct fetch failed: {}; proxy fallback failed: {}",
                    direct_err,
                    proxy_err
                )),
            }
        }
    }
}

async fn fetch_bytes(url: &str, proxy_url: Option<&str>) -> Result<bytes::Bytes> {
    let client = build_client(proxy_url)?;
    Ok(client.get(url).send().await?.bytes().await?)
}

fn build_client(proxy_url: Option<&str>) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .user_agent(DEFAULT_USER_AGENT)
        .timeout(std::time::Duration::from_secs(30))
        .connect_timeout(std::time::Duration::from_secs(10));

    if let Some(proxy_url) = proxy_url {
        builder = builder.proxy(reqwest::Proxy::all(proxy_url)?);
    }

    Ok(builder.build()?)
}

fn is_cached_proxy_domain(url: &str) -> bool {
    proxy_cache_key(url).is_some_and(|domain| {
        proxy_domains()
            .lock()
            .is_ok_and(|set| set.contains(&domain))
    })
}

fn remember_proxy_domain(url: &str) {
    let Some(domain) = proxy_cache_key(url) else {
        return;
    };

    let Ok(mut domains) = proxy_domains().lock() else {
        return;
    };
    if domains.insert(domain.clone()) {
        persist_proxy_domains(&domains);
        log::info!("Marked feed domain for proxy-first fetch: {}", domain);
    }
}

fn forget_proxy_domain(url: &str) {
    let Some(domain) = proxy_cache_key(url) else {
        return;
    };

    let Ok(mut domains) = proxy_domains().lock() else {
        return;
    };
    if domains.remove(&domain) {
        persist_proxy_domains(&domains);
        log::info!("Removed feed domain from proxy-first cache: {}", domain);
    }
}

fn proxy_domains() -> &'static Mutex<HashSet<String>> {
    PROXY_DOMAINS.get_or_init(|| Mutex::new(load_proxy_domains()))
}

fn load_proxy_domains() -> HashSet<String> {
    let path = proxy_domain_cache_path();
    let Ok(bytes) = fs::read(path) else {
        return HashSet::new();
    };
    serde_json::from_slice::<Vec<String>>(&bytes)
        .unwrap_or_default()
        .into_iter()
        .map(|domain| domain.trim().to_ascii_lowercase())
        .filter(|domain| !domain.is_empty())
        .collect()
}

fn persist_proxy_domains(domains: &HashSet<String>) {
    let path = proxy_domain_cache_path();
    if let Some(parent) = path.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            log::warn!(
                "Failed to create proxy domain cache dir {:?}: {}",
                parent,
                e
            );
            return;
        }
    }

    let mut sorted = domains.iter().cloned().collect::<Vec<_>>();
    sorted.sort();
    match serde_json::to_vec_pretty(&sorted) {
        Ok(bytes) => {
            if let Err(e) = fs::write(&path, bytes) {
                log::warn!("Failed to persist proxy domain cache {:?}: {}", path, e);
            }
        }
        Err(e) => log::warn!("Failed to serialize proxy domain cache: {}", e),
    }
}

fn proxy_domain_cache_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(PROXY_DOMAIN_CACHE)
}

fn proxy_cache_key(url: &str) -> Option<String> {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(|host| host.to_ascii_lowercase()))
        .filter(|host| !host.trim().is_empty())
}

pub(crate) fn parse_feed_entries(content: &[u8]) -> Result<Vec<FetchedEntry>> {
    let cursor = Cursor::new(content);
    let feed = feed_rs::parser::parse(cursor)?;
    let source_title = feed.title.map(|t| t.content).unwrap_or_default();

    let items = feed
        .entries
        .into_iter()
        .map(|entry| {
            let title = entry.title.map(|t| t.content).unwrap_or_default();
            let link = entry
                .links
                .first()
                .map(|l| l.href.clone())
                .unwrap_or_default();

            let summary = entry.summary.map(|s| s.content).unwrap_or_default();
            let content = entry.content.and_then(|c| c.body).unwrap_or_default();
            let description = if content.chars().count() > summary.chars().count() {
                content
            } else {
                summary
            };

            let pub_date = entry.published.or(entry.updated).map(|d| d.to_rfc3339());

            FetchedEntry {
                title,
                link,
                description,
                pub_date,
                source_name: if source_title.is_empty() {
                    None
                } else {
                    Some(source_title.clone())
                },
            }
        })
        .filter(|i| !i.link.is_empty())
        .collect();

    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rss_entries_without_network() {
        let rss = br#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
    <title>Example Feed</title>
    <item>
      <title>First</title>
      <link>https://example.com/first</link>
      <description>Hello world</description>
      <pubDate>Sun, 17 May 2026 10:00:00 GMT</pubDate>
    </item>
  </channel>
</rss>"#;

        let entries = parse_feed_entries(rss).expect("parse feed");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].title, "First");
        assert_eq!(entries[0].link, "https://example.com/first");
        assert_eq!(entries[0].description, "Hello world");
        assert_eq!(entries[0].source_name.as_deref(), Some("Example Feed"));
        assert!(entries[0].pub_date.is_some());
    }

    #[test]
    fn prefers_atom_content_over_short_summary_and_uses_updated_time() {
        let atom = br#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>Example Atom</title>
  <entry>
    <title>Long Post</title>
    <link href="https://example.com/long"/>
    <updated>2026-05-17T10:00:00Z</updated>
    <summary>Short teaser.</summary>
    <content type="html">&lt;p&gt;This is the much longer article body with enough detail to be treated as the reader content instead of the summary teaser.&lt;/p&gt;</content>
  </entry>
</feed>"#;

        let entries = parse_feed_entries(atom).expect("parse atom feed");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].title, "Long Post");
        assert_eq!(entries[0].link, "https://example.com/long");
        assert!(entries[0].description.contains("much longer article body"));
        assert_eq!(
            entries[0].pub_date.as_deref(),
            Some("2026-05-17T10:00:00+00:00")
        );
    }

    #[test]
    fn proxy_cache_key_uses_lowercase_host() {
        assert_eq!(
            proxy_cache_key("https://Example.COM/feed.xml").as_deref(),
            Some("example.com")
        );
        assert_eq!(proxy_cache_key("not a url"), None);
    }
}
