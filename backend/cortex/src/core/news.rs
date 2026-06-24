use crate::core::aggregator::NewsAggregator;
use crate::core::config::{Config, CuratedFeedConfig};
use crate::core::content::{
    clean_text_for_processing, fetch_feed_entries, fetch_url_bytes, parse_opml_sources,
    FeedFetchOptions,
};
use crate::core::llm::LlmClient;
use crate::core::news_buffer::{NewsBuffer, PendingNewsItem};
use crate::core::nexus::NexusClient;
use crate::core::products::curated_feed::CuratedFeedPipeline;
use crate::core::products::loop_preferences::LoopPreferencePipeline;
use crate::core::topic_registry::TopicRegistry;
use crate::core::tts::TtsClient;
use anyhow::Result;
use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use sysinfo::{get_current_pid, System};
use tokio::time::{self, Duration};
use tokio_cron_scheduler::{Job, JobScheduler};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
struct ItemAnalysis {
    title: String,    // Cleaned/Translated title
    summary: String,  // 2-sentence summary
    category: String, // AI, Tech, Economy, Politics, Gaming, Other
    score: u8,        // 0-10, relevance/importance
}

async fn run_one_cycle(
    config: Arc<Config>,
    llm: Arc<LlmClient>,
    nexus: Arc<NexusClient>,
    aggregator: Arc<NewsAggregator>,
    buffer: Arc<tokio::sync::Mutex<NewsBuffer>>,
    now: chrono::DateTime<chrono::FixedOffset>,
) -> Result<()> {
    let current_time_str = now.format("%H:%M").to_string();
    let today_ymd = now.format("%Y-%m-%d").to_string();
    log::info!("Starting SMART news cycle at {}", current_time_str);

    // 1. Fetch ALL items from ALL sources (flat list)
    let mut all_candidate_items = Vec::new();
    let feed_count = config.rss_feeds.as_ref().map(|f| f.len()).unwrap_or(0);
    log::info!("Configured RSS feeds: {}", feed_count);
    if let Some(feeds) = &config.rss_feeds {
        let fetch_options = FeedFetchOptions::new(config.http_proxy.clone());
        let curated_source_urls = collect_curated_source_urls(&config).await;
        if !curated_source_urls.is_empty() {
            log::info!(
                "Radio RSS exclusion loaded {} curated source URLs",
                curated_source_urls.len()
            );
        }

        for url in feeds {
            if curated_source_urls.contains(&normalize_feed_url(url)) {
                log::info!(
                    "Skipping Radio RSS source already owned by curated feed: {}",
                    url
                );
                continue;
            }

            match fetch_feed_entries(url, &fetch_options).await {
                Ok(items) => all_candidate_items.extend(items),
                Err(e) => log::warn!("Failed to fetch RSS {}: {}", url, e),
            }
        }
    }

    if all_candidate_items.is_empty() {
        log::info!("No items found in any feed.");
        return Ok(());
    }

    // 2. Filter by Date (Today Only) & Dedup by Link
    let mut today_items = Vec::new();
    {
        let buf = buffer.lock().await;
        for item in all_candidate_items {
            if let Ok(true) = buf.has_processed_link(&item.link) {
                continue;
            }

            if item.title.contains("【推广】")
                || item.title.contains("[推广]")
                || item.title.contains("[广告]")
            {
                log::info!("Skipping Ad/Promotion Item: {}", item.title);
                buf.mark_link_processed(&item.link).ok();
                continue;
            }

            if let Some(pub_date_str) = &item.pub_date {
                if pub_date_str.starts_with(&today_ymd) {
                    today_items.push(item);
                } else if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(pub_date_str) {
                    let item_ymd = dt
                        .with_timezone(&chrono::Local)
                        .format("%Y-%m-%d")
                        .to_string();
                    if item_ymd == today_ymd {
                        today_items.push(item);
                    }
                }
            }
        }
    }

    log::info!(
        "Filtered {} items for today ({})",
        today_items.len(),
        today_ymd
    );
    if today_items.is_empty() {
        return Ok(());
    }

    // Deduplication against Nexus with 5 retries
    let urls: Vec<String> = today_items.iter().map(|i| i.link.clone()).collect();
    let mut existing_urls = Vec::new();

    for attempt in 0..5 {
        match nexus.check_urls(urls.clone()).await {
            Ok(u) => {
                existing_urls = u;
                let buf = buffer.lock().await;
                for url in &existing_urls {
                    buf.mark_link_processed(url).ok();
                }
                log::info!("Deduplication successful on attempt {}/5", attempt + 1);
                break;
            }
            Err(e) => {
                log::warn!("Check URLs failed (attempt {}/5): {}", attempt + 1, e);
                if attempt < 4 {
                    // Exponential backoff: 1s, 2s, 4s, 8s
                    let delay = std::time::Duration::from_secs(2_u64.pow(attempt as u32));
                    log::info!("Retrying deduplication in {:?}...", delay);
                    tokio::time::sleep(delay).await;
                } else {
                    log::error!("Failed to connect to Nexus for deduplication after 5 attempts");
                    return Err(anyhow::anyhow!(
                        "Failed to connect to Nexus for deduplication after 5 attempts: {}",
                        e
                    ));
                }
            }
        }
    }

    let new_items: Vec<_> = today_items
        .into_iter()
        .filter(|i| !existing_urls.contains(&i.link))
        .collect();

    let mut unique_links = HashSet::new();
    let mut unique_items = Vec::new();
    for item in new_items {
        if unique_links.insert(item.link.clone()) {
            unique_items.push(item);
        }
    }

    log::info!("Found {} NEW unique items to process", unique_items.len());
    if unique_items.is_empty() {
        return Ok(());
    }

    // 3. Classify and Buffer Steps
    let categories = config.categories.clone().unwrap_or_else(|| {
        vec![
            crate::core::config::CategoryDef {
                name: "Tech".to_string(),
                description: "Technology news".to_string(),
            },
            crate::core::config::CategoryDef {
                name: "Other".to_string(),
                description: "Other news".to_string(),
            },
        ]
    });
    let topics_str = categories
        .iter()
        .map(|c| format!("- {}: {}", c.name, c.description))
        .collect::<Vec<_>>()
        .join("\n");
    let valid_category_names: Vec<String> = categories.iter().map(|c| c.name.clone()).collect();
    let category_names_str = valid_category_names.join("、");

    for item in unique_items {
        let clean_desc = clean_text_for_processing(&item.description, 5000);
        let analysis_prompt = format!(
            "Analyze this news item.\nTitle: {}\nContent: {}\n\n\
            Task:\n\
            1. Classify into EXACTLY ONE of these categories:\n{}\n\
            **IMPORTANT**: The category field MUST be one of: [{}]\n\
               Copy the name exactly. Do NOT use synonyms or variations.\n\
            2. Summarize into 2 sentences (Chinese).\n\
            3. Translate title into Chinese.\n\
            Output JSON only: {{ \"category\": \"...\", \"summary\": \"...\", \"title\": \"...\", \"score\": 8 }}",
            item.title, clean_desc, topics_str, category_names_str
        );

        match llm
            .chat_json::<ItemAnalysis>(&analysis_prompt, "item_analysis", false)
            .await
        {
            Ok(analysis) => {
                if analysis.category == "广告"
                    || analysis.category == "Advertisement"
                    || analysis.score < 6
                {
                    log::info!(
                        "[FILTER] Discarding Low Quality/Ad: [{}] {} (Score: {})",
                        analysis.category,
                        analysis.title,
                        analysis.score
                    );
                    let buf = buffer.lock().await;
                    buf.mark_link_processed(&item.link).ok();
                    continue;
                }

                let normalized_category = {
                    let llm_cat = analysis.category.trim();
                    if valid_category_names.contains(&llm_cat.to_string()) {
                        llm_cat.to_string()
                    } else {
                        let mut matched = None;
                        let llm_cat_clean = llm_cat.replace(" ", "");
                        for valid_name in &valid_category_names {
                            let valid_name_clean = valid_name.replace(" ", "");
                            if valid_name_clean.contains(&llm_cat_clean)
                                || llm_cat_clean.contains(&valid_name_clean)
                            {
                                matched = Some(valid_name.clone());
                                break;
                            }
                        }
                        match matched {
                            Some(name) => {
                                log::info!("[CATEGORY FIX] Normalized '{}' -> '{}'", llm_cat, name);
                                name
                            }
                            None => {
                                log::warn!(
                                    "[CATEGORY FIX] Unknown category '{}', falling back to '其他'",
                                    llm_cat
                                );
                                "其他".to_string()
                            }
                        }
                    }
                };

                log::info!(
                    "Buffering item: [{}] {}",
                    normalized_category,
                    analysis.title
                );
                let pending = PendingNewsItem {
                    title: analysis.title,
                    link: item.link.clone(),
                    description: analysis.summary,
                    category: normalized_category,
                    source_name: item.source_name.clone(),
                    timestamp: chrono::Utc::now().timestamp() as u64,
                    original_text: clean_desc.clone(),
                };

                match aggregator.push_with_clustering(pending).await {
                    Ok(_) => {
                        let buf = buffer.lock().await;
                        buf.mark_link_processed(&item.link).ok();
                    }
                    Err(e) => log::error!("Failed to push with clustering: {}", e),
                }
            }
            Err(e) => {
                log::warn!("LLM analysis failed for '{}': {}", item.title, e);
                let buf = buffer.lock().await;
                buf.mark_link_processed(&item.link).ok();
            }
        }
    }

    // 4. Trigger Aggregation
    log::info!("Checking aggregator triggers...");
    if let Err(e) = aggregator.try_process().await {
        log::error!("Aggregator process failed: {}", e);
    }
    log::info!("News Cycle Finished.");
    Ok(())
}

async fn collect_curated_source_urls(config: &Config) -> HashSet<String> {
    let mut urls = HashSet::new();
    let Some(feed_config) = config.curated_feed.as_ref() else {
        return urls;
    };
    if !feed_config.enabled {
        return urls;
    }

    let Some(sources) = feed_config.feeds.as_ref() else {
        return urls;
    };

    let fetch_options = curated_feed_fetch_options(config, feed_config);
    for source in sources {
        let kind = source.kind.as_deref().unwrap_or_else(|| {
            if source.url.to_ascii_lowercase().ends_with(".opml") {
                "opml"
            } else {
                "rss"
            }
        });

        if kind.eq_ignore_ascii_case("opml") {
            match fetch_url_bytes(&source.url, &fetch_options).await {
                Ok(bytes) => match parse_opml_sources(&bytes, source.source_group.as_deref()) {
                    Ok(parsed) => {
                        urls.extend(
                            parsed
                                .into_iter()
                                .map(|source| normalize_feed_url(&source.url)),
                        );
                    }
                    Err(e) => log::warn!(
                        "Failed to parse curated OPML for Radio exclusions {}: {}",
                        source.url,
                        e
                    ),
                },
                Err(e) => log::warn!(
                    "Failed to fetch curated OPML for Radio exclusions {}: {}",
                    source.url,
                    e
                ),
            }
        } else {
            urls.insert(normalize_feed_url(&source.url));
        }
    }

    urls
}

fn curated_feed_fetch_options(
    config: &Config,
    feed_config: &CuratedFeedConfig,
) -> FeedFetchOptions {
    FeedFetchOptions::new(config.http_proxy.clone())
        .with_prefer_proxy(feed_config.prefer_proxy.unwrap_or(false))
}

fn normalize_feed_url(url: &str) -> String {
    let trimmed = url.trim();
    if let Ok(parsed) = reqwest::Url::parse(trimmed) {
        let scheme = parsed.scheme().to_ascii_lowercase();
        let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
        let port = parsed
            .port()
            .map(|port| format!(":{port}"))
            .unwrap_or_default();
        let path = parsed.path().trim_end_matches('/');
        let query = parsed
            .query()
            .map(|query| format!("?{query}"))
            .unwrap_or_default();
        return format!("{scheme}://{host}{port}{path}{query}");
    }

    trimmed.trim_end_matches('/').to_ascii_lowercase()
}

pub async fn run_news_loop(
    config: Config,
    llm: Arc<LlmClient>,
    tts: Arc<TtsClient>,
    nexus: Arc<NexusClient>,
    _retry: Arc<crate::core::retry::RetryManager>,
    cache_dir: String,
) {
    let config = Arc::new(config);
    log::info!(
        "Scheduler configured with timezone_offset: {:?}",
        config.timezone_offset
    );
    // Initialize v2.0 Components
    let buffer = Arc::new(tokio::sync::Mutex::new(
        NewsBuffer::new(&cache_dir).expect("Failed to init NewsBuffer"),
    ));
    let registry = Arc::new(TopicRegistry::new(&cache_dir).expect("Failed to init TopicRegistry"));
    let tts_gen_dir = PathBuf::from(&cache_dir).join("tts_temp");
    std::fs::create_dir_all(&tts_gen_dir).ok();

    let aggregator = Arc::new(NewsAggregator::new(
        buffer.clone(),
        registry.clone(),
        llm.clone(),
        tts.clone(),
        nexus.clone(),
        config.hosts.clone(),
    ));
    let curated_pipeline = Arc::new(CuratedFeedPipeline::new(
        config.clone(),
        llm.clone(),
        tts.clone(),
        nexus.clone(),
    ));
    let loop_preference_pipeline = Arc::new(LoopPreferencePipeline::new(
        config.clone(),
        llm.clone(),
        nexus.clone(),
    ));

    // Migration / Startup Maintenance
    let _ = aggregator
        .backfill_history()
        .await
        .map_err(|e| log::warn!("Backfill failed: {}", e));
    let _ = registry
        .prune()
        .map(|n| log::info!("Pruned {} old topics", n));

    // Background Prune Task (every 6 hours)
    let registry_clone = registry.clone();
    tokio::spawn(async move {
        let mut interval = time::interval(Duration::from_secs(6 * 3600)); // 6 hours
        interval.tick().await; // Skip first tick (already pruned above)
        loop {
            interval.tick().await;
            match registry_clone.prune() {
                Ok(n) => log::info!("Periodic Prune: Removed {} expired topics", n),
                Err(e) => log::warn!("Periodic Prune failed: {}", e),
            }
        }
    });

    // Background Link Prune Task (daily)
    let buffer_clone = buffer.clone();
    tokio::spawn(async move {
        let mut interval = time::interval(Duration::from_secs(24 * 3600)); // Daily
        interval.tick().await; // Skip first tick
        loop {
            interval.tick().await;
            let buf = buffer_clone.lock().await;
            match buf.prune_old_links(3 * 24 * 3600) {
                // 3 days TTL
                Ok(n) => {
                    if n > 0 {
                        log::info!("Link Prune: Removed {} old processed links", n);
                    }
                }
                Err(e) => log::warn!("Link Prune failed: {}", e),
            }
        }
    });

    // Background Cluster Prune Task (daily - removes orphaned clusters older than 7 days)
    let buffer_clone2 = buffer.clone();
    tokio::spawn(async move {
        let mut interval = time::interval(Duration::from_secs(24 * 3600)); // Daily
        interval.tick().await; // Skip first tick
        loop {
            interval.tick().await;
            let buf = buffer_clone2.lock().await;
            match buf.prune_old_clusters(7 * 24 * 3600) {
                // 7 days TTL
                Ok(n) => {
                    if n > 0 {
                        log::info!("Cluster Prune: Removed {} old clusters", n);
                    }
                }
                Err(e) => log::warn!("Cluster Prune failed: {}", e),
            }
        }
    });

    // Background Trace Log Cleanup (daily - removes trace files older than 7 days)
    tokio::spawn(async move {
        let mut interval = time::interval(Duration::from_secs(24 * 3600)); // Daily
        interval.tick().await; // Skip first tick
        loop {
            interval.tick().await;
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            let trace_dir = std::path::PathBuf::from(&home).join(".freshloop/logs/traces");
            if let Ok(entries) = std::fs::read_dir(&trace_dir) {
                let cutoff = std::time::SystemTime::now()
                    .checked_sub(std::time::Duration::from_secs(7 * 24 * 3600))
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                let mut removed = 0;
                for entry in entries.flatten() {
                    if let Ok(meta) = entry.metadata() {
                        if let Ok(modified) = meta.modified() {
                            if modified < cutoff {
                                let _ = std::fs::remove_file(entry.path());
                                removed += 1;
                            }
                        }
                    }
                }
                if removed > 0 {
                    log::info!("Trace Prune: Removed {} old trace files", removed);
                }
            }
        }
    });

    let sched = JobScheduler::new()
        .await
        .expect("Failed to create scheduler");
    let timezone_config = config.clone();
    let get_now = move || {
        let offset = timezone_config.timezone_offset.unwrap_or(8);
        let tz = chrono::FixedOffset::east_opt(offset * 3600)
            .unwrap_or(chrono::FixedOffset::east_opt(8 * 3600).unwrap());
        chrono::Utc::now().with_timezone(&tz)
    };

    // Scheduled runs
    if let Some(times) = &config.schedule_times {
        for time_str in times {
            let parts: Vec<&str> = time_str.split(':').collect();
            if parts.len() == 2 {
                let h: i32 = parts[0].parse().unwrap_or(0);
                let m: i32 = parts[1].parse().unwrap_or(0);

                // Adjust for offset to get UTC cron string
                let offset = config.timezone_offset.unwrap_or(8);
                let mut utc_h = h - offset;
                while utc_h < 0 {
                    utc_h += 24;
                }
                while utc_h >= 24 {
                    utc_h -= 24;
                }

                let cron_str = format!("0 {} {} * * *", m, utc_h);
                log::info!(
                    "Adding scheduled job: {} (Local) -> {} (UTC Cron)",
                    time_str,
                    cron_str
                );

                let c = config.clone();
                let l = llm.clone();
                let n = nexus.clone();
                let a = aggregator.clone();
                let b = buffer.clone();
                let gn = get_now.clone();

                let job = Job::new_async(cron_str.as_str(), move |_uuid, _l| {
                    let c = c.clone();
                    let l = l.clone();
                    let n = n.clone();
                    let a = a.clone();
                    let b = b.clone();
                    let now = gn();
                    Box::pin(async move {
                        if let Err(e) = run_one_cycle(c, l, n, a, b, now).await {
                            log::error!("Scheduled news cycle failed: {}", e);
                        }
                    })
                })
                .expect("Failed to create job");
                sched.add(job).await.expect("Failed to add job");
            }
        }
    } else {
        // Fallback: Default hourly if no schedule (using cron)
        log::info!("No schedule_times configured, adding hourly default job");
        let c = config.clone();
        let l = llm.clone();
        let n = nexus.clone();
        let a = aggregator.clone();
        let b = buffer.clone();
        let gn = get_now.clone();
        let job = Job::new_async("0 0 * * * *", move |_uuid, _l| {
            let c = c.clone();
            let l = l.clone();
            let n = n.clone();
            let a = a.clone();
            let b = b.clone();
            let now = gn();
            Box::pin(async move {
                if let Err(e) = run_one_cycle(c, l, n, a, b, now).await {
                    log::error!("Hourly news cycle failed: {}", e);
                }
            })
        })
        .expect("Failed to create hourly job");
        sched.add(job).await.expect("Failed to add hourly job");
    }

    if curated_pipeline.is_enabled() {
        let curated_times = config
            .curated_feed
            .as_ref()
            .and_then(|feed| feed.schedule_times.clone())
            .unwrap_or_else(|| vec!["08:30".to_string()]);

        for time_str in curated_times {
            if let Some(cron_str) = local_time_to_utc_cron(&time_str, config.timezone_offset) {
                log::info!(
                    "Adding curated feed job: {} (Local) -> {} (UTC Cron)",
                    time_str,
                    cron_str
                );
                let pipeline = curated_pipeline.clone();
                let gn = get_now.clone();
                let job = Job::new_async(cron_str.as_str(), move |_uuid, _l| {
                    let pipeline = pipeline.clone();
                    let now = gn();
                    Box::pin(async move {
                        if let Err(e) = pipeline.run_once(now).await {
                            log::error!("Scheduled curated feed cycle failed: {}", e);
                        }
                    })
                })
                .expect("Failed to create curated feed job");
                sched
                    .add(job)
                    .await
                    .expect("Failed to add curated feed job");
            } else {
                log::warn!("Invalid curated feed schedule time '{}'", time_str);
            }
        }

        let weekly_digest_enabled = config
            .curated_feed
            .as_ref()
            .and_then(|feed| feed.weekly_digest_enabled)
            .unwrap_or(true);
        if weekly_digest_enabled {
            let weekly_times = config
                .curated_feed
                .as_ref()
                .and_then(|feed| feed.weekly_digest_schedule_times.clone())
                .unwrap_or_else(|| vec!["21:00".to_string()]);

            for time_str in weekly_times {
                if let Some(cron_str) = local_time_to_utc_cron(&time_str, config.timezone_offset) {
                    log::info!(
                        "Adding curated weekly digest check: {} (Local) -> {} (UTC Cron)",
                        time_str,
                        cron_str
                    );
                    let pipeline = curated_pipeline.clone();
                    let gn = get_now.clone();
                    let job = Job::new_async(cron_str.as_str(), move |_uuid, _l| {
                        let pipeline = pipeline.clone();
                        let now = gn();
                        Box::pin(async move {
                            match pipeline.run_weekly_digest(now, false).await {
                                Ok(stats) if stats.published => log::info!(
                                    "Scheduled curated weekly digest published: included={}",
                                    stats.included_items
                                ),
                                Ok(stats) => log::info!(
                                    "Scheduled curated weekly digest skipped: {:?}",
                                    stats.skipped_reason
                                ),
                                Err(e) => {
                                    log::error!("Scheduled curated weekly digest failed: {}", e)
                                }
                            }
                        })
                    })
                    .expect("Failed to create curated weekly digest job");
                    sched
                        .add(job)
                        .await
                        .expect("Failed to add curated weekly digest job");
                } else {
                    log::warn!("Invalid curated weekly digest schedule time '{}'", time_str);
                }
            }
        }
    }

    if loop_preference_pipeline.is_enabled() {
        let loop_preference_times = config
            .loop_preferences
            .as_ref()
            .and_then(|prefs| prefs.schedule_times.clone())
            .unwrap_or_else(|| vec!["09:00".to_string(), "21:00".to_string()]);

        for time_str in loop_preference_times {
            if let Some(cron_str) = local_time_to_utc_cron(&time_str, config.timezone_offset) {
                log::info!(
                    "Adding Loop preference extraction job: {} (Local) -> {} (UTC Cron)",
                    time_str,
                    cron_str
                );
                let pipeline = loop_preference_pipeline.clone();
                let job = Job::new_async(cron_str.as_str(), move |_uuid, _l| {
                    let pipeline = pipeline.clone();
                    Box::pin(async move {
                        match pipeline.run_once().await {
                            Ok(stats) => log::info!(
                                "Scheduled Loop preference extraction completed: considered={}, processed={}, skipped={}, failed={}, signals={}",
                                stats.considered_posts,
                                stats.processed_posts,
                                stats.skipped_posts,
                                stats.failed_posts,
                                stats.written_signals
                            ),
                            Err(e) => log::error!(
                                "Scheduled Loop preference extraction failed: {}",
                                e
                            ),
                        }
                    })
                })
                .expect("Failed to create Loop preference extraction job");
                sched
                    .add(job)
                    .await
                    .expect("Failed to add Loop preference extraction job");
            } else {
                log::warn!("Invalid Loop preference schedule time '{}'", time_str);
            }
        }
    }

    // 3. Maintenance job (every minute)
    let m_aggregator = aggregator.clone();
    let maintenance_job = Job::new_async("0 * * * * *", move |_uuid, _l| {
        let a = m_aggregator.clone();
        Box::pin(async move {
            if let Err(e) = a.process_regenerations().await {
                log::error!("Regeneration cycle failed: {}", e);
            }
        })
    })
    .expect("Failed to create maintenance job");
    sched
        .add(maintenance_job)
        .await
        .expect("Failed to add maintenance job");

    sched.start().await.expect("Failed to start scheduler");
    log::info!("Job scheduler started successfully.");

    // Start HTTP trigger server (replaces the idle sleep loop)
    let bind_addr = std::env::var("CORTEX_BIND_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:3721".to_string())
        .parse::<SocketAddr>()
        .expect("CORTEX_BIND_ADDR must be a valid socket address, e.g. 127.0.0.1:3721");
    let api_key = std::env::var("CORTEX_API_KEY")
        .ok()
        .filter(|key| !key.trim().is_empty());

    if !bind_addr.ip().is_loopback() && api_key.is_none() {
        panic!(
            "Refusing to bind Cortex trigger API to non-loopback address {} without CORTEX_API_KEY",
            bind_addr
        );
    }

    let app_state = Arc::new(TriggerState {
        config: config.clone(),
        llm: llm.clone(),
        nexus: nexus.clone(),
        aggregator: aggregator.clone(),
        buffer: buffer.clone(),
        registry: registry.clone(),
        curated_pipeline: curated_pipeline.clone(),
        loop_preference_pipeline: loop_preference_pipeline.clone(),
        get_now: Box::new(get_now),
        running: tokio::sync::Mutex::new(false),
        api_key,
    });

    let app = Router::new()
        .route("/api/trigger", post(handle_trigger))
        .route("/api/trigger/feed", post(handle_feed_trigger))
        .route("/api/trigger/feed/weekly", post(handle_feed_weekly_trigger))
        .route(
            "/api/trigger/loop/preferences",
            post(handle_loop_preferences_trigger),
        )
        .route("/api/status", get(handle_status))
        .route("/api/memory", get(handle_memory))
        .route("/api/health/nexus", get(handle_nexus_health))
        .with_state(app_state);

    log::info!("Cortex trigger API listening on {}", bind_addr);
    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .expect("Failed to bind trigger API");
    axum::serve(listener, app)
        .await
        .expect("Trigger API server failed");
}

// --- HTTP Trigger API ---

struct TriggerState {
    config: Arc<Config>,
    llm: Arc<LlmClient>,
    nexus: Arc<NexusClient>,
    aggregator: Arc<NewsAggregator>,
    buffer: Arc<tokio::sync::Mutex<NewsBuffer>>,
    registry: Arc<TopicRegistry>,
    curated_pipeline: Arc<CuratedFeedPipeline>,
    loop_preference_pipeline: Arc<LoopPreferencePipeline>,
    get_now: Box<dyn Fn() -> chrono::DateTime<chrono::FixedOffset> + Send + Sync>,
    running: tokio::sync::Mutex<bool>,
    api_key: Option<String>,
}

#[derive(serde::Serialize)]
struct TriggerResponse {
    success: bool,
    message: String,
}

#[derive(serde::Deserialize, Default)]
struct TriggerQuery {
    /// When true, skip RSS fetching and LLM clustering — go straight to
    /// aggregator.try_process() which generates scripts + TTS from
    /// already-buffered clusters.
    #[serde(default)]
    flush_only: bool,
}

#[derive(serde::Deserialize, Default)]
struct FeedWeeklyTriggerQuery {
    #[serde(default)]
    force: bool,
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }

    left.as_bytes()
        .iter()
        .zip(right.as_bytes())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

fn has_valid_cortex_key(headers: &HeaderMap, expected_key: &Option<String>) -> bool {
    let Some(expected_key) = expected_key.as_deref() else {
        return true;
    };

    let direct_key = headers
        .get("X-CORTEX-KEY")
        .and_then(|value| value.to_str().ok());
    if direct_key.is_some_and(|key| constant_time_eq(key, expected_key)) {
        return true;
    }

    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|key| constant_time_eq(key, expected_key))
}

fn unauthorized_response() -> axum::response::Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({
            "success": false,
            "message": "Invalid or missing Cortex API key"
        })),
    )
        .into_response()
}

fn local_time_to_utc_cron(time_str: &str, timezone_offset: Option<i32>) -> Option<String> {
    let parts: Vec<&str> = time_str.split(':').collect();
    if parts.len() != 2 {
        return None;
    }
    let h: i32 = parts[0].parse().ok()?;
    let m: i32 = parts[1].parse().ok()?;
    if !(0..=23).contains(&h) || !(0..=59).contains(&m) {
        return None;
    }

    let offset = timezone_offset.unwrap_or(8);
    let mut utc_h = h - offset;
    while utc_h < 0 {
        utc_h += 24;
    }
    while utc_h >= 24 {
        utc_h -= 24;
    }
    Some(format!("0 {} {} * * *", m, utc_h))
}

async fn handle_trigger(
    State(state): State<Arc<TriggerState>>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<TriggerQuery>,
) -> impl IntoResponse {
    if !has_valid_cortex_key(&headers, &state.api_key) {
        return unauthorized_response();
    }

    // Prevent concurrent triggers
    {
        let mut running = state.running.lock().await;
        if *running {
            return Json(TriggerResponse {
                success: false,
                message: "A cycle is already running. Please wait.".to_string(),
            })
            .into_response();
        }
        *running = true;
    }

    let now = (state.get_now)();
    let flush_only = query.flush_only;

    if flush_only {
        log::info!(
            "[Trigger API] Flush-only triggered at {} (skipping RSS+LLM, running TTS directly)",
            now
        );
    } else {
        log::info!("[Trigger API] Manual cycle triggered at {}", now);
    }

    let config = state.config.clone();
    let llm = state.llm.clone();
    let nexus = state.nexus.clone();
    let aggregator = state.aggregator.clone();
    let buffer = state.buffer.clone();
    let state_clone = state.clone();

    // Run in background so the HTTP response returns immediately
    tokio::spawn(async move {
        let result = if flush_only {
            // Skip RSS + LLM, directly process buffered clusters
            log::info!(
                "[Flush-Only] Running aggregator.try_process() on existing buffered data..."
            );
            aggregator.try_process().await
        } else {
            run_one_cycle(config, llm, nexus, aggregator, buffer, now).await
        };
        match &result {
            Ok(()) => log::info!(
                "[Trigger API] {} completed successfully",
                if flush_only {
                    "Flush-only"
                } else {
                    "Manual cycle"
                }
            ),
            Err(e) => log::error!(
                "[Trigger API] {} failed: {}",
                if flush_only {
                    "Flush-only"
                } else {
                    "Manual cycle"
                },
                e
            ),
        }
        *state_clone.running.lock().await = false;
    });

    let mode = if flush_only {
        "Flush-only (TTS only)"
    } else {
        "Full cycle"
    };
    Json(TriggerResponse {
        success: true,
        message: format!("{} triggered at {}. Running in background.", mode, now),
    })
    .into_response()
}

async fn handle_feed_trigger(
    State(state): State<Arc<TriggerState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if !has_valid_cortex_key(&headers, &state.api_key) {
        return unauthorized_response();
    }

    {
        let mut running = state.running.lock().await;
        if *running {
            return Json(TriggerResponse {
                success: false,
                message: "A cycle is already running. Please wait.".to_string(),
            })
            .into_response();
        }
        *running = true;
    }

    let now = (state.get_now)();
    let pipeline = state.curated_pipeline.clone();
    let state_clone = state.clone();

    tokio::spawn(async move {
        let result = pipeline.run_once(now).await;
        match &result {
            Ok(stats) => log::info!(
                "[Trigger API] curated feed completed: sources={}, entries={}, published={}, audio={}, skipped={}",
                stats.resolved_sources,
                stats.fetched_entries,
                stats.published_items,
                stats.published_audio_items,
                stats.skipped_items
            ),
            Err(e) => log::error!("[Trigger API] curated feed failed: {}", e),
        }
        *state_clone.running.lock().await = false;
    });

    Json(TriggerResponse {
        success: true,
        message: format!(
            "Curated feed cycle triggered at {}. Running in background.",
            now
        ),
    })
    .into_response()
}

async fn handle_feed_weekly_trigger(
    State(state): State<Arc<TriggerState>>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<FeedWeeklyTriggerQuery>,
) -> impl IntoResponse {
    if !has_valid_cortex_key(&headers, &state.api_key) {
        return unauthorized_response();
    }

    {
        let mut running = state.running.lock().await;
        if *running {
            return Json(TriggerResponse {
                success: false,
                message: "A cycle is already running. Please wait.".to_string(),
            })
            .into_response();
        }
        *running = true;
    }

    let now = (state.get_now)();
    let force = query.force;
    let pipeline = state.curated_pipeline.clone();
    let state_clone = state.clone();

    tokio::spawn(async move {
        let result = pipeline.run_weekly_digest(now, force).await;
        match &result {
            Ok(stats) if stats.published => log::info!(
                "[Trigger API] curated weekly digest published: considered={}, included={}",
                stats.considered_items,
                stats.included_items
            ),
            Ok(stats) => log::info!(
                "[Trigger API] curated weekly digest skipped: {:?}",
                stats.skipped_reason
            ),
            Err(e) => log::error!("[Trigger API] curated weekly digest failed: {}", e),
        }
        *state_clone.running.lock().await = false;
    });

    Json(TriggerResponse {
        success: true,
        message: format!(
            "Curated weekly digest triggered at {} (force={}). Running in background.",
            now, force
        ),
    })
    .into_response()
}

async fn handle_loop_preferences_trigger(
    State(state): State<Arc<TriggerState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if !has_valid_cortex_key(&headers, &state.api_key) {
        return unauthorized_response();
    }

    {
        let mut running = state.running.lock().await;
        if *running {
            return Json(TriggerResponse {
                success: false,
                message: "A cycle is already running. Please wait.".to_string(),
            })
            .into_response();
        }
        *running = true;
    }

    let pipeline = state.loop_preference_pipeline.clone();
    let state_clone = state.clone();

    tokio::spawn(async move {
        let result = pipeline.run_once().await;
        match &result {
            Ok(stats) => log::info!(
                "[Trigger API] Loop preference extraction completed: considered={}, processed={}, skipped={}, failed={}, signals={}",
                stats.considered_posts,
                stats.processed_posts,
                stats.skipped_posts,
                stats.failed_posts,
                stats.written_signals
            ),
            Err(e) => log::error!("[Trigger API] Loop preference extraction failed: {}", e),
        }
        *state_clone.running.lock().await = false;
    });

    Json(TriggerResponse {
        success: true,
        message: "Loop preference extraction triggered. Running in background.".to_string(),
    })
    .into_response()
}

async fn handle_status(
    State(state): State<Arc<TriggerState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if !has_valid_cortex_key(&headers, &state.api_key) {
        return unauthorized_response();
    }

    let running = *state.running.lock().await;
    let category_stats = {
        let buf = state.buffer.lock().await;
        buf.get_category_stats().unwrap_or_default()
    };
    let total_clusters: usize = category_stats.values().map(|(c, _)| c).sum();
    let now = (state.get_now)();

    Json(serde_json::json!({
        "status": if running { "running" } else { "idle" },
        "current_time": now.to_string(),
        "pending_clusters": total_clusters,
        "curated_feed_enabled": state.curated_pipeline.is_enabled(),
        "curated_weekly_digest_enabled": state.config.curated_feed.as_ref().and_then(|feed| feed.weekly_digest_enabled).unwrap_or(true),
        "loop_preferences_enabled": state.loop_preference_pipeline.is_enabled(),
        "categories": category_stats.iter().map(|(k, (count, oldest))| {
            serde_json::json!({ "name": k, "clusters": count, "oldest_ts": oldest })
        }).collect::<Vec<_>>(),
    }))
    .into_response()
}

// --- Memory Diagnostics API ---

#[derive(serde::Serialize)]
struct MemorySnapshot {
    timestamp: String,
    process: ProcessMemoryInfo,
    system: SystemMemoryInfo,
    components: ComponentMemoryInfo,
}

#[derive(serde::Serialize)]
struct ProcessMemoryInfo {
    pid: u32,
    name: String,
    memory_mb: f64,
    virtual_memory_mb: f64,
    cpu_percent: f64,
}

#[derive(serde::Serialize)]
struct SystemMemoryInfo {
    total_mb: f64,
    used_mb: f64,
    free_mb: f64,
    used_percent: f64,
}

#[derive(serde::Serialize)]
struct ComponentMemoryInfo {
    // NewsBuffer stats
    news_buffer_clusters: usize,
    news_buffer_categories: usize,
    news_buffer_db_size_mb: f64,

    // TopicRegistry stats
    topic_registry_topics: usize,
    topic_registry_estimate_mb: f64,

    // LLM Cache stats
    llm_cache_entries: usize,
    llm_cache_estimate_mb: f64,

    // Retry queue stats
    retry_queue_size: usize,
    retry_queue_estimate_mb: f64,

    // TTS stats (if active)
    tts_active_chunks: usize,
    tts_audio_buffer_mb: f64,

    // Nexus connection health
    nexus_healthy: bool,
    nexus_latency_ms: u64,
}

async fn handle_memory(
    State(state): State<Arc<TriggerState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if !has_valid_cortex_key(&headers, &state.api_key) {
        return unauthorized_response();
    }

    let mut system = System::new_all();
    system.refresh_all();

    let pid = get_current_pid().unwrap_or_else(|_| sysinfo::Pid::from_u32(0));
    let process = system
        .process(pid)
        .map(|p| ProcessMemoryInfo {
            pid: pid.as_u32(),
            name: p.name().to_str().unwrap_or("unknown").to_string(),
            memory_mb: p.memory() as f64 / 1024.0 / 1024.0,
            virtual_memory_mb: p.virtual_memory() as f64 / 1024.0 / 1024.0,
            cpu_percent: p.cpu_usage() as f64,
        })
        .unwrap_or_else(|| ProcessMemoryInfo {
            pid: pid.as_u32(),
            name: "unknown".to_string(),
            memory_mb: 0.0,
            virtual_memory_mb: 0.0,
            cpu_percent: 0.0,
        });

    let total_memory = system.total_memory();
    let used_memory = system.used_memory();

    let system_info = SystemMemoryInfo {
        total_mb: total_memory as f64 / 1024.0 / 1024.0,
        used_mb: used_memory as f64 / 1024.0 / 1024.0,
        free_mb: (total_memory - used_memory) as f64 / 1024.0 / 1024.0,
        used_percent: (used_memory as f64 / total_memory as f64) * 100.0,
    };

    // Get component stats
    let (cluster_count, category_count, news_buffer_size) = {
        let buf = state.buffer.lock().await;
        let stats = buf.get_category_stats().unwrap_or_default();
        let clusters: usize = stats.values().map(|(c, _)| c).sum();
        let size_bytes = buf.get_db_size().unwrap_or(0);
        (clusters, stats.len(), size_bytes)
    };

    // Get topic registry stats
    let (topic_count, topic_size) = state.registry.get_stats().unwrap_or((0, 0));

    // Get LLM cache stats
    let (llm_cache_count, llm_cache_size) = state.llm.get_cache_stats().unwrap_or((0, 0));

    // Get Nexus health status
    let nexus_health = state.nexus.get_health().await;

    let components = ComponentMemoryInfo {
        news_buffer_clusters: cluster_count,
        news_buffer_categories: category_count,
        news_buffer_db_size_mb: news_buffer_size as f64 / 1024.0 / 1024.0,

        topic_registry_topics: topic_count,
        topic_registry_estimate_mb: topic_size as f64 / 1024.0 / 1024.0,

        llm_cache_entries: llm_cache_count,
        llm_cache_estimate_mb: llm_cache_size as f64 / 1024.0 / 1024.0,

        // TODO: Add retry queue stats when RetryManager is accessible from state
        retry_queue_size: 0,
        retry_queue_estimate_mb: 0.0,

        // TODO: Add TTS stats when TTS is processing
        tts_active_chunks: 0,
        tts_audio_buffer_mb: 0.0,

        // Nexus connection health
        nexus_healthy: nexus_health.is_healthy,
        nexus_latency_ms: nexus_health.latency_ms,
    };

    Json(MemorySnapshot {
        timestamp: chrono::Local::now().to_rfc3339(),
        process,
        system: system_info,
        components,
    })
    .into_response()
}

/// Trigger a fresh health check to Nexus (bypasses DNS cache)
async fn handle_nexus_health(
    State(state): State<Arc<TriggerState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if !has_valid_cortex_key(&headers, &state.api_key) {
        return unauthorized_response();
    }

    match state.nexus.health_check().await {
        Ok(health) => Json(serde_json::json!({
            "success": true,
            "healthy": health.is_healthy,
            "latency_ms": health.latency_ms,
            "last_check": health.last_check,
            "error_count": health.error_count,
            "last_error": health.last_error,
            "message": if health.is_healthy {
                "Nexus is healthy"
            } else {
                "Nexus health check failed"
            }
        }))
        .into_response(),
        Err(e) => Json(serde_json::json!({
            "success": false,
            "healthy": false,
            "error": e.to_string(),
            "message": "Failed to check Nexus health"
        }))
        .into_response(),
    }
}
