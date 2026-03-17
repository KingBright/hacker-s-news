use crate::core::aggregator::NewsAggregator;
use crate::core::config::Config;
use crate::core::llm::LlmClient;
use crate::core::news_buffer::{NewsBuffer, PendingNewsItem};
use crate::core::nexus::NexusClient;
use crate::core::topic_registry::TopicRegistry;
use crate::core::tts::TtsClient;
use anyhow::Result;
use axum::{extract::State, routing::{get, post}, Json, Router};
use regex::Regex;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::time::{self, Duration};
use tokio_cron_scheduler::{Job, JobScheduler};
use sysinfo::{System, get_current_pid};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
        let proxy = config.http_proxy.as_deref();
        for url in feeds {
            match fetch_rss_items(url, proxy).await {
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
                    return Err(anyhow::anyhow!("Failed to connect to Nexus for deduplication after 5 attempts: {}", e));
                }
            }
        }
    }

    let new_items: Vec<_> = today_items
        .into_iter()
        .filter(|i| !existing_urls.contains(&i.link))
        .collect();

    use std::collections::HashSet;
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
        let clean_desc = clean_text(&item.description, 5000);
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

        match llm.chat(&analysis_prompt, false).await {
            Ok(json_str) => {
                let json_clean = json_str.trim().trim_matches('`').trim();
                let start = json_clean.find('{').unwrap_or(0);
                let end = json_clean.rfind('}').unwrap_or(json_clean.len()) + 1;
                let potential_json = &json_clean[start..end];

                if let Ok(analysis) = serde_json::from_str::<ItemAnalysis>(potential_json) {
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
                                    log::info!(
                                        "[CATEGORY FIX] Normalized '{}' -> '{}'",
                                        llm_cat,
                                        name
                                    );
                                    name
                                }
                                None => {
                                    log::warn!("[CATEGORY FIX] Unknown category '{}', falling back to '其他'", llm_cat);
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
            }
            Err(e) => log::warn!("LLM analysis failed: {}", e),
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
            match buf.prune_old_links(3 * 24 * 3600) { // 3 days TTL
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
            match buf.prune_old_clusters(7 * 24 * 3600) { // 7 days TTL
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
    let app_state = Arc::new(TriggerState {
        config: config.clone(),
        llm: llm.clone(),
        nexus: nexus.clone(),
        aggregator: aggregator.clone(),
        buffer: buffer.clone(),
        registry: registry.clone(),
        get_now: Box::new(get_now),
        running: tokio::sync::Mutex::new(false),
    });

    let app = Router::new()
        .route("/api/trigger", post(handle_trigger))
        .route("/api/status", get(handle_status))
        .route("/api/memory", get(handle_memory))
        .route("/api/health/nexus", get(handle_nexus_health))
        .with_state(app_state);

    let bind_addr = "0.0.0.0:3721";
    log::info!("Cortex trigger API listening on {}", bind_addr);
    let listener = tokio::net::TcpListener::bind(bind_addr).await.expect("Failed to bind trigger API");
    axum::serve(listener, app).await.expect("Trigger API server failed");
}

// --- HTTP Trigger API ---

struct TriggerState {
    config: Arc<Config>,
    llm: Arc<LlmClient>,
    nexus: Arc<NexusClient>,
    aggregator: Arc<NewsAggregator>,
    buffer: Arc<tokio::sync::Mutex<NewsBuffer>>,
    registry: Arc<TopicRegistry>,
    get_now: Box<dyn Fn() -> chrono::DateTime<chrono::FixedOffset> + Send + Sync>,
    running: tokio::sync::Mutex<bool>,
}

#[derive(serde::Serialize)]
struct TriggerResponse {
    success: bool,
    message: String,
}

async fn handle_trigger(
    State(state): State<Arc<TriggerState>>,
) -> Json<TriggerResponse> {
    // Prevent concurrent triggers
    {
        let mut running = state.running.lock().await;
        if *running {
            return Json(TriggerResponse {
                success: false,
                message: "A cycle is already running. Please wait.".to_string(),
            });
        }
        *running = true;
    }

    let now = (state.get_now)();
    log::info!("[Trigger API] Manual cycle triggered at {}", now);

    let config = state.config.clone();
    let llm = state.llm.clone();
    let nexus = state.nexus.clone();
    let aggregator = state.aggregator.clone();
    let buffer = state.buffer.clone();
    let state_clone = state.clone();

    // Run in background so the HTTP response returns immediately
    tokio::spawn(async move {
        let result = run_one_cycle(config, llm, nexus, aggregator, buffer, now).await;
        match &result {
            Ok(()) => log::info!("[Trigger API] Manual cycle completed successfully"),
            Err(e) => log::error!("[Trigger API] Manual cycle failed: {}", e),
        }
        *state_clone.running.lock().await = false;
    });

    Json(TriggerResponse {
        success: true,
        message: format!("Cycle triggered at {}. Running in background.", now),
    })
}

async fn handle_status(
    State(state): State<Arc<TriggerState>>,
) -> Json<serde_json::Value> {
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
        "categories": category_stats.iter().map(|(k, (count, oldest))| {
            serde_json::json!({ "name": k, "clusters": count, "oldest_ts": oldest })
        }).collect::<Vec<_>>(),
    }))
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
) -> Json<MemorySnapshot> {
    let mut system = System::new_all();
    system.refresh_all();

    let pid = get_current_pid().unwrap_or_else(|_| sysinfo::Pid::from_u32(0));
    let process = system.process(pid).map(|p| ProcessMemoryInfo {
        pid: pid.as_u32(),
        name: p.name().to_str().unwrap_or("unknown").to_string(),
        memory_mb: p.memory() as f64 / 1024.0 / 1024.0,
        virtual_memory_mb: p.virtual_memory() as f64 / 1024.0 / 1024.0,
        cpu_percent: p.cpu_usage() as f64,
    }).unwrap_or_else(|| ProcessMemoryInfo {
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
}

/// Trigger a fresh health check to Nexus (bypasses DNS cache)
async fn handle_nexus_health(
    State(state): State<Arc<TriggerState>>,
) -> Json<serde_json::Value> {
    match state.nexus.health_check().await {
        Ok(health) => {
            Json(serde_json::json!({
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
        }
        Err(e) => {
            Json(serde_json::json!({
                "success": false,
                "healthy": false,
                "error": e.to_string(),
                "message": "Failed to check Nexus health"
            }))
        }
    }
}

// Refactored from process_category

fn clean_text(input: &str, max_chars: usize) -> String {
    // 1. Strip HTML tags
    let re = Regex::new(r"<[^>]*>").unwrap();
    let no_html = re.replace_all(input, " ");

    // 2. Fix HTML Entities (Basic)
    let entity_fixed = no_html
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&#39;", "'");

    // 3. Normalize Punctuation (Half -> Full for TTS)
    // This helps the LLM and TTS model better understand sentence structure
    let punct_fixed = entity_fixed
        .replace(",", "，")
        .replace("?", "？")
        .replace("!", "！")
        .replace("(", "（")
        .replace(")", "）");
    // Note: We keep '.' as is for now, or convert to '。' if it looks like a sentence end.
    // But for mixed English content, blindly converting '.' might be risky (e.g. v2.0).
    // Let's stick to safe separators.

    // 4. Remove Noise Symbols (Common in RSS titles)
    let noise_fixed = punct_fixed
        .replace("【", " ")
        .replace("】", " ")
        .replace("[", " ")
        .replace("]", " ")
        .replace("|", " ");

    // 5. Collapse whitespace
    let re_space = Regex::new(r"\s+").unwrap();
    let clean = re_space.replace_all(&noise_fixed, " ");

    // 6. Truncate
    if clean.chars().count() > max_chars {
        let mut s: String = clean.chars().take(max_chars).collect();
        s.push_str("...");
        s
    } else {
        clean.to_string()
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct RssItem {
    title: String,
    link: String,
    description: String,
    #[allow(dead_code)]
    pub_date: Option<String>,
    source_name: Option<String>,
}

async fn fetch_rss_items(url: &str, proxy_url: Option<&str>) -> Result<Vec<RssItem>> {
    let client_builder = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .timeout(std::time::Duration::from_secs(30))
        .connect_timeout(std::time::Duration::from_secs(10));

    let client = client_builder.build()?;
    
    let mut response = client.get(url).send().await;

    // Retry with proxy if direct fetch fails and proxy is configured
    if response.is_err() {
        if let Some(proxy) = proxy_url {
            log::info!("Direct fetch failed for {}, retrying with proxy: {}", url, proxy);
            let proxy_client = reqwest::Client::builder()
                .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
                .timeout(std::time::Duration::from_secs(30))
                .connect_timeout(std::time::Duration::from_secs(10))
                .proxy(reqwest::Proxy::all(proxy)?)
                .build()?;
            response = proxy_client.get(url).send().await;
        }
    }

    let content = response?.bytes().await?;
    let cursor = std::io::Cursor::new(content);
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

            // Try summary first, then content body
            let description = entry
                .summary
                .map(|s| s.content)
                .or_else(|| entry.content.and_then(|c| c.body))
                .unwrap_or_default();

            let pub_date = entry.published.map(|d| d.to_rfc3339());

            RssItem {
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
