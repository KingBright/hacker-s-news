use anyhow::Result;
use std::sync::Arc;
use tokio::time::{self, Duration};
use chrono::Timelike;
use crate::core::config::Config;
use crate::core::llm::LlmClient;
use crate::core::tts::TtsClient;
use crate::core::nexus::{NexusClient, ItemPayload};
use regex::Regex;

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
                    false 
                } else {
                    true
                }
            } else {
                false
            }
        } else {
             true 
        };

        if !should_run {
            continue;
        }
        
        
        // Check for pending regeneration jobs
        if let Ok(processed) = process_pending_jobs(&llm, &tts, &nexus, &retry, &config.hosts).await {
            if processed {
                // If we processed jobs, we can skip the heavy RSS fetch or just continue?
                // Let's continue to RSS fetch as they are independent, but logging separation is good.
                log::info!("Pending jobs processed. Continuing to RSS cycle...");
            }
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
        for item in all_candidate_items {
            // Check if pub_date is today
            if let Some(pub_date_str) = &item.pub_date {
                // Try to parse RFC3339 or simple check
                // Most feeds return RFC3339 or RSS date. 
                // Simple heuristic: does it contain YYYY-MM-DD?
                // Or better, let feed-rs handle parsing (it already does, we stored string).
                // Let's rely on string matching first for safety if parsing fails? 
                // Actually feed-rs `pub_date` we converted to rfc3339 string.
                // So "2026-01-04T..."
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

        log::info!("Filtered {} items for today ({})", today_items.len(), today_ymd);
        if today_items.is_empty() {
            continue;
        }

        // Deduplication against Nexus
        let urls: Vec<String> = today_items.iter().map(|i| i.link.clone()).collect();
        // (Simplification: send all, Nexus returns found)
        let existing_urls = match nexus.check_urls(urls.clone()).await {
             Ok(u) => u,
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

        // 3. Smart Analysis Loop
        // Map: Item -> Analysis (Class + Summary)
        // Group by Class
        let mut categorized_groups: std::collections::HashMap<String, Vec<(RssItem, ItemAnalysis)>> = std::collections::HashMap::new();
        
        // Use categories from config (or fallback to defaults)
        let categories = config.categories.clone().unwrap_or_else(|| {
            vec!["Tech".to_string(), "Economy".to_string(), "Politics".to_string(), "Gaming".to_string(), "Other".to_string()]
        });
        let topics_str = categories.join(", ");

        for item in unique_items {
            let clean_desc = clean_text(&item.description, 1000); // Allow more context for analysis
            let analysis_prompt = format!(
                "Analyze this news item.\nTitle: {}\nContent: {}\n\n\
                Task:\n\
                1. Classify into ONE of: [{}].\n\
                2. Summarize into 2 sentences (Chinese).\n\
                3. translate title into Chinese.\n\
                Output JSON only: {{ \"category\": \"...\", \"summary\": \"...\", \"title\": \"...\", \"score\": 8 }}",
                item.title, clean_desc, topics_str
            );

            // Sequential LLM calls for now (could be parallelized)
            match llm.chat(&analysis_prompt).await {
                Ok(json_str) => {
                    // Try to parse JSON
                    // LLM might output text around JSON, simple cleanup
                    let json_clean = json_str.trim().trim_matches('`').trim();
                    let start = json_clean.find('{').unwrap_or(0);
                    let end = json_clean.rfind('}').unwrap_or(json_clean.len()) + 1;
                    let potential_json = &json_clean[start..end];

                    if let Ok(analysis) = serde_json::from_str::<ItemAnalysis>(potential_json) {
                        // Filter out Advertisements
                        if analysis.category == "广告" || analysis.category == "Advertisement" {
                            log::info!("[FILTER] Discarding Advertisement: [{}] {}", analysis.category, analysis.title);
                            continue;
                        }

                        log::info!("Analyzed item: [{}] {}", analysis.category, analysis.title);
                        categorized_groups.entry(analysis.category.clone()).or_default().push((item, analysis));
                    } else {
                        log::warn!("Failed to parse LLM analysis JSON. Skipping item.");
                    }
                },
                Err(e) => {
                    log::warn!("LLM analysis failed: {}", e);
                }
            }
        }

        // 4. Generate Scripts per Category
        for (category, group_items) in categorized_groups {
            if group_items.is_empty() { continue; }
            log::info!("Generating script for '{}' with {} items", category, group_items.len());

            // Collect sources and context
            let mut context = String::new();
            let mut sources: Vec<crate::core::nexus::SourceInfo> = Vec::new();
            for (idx, (original, analysis)) in group_items.iter().enumerate() {
                context.push_str(&format!("{}. {}\nDetails: {}\n\n", idx+1, analysis.title, analysis.summary));
                sources.push(crate::core::nexus::SourceInfo {
                    url: original.link.clone(),
                    title: analysis.title.clone(),
                    summary: normalize_content(&original.description),
                });
            }
            
            // Generate and broadcast, with sources
            if let Err(e) = generate_and_broadcast(&category, &context, &llm, &tts, &nexus, &retry, &config.hosts, sources).await {
                log::error!("Failed to broadcast category {}: {}", category, e);
            }
            
            // Mark URLs as seen
            for (original, _) in group_items {
                let _ = nexus.mark_url(&original.link, &category).await;
            }
        }

        log::info!("Smart News Cycle Finished.");
    }
}

async fn process_pending_jobs(
    llm: &LlmClient,
    tts: &TtsClient,
    nexus: &NexusClient,
    retry: &crate::core::retry::RetryManager,
    hosts: &Option<Vec<crate::core::config::Host>>,
) -> Result<bool> {
    let pending_items = match nexus.fetch_pending_jobs().await {
        Ok(items) => items,
        Err(e) => {
            log::warn!("Regen: Failed to fetch pending jobs: {}", e);
            return Ok(false);
        }
    };

    if pending_items.is_empty() {
        return Ok(false);
    }

    log::info!("Regen: Found {} pending jobs", pending_items.len());

    for item in pending_items {
        // Use the ID field directly now
        let id = match item.id {
            Some(ref id) => id.clone(),
            None => {
                log::warn!("Regen: Item has no ID. Skipping.");
                continue;
            }
        };

        log::info!("Regen [Item {}]: Starting full regeneration flow...", id);

        // Extract category from title (format: "Category - Smart Daily")
        let category = item.title.split(" - ").next().unwrap_or("").trim();
        
        // Find host for this category
        let host = hosts.as_ref().and_then(|h| {
            h.iter().find(|host| host.categories.iter().any(|c| c == category))
        });
        let host_voice = host.map(|h| h.voice.clone());
        log::info!("Regen [Item {}]: Category='{}', Voice={:?}", id, category, host_voice.as_ref().map(|v| v.split('/').last().unwrap_or(v)));

        let summary = item.summary.clone().unwrap_or_default();
        if summary.is_empty() {
             log::warn!("Regen [Item {}]: No content to regenerate. Skipping.", id);
             continue;
        }

        // 1. Text Optimization (Proofread / Rewrite)
        log::info!("Regen [Item {}]: Step 1/3 - Optimizing Text (LLM)...", id);
        
        // Get host name for this category
        let host_name = host.map(|h| h.name.clone()).unwrap_or("主播".to_string());
        
        // Get time context for dynamic greetings
        let now = chrono::Local::now();
        let hour = now.hour();
        let time_period = if hour < 6 { "深夜" } 
            else if hour < 12 { "早上" }
            else if hour < 14 { "中午" }
            else if hour < 18 { "下午" }
            else if hour < 21 { "晚上" }
            else { "深夜" };
        let weekday = match now.format("%A").to_string().as_str() {
            "Saturday" | "Sunday" => "周末",
            _ => "工作日"
        };
        
        let proofread_prompt = format!(
            "请对以下新闻稿进行**深度优化和润色**。\
            \n\n**当前时间**：{} {}\
            \n\n核心要求：\
            \n1. **提升流畅度**：使语句更符合口语习惯，更加自然。\
            \n2. **修正错误**：修复任何潜在的错别字或语病。\
            \n3. **保持结构**：保留核心事实，但可以微调句子结构以提高可读性。\
            \n4. **动态开场**：根据当前时间设计自然的问候语（如'大家{}好'），然后自然引入'欢迎收听FreshLoop {}版块，我是{}'。\
            \n5. **禁止臆造**：**绝对不要**提及任何未提供的信息（如天气等），因为不知道实际信息。\
            \n6. **动态结尾**：根据时间调整结束语，最后说'我是{}，感谢您收听FreshLoop'。\
            \n7. **输出限制**：只输出优化后的全文。\
            \n\n原文：\n{}",
            time_period, weekday, time_period, category, host_name, host_name, summary
        );

        let final_summary = match llm.chat(&proofread_prompt).await {
            Ok(s) => {
                let s_clean = s.trim().to_string();
                log::info!("Regen [Item {}]: Text optimized. Size: {} -> {} chars", id, summary.len(), s_clean.len());
                if s_clean.len() < 10 { summary.clone() } else { s_clean }
            },
            Err(e) => {
                log::warn!("Regen [Item {}]: Optimization failed: {}. Using original.", id, e);
                summary.clone()
            }
        };

        // 2. Audio Generation (Tts)
        let tts_text = clean_for_tts(&final_summary);
        log::info!("Regen [Item {}]: Step 2/3 - Generating Audio (TTS)...", id);

        // Use host-specific voice if available
        let audio_data = if let Some(voice) = &host_voice {
            log::info!("Regen [Item {}]: Using voice: {}", id, voice);
            match tts.speak_with_voice(&tts_text, voice).await {
                Ok(data) => data,
                Err(e) => {
                    log::error!("Regen [Item {}]: TTS failed: {}", id, e);
                    continue;
                }
            }
        } else {
            match tts.speak(&tts_text).await {
                Ok(data) => data,
                Err(e) => {
                    log::error!("Regen [Item {}]: TTS failed: {}", id, e);
                    continue;
                }
            }
        };
        log::info!("Regen [Item {}]: Audio generated. Size: {} bytes", id, audio_data.len());

        // 3. Upload & Complete
        log::info!("Regen [Item {}]: Step 3/3 - Uploading and Completing...", id);
        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
        let base_filename = format!("regen_{}_{}", id, timestamp);
        let text_filename = format!("{}.txt", base_filename);
        let audio_filename = format!("{}.wav", base_filename);

        // Upload Text
        let _ = nexus.upload_file(final_summary.as_bytes().to_vec(), &text_filename, "text/plain").await;
        
        // Upload Audio
        let audio_url = if let Ok(url) = nexus.upload_audio(audio_data.clone(), &audio_filename).await {
             url
        } else {
             let _ = retry.cache_audio(&audio_data, &audio_filename).await;
             format!("/audio/{}", audio_filename)
        };

        // 4. Calculate Duration
        let duration_sec = if !audio_data.is_empty() {
            let cursor = std::io::Cursor::new(&audio_data);
            match hound::WavReader::new(cursor) {
                Ok(reader) => Some((reader.duration() as f64 / reader.spec().sample_rate as f64) as i64),
                Err(_) => Some((audio_data.len() as f64 / 32000.0) as i64)
            }
        } else {
            None
        };

        // 5. Complete Job
        if let Err(e) = nexus.complete_job(&id, &audio_url, &final_summary, duration_sec).await {
            log::error!("Regen [Item {}]: Failed to update database: {}", id, e);
        } else {
             log::info!("Regen [Item {}]: SUCCESS. Items updated.", id);
        }
    }

    Ok(true)
}

// Refactored from process_category
async fn generate_and_broadcast(
    category: &str,
    context_content: &str,
    llm: &LlmClient,
    tts: &TtsClient,
    nexus: &NexusClient,
    retry: &crate::core::retry::RetryManager,
    hosts: &Option<Vec<crate::core::config::Host>>,
    sources: Vec<crate::core::nexus::SourceInfo>,
) -> Result<()> {
    
    // Find host for this category
    let host = hosts.as_ref().and_then(|h| {
        h.iter().find(|host| host.categories.iter().any(|c| c == category))
    });
    
    let host_name = host.map(|h| h.name.clone()).unwrap_or("主播".to_string());
    let host_voice = host.map(|h| h.voice.clone());
    
    log::info!("Category '{}' will be hosted by: {}", category, host_name);
    
    let now = chrono::Local::now();
    let date_str = now.format("%Y年%-m月%-d日 %H:%M").to_string();
    let weekday = match now.format("%A").to_string().as_str() {
        "Monday" => "星期一", "Tuesday" => "星期二", "Wednesday" => "星期三",
        "Thursday" => "星期四", "Friday" => "星期五", "Saturday" => "星期六",
        "Sunday" => "星期天", _ => ""
    };
    let hour = now.hour();
    let time_period = if hour < 6 { "深夜" } 
        else if hour < 12 { "早上" }
        else if hour < 14 { "中午" }
        else if hour < 18 { "下午" }
        else if hour < 21 { "晚上" }
        else { "深夜" };
    
    let prompt = format!(
        "请为'{}'类别的新闻生成一份详尽深入的中文口播新闻稿。\
        \n\n当前时间：{} {}\
        \n\n核心要求：\
        \n1. 详尽报道：基于提供的摘要，整合成连贯的报道。\
        \n2. 逻辑串联：使用自然流畅的过渡词。\
        \n3. 口语风格：适合TTS语音播报，禁止使用任何格式符号如星号、下划线等。\
        \n4. 动态开场：根据时间设计亲切的问候语（如'大家{}好'），结合是否是周末/节假日等，然后自然引入'欢迎收听FreshLoop {}版块，我是{}'。\
        \n5. **禁止臆造**：**绝对不要**提及任何未提供的天气情况（如'天气晴朗'等）。\
        \n6. 动态结尾：根据时间设计温暖的结束语，最后说'我是{}，感谢您收听FreshLoop'。\
        \n7. 绝对纯净输出：只输出新闻稿纯文本内容，禁止使用Markdown格式。\
        \n\n新闻素材摘要：\n{}",
        category, weekday, time_period, time_period, category, host_name, host_name, context_content
    );

    log::info!("Generating script for {} (Host: {})", category, host_name);
    
    // ... (existing summary generation) ...
    let summary = match llm.chat(&prompt).await {
        Ok(s) => s,
        Err(e) => {
            log::warn!("LLM script generation failed: {}", e);
            // Fallback: just read the summaries
            format!("大家好，以下是{}的简讯。\n\n{}", category, context_content)
        }
    };
    log::info!("Received summary, length: {}. Starting proofreading...", summary.len());

    // 3.5 Proofreading Step (User Request)
    let proofread_prompt = format!(
        "请对以下新闻稿进行校对和润色。\
        \n\n核心要求：\
        \n1. 纠正错别字：修复明显的拼写、词语错误。\
        \n2. 修正语法：使句子更通顺，符合中文口语习惯。\
        \n3. 严禁大改：绝对不要改变原稿的结构、顺序或删减内容。只做必要的微调。\
        \n4. 纯文本输出：只输出修正后的全文，禁止使用任何Markdown格式符号如星号、下划线等。\
        \n\n原文：\n{}",
        summary
    );

    // Use summary.clone() if needed, but here `summary` is available as we didn't move it yet except into fmt?
    // Actually `format!` borrows summary.
    // The previous error was because we tried to use `summary` after moving it into `match`.
    
    let final_summary = match llm.chat(&proofread_prompt).await {
        Ok(s) => {
            log::info!("Proofreading complete. Length: {} -> {}", summary.len(), s.len());
            if s.len() < 10 {
                log::warn!("Proofreading returned empty/short text. Reverting to original.");
                summary.clone()
            } else {
                s
            }
        },
        Err(e) => {
            log::warn!("Proofreading failed: {}. Using original draft.", e);
            summary.clone()
        }
    };

    // ... (File naming, Upload, TTS, Push Item logic same as before)
    
    let safe_category = category.replace(" ", "_").to_lowercase();
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let base_filename = format!("{}_{}", safe_category, timestamp);
    let text_filename = format!("{}.txt", base_filename);
    let audio_filename = format!("{}.wav", base_filename);

    // Save the PROOFREAD summary associated with the text file
    let _ = nexus.upload_file(final_summary.as_bytes().to_vec(), &text_filename, "text/plain").await;
    
    // Clean Markdown/HTML for TTS from the PROOFREAD summary
    let tts_text = clean_for_tts(&final_summary);
    
    // Use host-specific voice if available
    let audio_data = if let Some(voice) = &host_voice {
        log::info!("Using voice: {}", voice);
        tts.speak_with_voice(&tts_text, voice).await?
    } else {
        tts.speak(&tts_text).await?
    };
    let mut audio_url = None;
    if !audio_data.is_empty() {
        // Upload logic
         if let Ok(url) = nexus.upload_audio(audio_data.clone(), &audio_filename).await {
             audio_url = Some(url);
         } else {
             // Cache logic
             let _ = retry.cache_audio(&audio_data, &audio_filename).await;
             audio_url = Some(format!("/audio/{}", audio_filename));
         }
    }

    let duration_sec = if !audio_data.is_empty() {
        let cursor = std::io::Cursor::new(&audio_data);
        match hound::WavReader::new(cursor) {
            Ok(reader) => Some((reader.duration() as f64 / reader.spec().sample_rate as f64) as i64),
            Err(_) => Some((audio_data.len() as f64 / 32000.0) as i64)
        }
    } else {
        None
    };

    let payload = ItemPayload {
        id: None,
        title: format!("{} - Smart Daily", category),
        summary: Some(final_summary),
        original_url: None, // TODO: We lose the original source URL here because we aggregated multiple items. 
                            // Ideally, we should store a list of sources, but Item structure is 1:1.
                            // For regeneration, we are regenerating the *aggregated* script.
                            // So we actually NO LONGER have a single source URL. 
        cover_image_url: None,
        audio_url,
        publish_time: Some(chrono::Utc::now().timestamp()),
        duration_sec,
        sources: if sources.is_empty() { None } else { Some(sources) },
    };

    if let Err(e) = nexus.push_item(payload.clone()).await {
        log::warn!("Failed to push item to Nexus: {}. Enqueuing for retry.", e);
        let action = crate::core::retry::RetryAction::PushItem(payload);
        retry.enqueue(action).map_err(|e| anyhow::anyhow!("Failed to enqueue retry: {}", e))?;
    } else {
        log::info!("Successfully pushed item to Nexus.");
    }
    
    Ok(())
}

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

// Comprehensive text cleaning for TTS (Markdown, HTML, XML, Entities)
fn clean_for_tts(input: &str) -> String {
    let mut cleaned = input.to_string();

    // 1. Strip HTML/XML tags
    // Matches anything between < and >, non-greedy
    let re_tags = Regex::new(r"<[^>]*>").unwrap();
    cleaned = re_tags.replace_all(&cleaned, "").to_string();

    // 2. Decode common HTML entities
    cleaned = cleaned.replace("&nbsp;", " ");
    cleaned = cleaned.replace("&amp;", "&");
    cleaned = cleaned.replace("&lt;", "<");
    cleaned = cleaned.replace("&gt;", ">");
    cleaned = cleaned.replace("&quot;", "\"");
    cleaned = cleaned.replace("&apos;", "'");
    cleaned = cleaned.replace("&#39;", "'");

    // 3. Stip Markdown symbols
    // Remove bold/italic markers (* or _)
    let re_bold = Regex::new(r"(\*\*|__|\*|_)").unwrap();
    cleaned = re_bold.replace_all(&cleaned, "").to_string();

    // Remove headers (# )
    let re_header = Regex::new(r"^#+\s+").unwrap();
    cleaned = re_header.replace_all(&cleaned, "").to_string();

    // Remove links [text](url) -> text
    let re_link = Regex::new(r"\[([^\]]+)\]\([^\)]+\)").unwrap();
    cleaned = re_link.replace_all(&cleaned, "$1").to_string();

    // Remove images ![text](url) -> ""
    let re_img = Regex::new(r"!\[[^\]]*\]\([^\)]+\)").unwrap();
    cleaned = re_img.replace_all(&cleaned, "").to_string();
    
    // Remove code blocks
    let re_code = Regex::new(r"```[^`]*```").unwrap();
    cleaned = re_code.replace_all(&cleaned, "").to_string();

    // Remove inline code `
    let re_inline = Regex::new(r"`").unwrap();
    cleaned = re_inline.replace_all(&cleaned, "").to_string();

    // 4. Collapse multiple spaces
    let re_space = Regex::new(r"\s+").unwrap();
    cleaned = re_space.replace_all(&cleaned, " ").to_string();

    cleaned.trim().to_string()
}

/// Normalize RSS content to clean Markdown format
/// Handles: HTML, CDATA, plain text, and mixed formats
fn normalize_content(raw: &str) -> String {
    let trimmed = raw.trim();
    
    // 1. Handle CDATA wrapper
    let content = if trimmed.starts_with("<![CDATA[") && trimmed.ends_with("]]>") {
        trimmed
            .trim_start_matches("<![CDATA[")
            .trim_end_matches("]]>")
            .to_string()
    } else {
        trimmed.to_string()
    };
    
    // 2. Detect if content contains HTML tags
    let has_html = content.contains('<') && content.contains('>') 
        && (content.contains("</") || content.contains("/>") || content.contains("<br") || content.contains("<p"));
    
    // 3. Process based on content type
    let processed = if has_html {
        // Convert HTML to Markdown
        html2md::parse_html(&content)
    } else {
        // Already plain text or markdown - just clean entities
        content
            .replace("&nbsp;", " ")
            .replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&#39;", "'")
            .replace("&apos;", "'")
    };
    
    // 4. Final cleanup - collapse whitespace
    let re_space = Regex::new(r"\s+").unwrap();
    re_space.replace_all(&processed, " ").trim().to_string()
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct RssItem {
    title: String,
    link: String,
    description: String,
    #[allow(dead_code)]
    pub_date: Option<String>,
}

async fn fetch_rss_items(url: &str) -> Result<Vec<RssItem>> {
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .build()?;
        
    let content = client.get(url).send().await?.bytes().await?;
    let cursor = std::io::Cursor::new(content);
    let feed = feed_rs::parser::parse(cursor)?;
    
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
        }
    }).filter(|i| !i.link.is_empty())
    .collect();

    Ok(items)
}
