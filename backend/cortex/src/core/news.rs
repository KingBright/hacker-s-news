use crate::core::aggregator::NewsAggregator;
use crate::core::config::Config;
use crate::core::llm::LlmClient;
use crate::core::news_buffer::{NewsBuffer, PendingNewsItem};
use crate::core::nexus::NexusClient;
use crate::core::topic_registry::TopicRegistry;
use crate::core::tts::TtsClient;
use anyhow::Result;
use regex::Regex;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::time::{self, Duration};
use tokio_cron_scheduler::{Job, JobScheduler};

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
        for url in feeds {
            match fetch_rss_items(url).await {
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

    // Deduplication against Nexus
    let urls: Vec<String> = today_items.iter().map(|i| i.link.clone()).collect();
    let existing_urls = match nexus.check_urls(urls.clone()).await {
        Ok(u) => {
            let buf = buffer.lock().await;
            for url in &u {
                buf.mark_link_processed(url).ok();
            }
            u
        }
        Err(e) => {
            log::error!("Dedup failed: {}", e);
            return Err(e);
        }
    };

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
                            for valid_name in &valid_category_names {
                                if valid_name.contains(llm_cat)
                                    || llm_cat.contains(valid_name.as_str())
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

    let sched = JobScheduler::new()
        .await
        .expect("Failed to create scheduler");
    let timezone_config = config.clone();
    let get_now = move || {
        if let Some(offset) = timezone_config.timezone_offset {
            let tz = chrono::FixedOffset::east_opt(offset * 3600)
                .unwrap_or(chrono::FixedOffset::east_opt(8 * 3600).unwrap());
            chrono::Utc::now().with_timezone(&tz)
        } else {
            chrono::Local::now().fixed_offset()
        }
    };

    // 1. Initial run on startup
    {
        let c = config.clone();
        let l = llm.clone();
        let n = nexus.clone();
        let a = aggregator.clone();
        let b = buffer.clone();
        let now = get_now();
        tokio::spawn(async move {
            log::info!("Startup trigger: Running initial news cycle...");
            if let Err(e) = run_one_cycle(c, l, n, a, b, now).await {
                log::error!("Initial news cycle failed: {}", e);
            }
        });
    }

    // 2. Scheduled runs
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

    // Keep loop alive
    loop {
        tokio::time::sleep(Duration::from_secs(3600)).await;
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

async fn fetch_rss_items(url: &str) -> Result<Vec<RssItem>> {
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .build()?;

    let content = client.get(url).send().await?.bytes().await?;
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
