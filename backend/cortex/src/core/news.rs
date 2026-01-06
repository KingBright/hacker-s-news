use anyhow::Result;
use std::sync::Arc;
use tokio::time::{self, Duration};
use chrono::{Timelike, Datelike};
use crate::core::config::Config;
use crate::core::llm::LlmClient;
use crate::core::tts::TtsClient;
use crate::core::nexus::{NexusClient, ItemPayload};
use regex::Regex;
use crate::core::news_buffer::{NewsBuffer, PendingNewsItem};
use crate::core::topic_registry::TopicRegistry;
use crate::core::aggregator::NewsAggregator;
use std::sync::Mutex; 
use std::path::PathBuf;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ItemAnalysis {
    title: String, // Cleaned/Translated title
    summary: String, // 2-sentence summary
    category: String, // AI, Tech, Economy, Politics, Gaming, Other
    score: u8, // 0-10, relevance/importance
}

pub async fn run_news_loop(
    config: Config,
    llm: Arc<LlmClient>,
    tts: Arc<TtsClient>,
    nexus: Arc<NexusClient>,
    retry: Arc<crate::core::retry::RetryManager>,
    cache_dir: String,
) {
    // Determine loop interval
    let has_schedule = config.schedule_times.is_some();
    let loop_interval = if has_schedule {
        Duration::from_secs(60) // Check every minute
    } else {
        let interval_min = config.interval_min.unwrap_or(60);
        Duration::from_secs(interval_min * 60)
    };

    let mut interval = time::interval(loop_interval);
    let mut last_run_date = String::new();
    let mut first_run = true; // Trigger immediately on startup

    // Initialize v2.0 Components
    let buffer = Arc::new(tokio::sync::Mutex::new(NewsBuffer::new(&cache_dir).expect("Failed to init NewsBuffer")));
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
    let _ = aggregator.backfill_history().await.map_err(|e| log::warn!("Backfill failed: {}", e));
    let _ = registry.prune().map(|n| log::info!("Pruned {} old topics", n));

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

    loop {
        interval.tick().await;
        
        let now = chrono::Local::now();
        let current_time_str = now.format("%H:%M").to_string();
        let current_date_str = now.format("%Y-%m-%d:%H:%M").to_string();
        // User requested strictly TODAY's content (User Request 3)
        // We will define "Today" based on local time.
        let today_ymd = now.format("%Y-%m-%d").to_string();

        // Run immediately on first iteration, then follow schedule
        let should_run = if first_run {
            first_run = false;
            log::info!("Startup trigger: Running initial news cycle...");
            true
        } else if let Some(times) = &config.schedule_times {
            if times.contains(&current_time_str) {
                if last_run_date == current_date_str {
                    // Already ran this minute
                    false 
                } else {
                    true
                }
            } else {
                // Determine if we should log (once per hour to avoid spam)
                if now.minute() == 0 {
                   log::info!("Schedule Check: {} is not in configured times {:?}", current_time_str, times);
                }
                false
            }
        } else {
             true 
        };

        if !should_run {
            continue;
        }
        
        
        
        // Check for pending regeneration jobs
        if let Err(e) = aggregator.process_regenerations().await {
             log::error!("Regeneration cycle failed: {}", e);
        }

        last_run_date = current_date_str;
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
            continue;
        }

        // 2. Filter by Date (Today Only) & Dedup by Link
        // (V2EX items might have timezone issues in pub_date diff, but let's try strict string check first or parsing)
        
        let mut today_items = Vec::new();
        
        {
            let buf = buffer.lock().await;
            for item in all_candidate_items {
                // 1. Source Cache Check (Fastest L1)
                if let Ok(true) = buf.has_processed_link(&item.link) {
                    continue;
                }

                // 2. Filter out promotional/ad content (Strict Check)
                if item.title.contains("【推广】") || item.title.contains("[推广]") || item.title.contains("[广告]") {
                    log::info!("Skipping Ad/Promotion Item: {}", item.title);
                    buf.mark_link_processed(&item.link).ok();
                    continue;
                }

                // 3. Check if pub_date is today
                if let Some(pub_date_str) = &item.pub_date {
                    if pub_date_str.starts_with(&today_ymd) {
                        today_items.push(item);
                    } else {
                        // Try to parse generic DateTime
                        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(pub_date_str) {
                             let item_ymd = dt.with_timezone(&chrono::Local).format("%Y-%m-%d").to_string();
                             if item_ymd == today_ymd {
                                 today_items.push(item);
                             }
                        }
                    }
                }
            }
        }

        log::info!("Filtered {} items for today ({})", today_items.len(), today_ymd);
        if today_items.is_empty() {
            continue;
        }

        // Deduplication against Nexus
        let urls: Vec<String> = today_items.iter().map(|i| i.link.clone()).collect();
        // (Simplification: send all, Nexus returns found)
        let existing_urls = match nexus.check_urls(urls.clone()).await {
             Ok(u) => {
                 // Sync local cache: Mark existing URLs as processed
                 let buf = buffer.lock().await;
                 for url in &u {
                     buf.mark_link_processed(url).ok();
                 }
                 u
             },
             Err(e) => {
                 log::error!("Dedup failed: {}", e);
                 continue; // Skip run safety
             }
        };

        let new_items: Vec<_> = today_items.into_iter()
            .filter(|i| !existing_urls.contains(&i.link))
            // Basic internal dedup by link
            .collect();
            
        // Removed duplicated items in valid_items (multiple feeds might have same item)
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
             continue;
        }

        // 3. Classify and Buffer Steps (v2.0)
        let categories = config.categories.clone().unwrap_or_else(|| {
            vec!["Tech".to_string(), "Economy".to_string(), "Politics".to_string(), "Gaming".to_string(), "Other".to_string()]
        });
        let topics_str = categories.join(", ");

        for item in unique_items {
            let clean_desc = clean_text(&item.description, 1000); 
            let analysis_prompt = format!(
                "Analyze this news item.\nTitle: {}\nContent: {}\n\n\
                Task:\n\
                1. Classify into ONE of: [{}].\n\
                2. Summarize into 2 sentences (Chinese).\n\
                3. translate title into Chinese.\n\
                Output JSON only: {{ \"category\": \"...\", \"summary\": \"...\", \"title\": \"...\", \"score\": 8 }}",
                item.title, clean_desc, topics_str
            );

            // Sequential LLM calls (Pre-filter layer)
            match llm.chat(&analysis_prompt, false).await {
                Ok(json_str) => {
                    let json_clean = json_str.trim().trim_matches('`').trim();
                    let start = json_clean.find('{').unwrap_or(0);
                    let end = json_clean.rfind('}').unwrap_or(json_clean.len()) + 1;
                    let potential_json = &json_clean[start..end];

                    if let Ok(analysis) = serde_json::from_str::<ItemAnalysis>(potential_json) {
                        // Filter out Advertisements and Low Score
                        if analysis.category == "广告" || analysis.category == "Advertisement" || analysis.score < 6 {
                            log::info!("[FILTER] Discarding Low Quality/Ad: [{}] {} (Score: {})", analysis.category, analysis.title, analysis.score);
                            let buf = buffer.lock().await;
                            buf.mark_link_processed(&item.link).ok();
                            continue;
                        }

                        log::info!("Buffering item: [{}] {}", analysis.category, analysis.title);
                        
                        // Push to Buffer with intelligent clustering
                        let pending = PendingNewsItem {
                            title: analysis.title,
                            link: item.link.clone(),
                            description: analysis.summary,
                            category: analysis.category,
                            source_name: item.source_name.clone(),
                            timestamp: chrono::Utc::now().timestamp() as u64,
                        };
                        
                        match aggregator.push_with_clustering(pending).await {
                            Ok(is_new) => {
                                if is_new {
                                    log::debug!("Created new cluster");
                                } else {
                                    log::debug!("Merged into existing cluster");
                                }
                                let buf = buffer.lock().await;
                                buf.mark_link_processed(&item.link).ok();
                            },
                            Err(e) => log::error!("Failed to push with clustering: {}", e),
                        }
                    } else {
                        log::warn!("Failed to parse LLM analysis JSON. Skipping item.");
                    }
                },
                Err(e) => {
                    log::warn!("LLM analysis failed: {}", e);
                }
            }
        }

        // 4. Trigger Aggregation
        log::info!("Checking aggregator triggers...");
        if let Err(e) = aggregator.try_process().await {
            log::error!("Aggregator process failed: {}", e);
        }

        log::info!("News Cycle Finished.");
    }
}



// Refactored from process_category


fn clean_text(input: &str, max_chars: usize) -> String {
    // 1. Strip HTML tags
    let re = Regex::new(r"<[^>]*>").unwrap();
    let no_html = re.replace_all(input, " ");
    
    // 2. Collapse whitespace
    let re_space = Regex::new(r"\s+").unwrap();
    let clean = re_space.replace_all(&no_html, " ");
    
    // 3. Truncate
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
    
    let items = feed.entries.into_iter().map(|entry| {
        let title = entry.title.map(|t| t.content).unwrap_or_default();
        let link = entry.links.first().map(|l| l.href.clone()).unwrap_or_default();
        
        // Try summary first, then content body
        let description = entry.summary
            .map(|s| s.content)
            .or_else(|| entry.content.and_then(|c| c.body))
            .unwrap_or_default();

        let pub_date = entry.published.map(|d| d.to_rfc3339());

        RssItem {
            title,
            link,
            description,
            pub_date,
            source_name: if source_title.is_empty() { None } else { Some(source_title.clone()) },
        }
    }).filter(|i| !i.link.is_empty())
    .collect();

    Ok(items)
}
