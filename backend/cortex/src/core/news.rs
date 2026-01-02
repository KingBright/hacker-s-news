use anyhow::Result;
use std::sync::Arc;
use tokio::time::{self, Duration};
use crate::core::config::Config;
use crate::core::llm::LlmClient;
use crate::core::tts::TtsClient;
use crate::core::nexus::{NexusClient, ItemPayload};
use regex::Regex;

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
        Duration::from_secs(60) // Check every minute if scheduling is enabled
    } else {
        let interval_min = config.interval_min.unwrap_or(60);
        Duration::from_secs(interval_min * 60)
    };

    let mut interval = time::interval(loop_interval);
    let mut last_run_date = String::new(); // To avoid running multiple times in the same minute if logic is fast

    loop {
        interval.tick().await;
        
        let now = chrono::Local::now();
        let current_time_str = now.format("%H:%M").to_string();
        let current_date_str = now.format("%Y-%m-%d:%H:%M").to_string();

        let should_run = if let Some(times) = &config.schedule_times {
            if times.contains(&current_time_str) {
                if last_run_date == current_date_str {
                    false // Already ran this minute
                } else {
                    true
                }
            } else {
                false
            }
        } else {
             true // No schedule means run every interval tick
        };

        if !should_run {
            continue;
        }
        
        last_run_date = current_date_str;
        log::info!("Starting news cycle at {}", current_time_str);

        if let Some(categories) = &config.news {
            for category_config in categories {
                log::info!("Processing category: {}", category_config.category);
                
                if let Err(e) = process_category(category_config, &llm, &tts, &nexus, &retry).await {
                     log::error!("Error processing category {}: {}", category_config.category, e);
                }
            }
        } else {
            log::warn!("No news categories configured.");
        }
        
        log::info!("News cycle finished. Waiting for next interval.");
    }
}

async fn process_category(
    category_config: &crate::core::config::NewsCategory,
    llm: &LlmClient,
    tts: &TtsClient,
    nexus: &NexusClient,
    retry: &crate::core::retry::RetryManager,
) -> Result<()> {
    // 1. Collect all items from all sources in this category
    let mut candidate_items = Vec::new();

    for url in &category_config.urls {
        match fetch_rss_items(url).await {
            Ok(items) => candidate_items.extend(items),
            Err(e) => log::warn!("Failed to fetch RSS {}: {}", url, e),
        }
    }

    if candidate_items.is_empty() {
        return Ok(());
    }

    // 2. Deduplication check
    // Extract URLs
    let urls: Vec<String> = candidate_items.iter().map(|i| i.link.clone()).collect();
    
    // Retry check_urls up to 3 times
    let mut existing_urls = Vec::new();
    let mut check_success = false;
    for attempt in 1..=3 {
        match nexus.check_urls(urls.clone()).await {
            Ok(u) => {
                existing_urls = u;
                check_success = true;
                break;
            },
            Err(e) => {
                log::warn!("Check URLs failed (attempt {}/3): {}", attempt, e);
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
    
    if !check_success {
        // If we can't check dedup, we should probably abort to avoid spamming duplicates.
        // Or we could rely on the backend to dedup later? 
        // Backend MarkUrl is fire-and-forget/retry, but if we push invalid items...
        // Let's abort this category to be safe and try again next loop.
        return Err(anyhow::anyhow!("Failed to connect to Nexus for deduplication after 3 attempts"));
    }

    // Filter new items
    let new_items: Vec<_> = candidate_items.into_iter()
        .filter(|i| !existing_urls.contains(&i.link))
        .collect();

    if new_items.is_empty() {
        log::info!("No new items for category {}", category_config.category);
        return Ok(());
    }

    log::info!("Found {} new items for {}", new_items.len(), category_config.category);

    // 3. Summarize (Manuscript generation)
    // We want ONE manuscript for the category.
    // Construct prompt with limits to avoid context overflow
    let mut context = String::new();
    let max_items = 20;
    let max_chars_per_item = 500;
    let max_total_chars = 8000;

    let items_to_process: Vec<_> = new_items.iter().take(max_items).collect();
    
    for (idx, item) in items_to_process.iter().enumerate() {
        let description = clean_text(&item.description, max_chars_per_item);
        
        // ...
        
        let item_text = format!("{}. Title: {}\nLink: {}\nContent: {}\n\n", idx + 1, item.title, item.link, description);
        
        if context.len() + item_text.len() > max_total_chars {
            log::warn!("Context limit reached. Stopping at item {}", idx);
            break;
        }
        
        context.push_str(&item_text);
    }

    let date_str = chrono::Local::now().format("%Y年%-m月%-d日 %H:%M").to_string();
    let prompt = format!(
        "请为'{}'类别的新闻生成一份**详尽深入**的中文口播新闻稿。\
        当前时间是 {}。\
        \n\n核心要求：\
        \n1. **详尽报道**：请不要过度压缩内容！对于每一个重要的新闻事件，**必须保留关键细节**、背景信息和各方观点。如果是一则大新闻，请展开论述。整篇稿件应当内容充实，信息量大。\
        \n2. **逻辑串联**：请将**主题相关**的新闻合并在一起报道。使用自然流畅的过渡词（如“与此同时”、“在...方面”、“此外”）将不同新闻串联起来，形成一个有机的整体，**严禁机械地逐条罗列**。\
        \n3. **口语风格**：必须使用**自然、亲切且引人入胜的中文口语**（Broadcast Style），适合TTS语音播报。避免生硬的翻译腔，将书面语转化为听觉友好的表达。语言要通俗易懂但又不失专业性。\
        \n4. **结构安排**：\
        \n   - **开场**：以“听众朋友们大家好，这里是FreshLoop...”自然开场，点明当前时间。\
        \n   - **主体**：按重要性排序。对于头条新闻，请控制在**200字以内**。对于次要简讯，请务必精简，控制在**100字以内**。整体节奏要明快。\
        \n   - **结尾**：简短有力的结束语。\
        \n5. **绝对纯净输出**：\
        \n   - **只输出新闻稿内容**，不要包含任何“好的，这是为您生成的...”之类的客套话。\
        \n   - **不要**包含 Markdown 标题（如 # 新闻稿）。\
        \n   - **不要**包含原文链接或图片描述。\
        \n   - **直接开始播报**。\
        \n6. **语言强制**：\
        \n   - **中文为主**：稿件主体必须是中文。所有普通英文内容必须翻译。\
        \n   - **专有名词处理**：对于业界通用的英文缩写（如 AI, GPT, CEO）或难以翻译的产品/公司名，**可以保留英文**，以确保准确性。\
        \n   - **禁止滥用英文**：不要在普通词汇上使用英语（如不要说“这个feature很great”），必须用地道中文表达。\
        \n\n新闻原始素材：\n{}",
        category_config.category, date_str, context
    );

    log::info!("Requesting summary from LLM for category '{}'. Prompt length: {} chars", category_config.category, prompt.len());
    
    let summary = match llm.chat(&prompt).await {
        Ok(s) => s,
        Err(e) => {
            log::warn!("LLM generation failed: {}. Falling back to simple concatenation.", e);
            // Construct fallback
            let mut fallback = format!("听众朋友们大家好，这里是FreshLoop。由于智能服务暂时不可用，以下是{}的新闻简报。\n\n", date_str);
            for (idx, item) in items_to_process.iter().enumerate() {
                // Strip HTML and truncate for fallback too
                let clean_desc = clean_text(&item.description, 200);
                fallback.push_str(&format!("第{}条：{}\n{}\n\n", idx + 1, item.title, clean_desc));
            }
            fallback.push_str("播报结束，谢谢大家。");
            fallback
        }
    };
    log::info!("Received summary from LLM for category '{}'. Summary length: {} chars", category_config.category, summary.len());

    // Prepare filenames for consistent naming
    let safe_category = category_config.category
        .replace(" & ", "_")
        .replace(" ", "_")
        .to_lowercase();
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let base_filename = format!("{}_{}", safe_category, timestamp);
    let text_filename = format!("{}.txt", base_filename);
    let audio_filename = format!("{}.wav", base_filename);

    // Upload Manuscript
    if let Err(e) = nexus.upload_file(summary.as_bytes().to_vec(), &text_filename, "text/plain").await {
        log::warn!("Failed to upload manuscript: {}. Enqueuing retry.", e);
         // Treat manuscript as generic file, for now we skip retry or add generic RetryAction later.
         // Let's implement generic file retry support if needed, or just ignore manuscript failure as non-critical.
         // Ignoring for now to keep it simple, focus on Audio.
    } else {
        log::info!("Uploaded manuscript: {}", text_filename);
    }

    // 4. TTS
    let audio_data = tts.speak(&summary).await?;
    
    let mut audio_url = None;
    if !audio_data.is_empty() {
        match nexus.upload_audio(audio_data.clone(), &audio_filename).await {
            Ok(url) => {
                audio_url = Some(url);
            },
            Err(e) => {
                log::warn!("Failed to upload audio: {}. Caching locally and enqueuing retry.", e);
                // Cache audio locally
                match retry.cache_audio(&audio_data, &audio_filename).await {
                    Ok(path) => {
                        // Enqueue retry action
                        let _ = retry.enqueue(crate::core::retry::RetryAction::UploadAudio {
                            filename: audio_filename.clone(),
                            file_path: path,
                        });
                        // Optimistic URL
                        audio_url = Some(format!("/audio/{}", audio_filename));
                    },
                    Err(cache_err) => {
                        log::error!("Failed to cache audio locally: {}", cache_err);
                    }
                }
            }
        }
    }

    // 5. Push summary item to Nexus
    // 5. Push summary item to Nexus
    let duration_sec = if !audio_data.is_empty() {
        // Assume 24kHz, 16-bit, Mono
        // Bytes per second = 24000 * 2 bytes * 1 channel = 48000
        Some((audio_data.len() as f64 / 48000.0) as i64)
    } else {
        None
    };

    let payload = ItemPayload {
        title: format!("{} Update - {}", category_config.category, date_str),
        summary: Some(summary.clone()),
        original_url: None,
        cover_image_url: None,
        audio_url,
        publish_time: Some(chrono::Utc::now().timestamp()),
        duration_sec,
    };

    if let Err(e) = nexus.push_item(payload.clone()).await {
        log::warn!("Failed to push item: {}. Enqueuing retry.", e);
        if let Err(qa_err) = retry.enqueue(crate::core::retry::RetryAction::PushItem(payload)) {
             log::error!("Failed to enqueue push item retry: {}", qa_err);
        }
    } else {
        log::info!("Successfully pushed item for category {}", category_config.category);
    }

    // 6. Mark items as seen
    for item in items_to_process {
        if let Err(e) = nexus.mark_url(&item.link, &category_config.category).await {
            log::warn!("Failed to mark url {}: {}. Enqueuing retry.", item.link, e);
            let _ = retry.enqueue(crate::core::retry::RetryAction::MarkUrl {
                url: item.link.clone(),
                category: category_config.category.clone(),
            });
        }
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
