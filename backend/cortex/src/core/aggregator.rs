use crate::core::config::Host;
use chrono::{Local, Datelike};
use chinese_lunisolar_calendar::LunisolarDate;
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::Mutex; 
use crate::core::news_buffer::{NewsBuffer, PendingNewsItem, ClusterData};
use crate::core::topic_registry::TopicRegistry;
use crate::core::llm::LlmClient;
use crate::core::tts::TtsClient;
use crate::core::nexus::{NexusClient, ItemPayload};
use regex::Regex;
use std::io::Write;

// --- Trace Logger ---
#[derive(Debug, serde::Serialize)]
struct TraceStep {
    timestamp: String,
    step_name: String,
    details: String,
    llm_prompt: Option<String>,
    llm_response: Option<String>,
}

pub struct TraceLogger {
    id: String,
    category: String,
    start_time: chrono::DateTime<chrono::Local>,
    steps: Vec<TraceStep>,
}

impl TraceLogger {
    pub fn new(category: &str) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            category: category.to_string(),
            start_time: chrono::Local::now(),
            steps: Vec::new(),
        }
    }

    pub fn log(&mut self, step_name: &str, details: &str) {
        self.steps.push(TraceStep {
            timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
            step_name: step_name.to_string(),
            details: details.to_string(),
            llm_prompt: None,
            llm_response: None,
        });
        // Mirror to stdout
        log::info!("[Trace: {}] {}: {}", self.step_name_slug(), step_name, details);
    }

    pub fn log_llm(&mut self, step_name: &str, details: &str, prompt: &str, response: &str) {
        self.steps.push(TraceStep {
            timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
            step_name: step_name.to_string(),
            details: details.to_string(),
            llm_prompt: Some(prompt.to_string()),
            llm_response: Some(response.to_string()),
        });
        log::info!("[Trace: {}] {} (LLM Invoked)", self.step_name_slug(), step_name);
    }

    fn step_name_slug(&self) -> String {
        self.category.chars().take(4).collect()
    }

    pub fn save(&self) -> Result<String> {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let log_dir = std::path::Path::new(&home).join(".freshloop/logs/traces");
        std::fs::create_dir_all(&log_dir)?;
        
        let filename = format!("trace_{}_{}_{}.md", 
            self.start_time.format("%Y%m%d_%H%M"), 
            self.category.replace(" ", "_"), 
            self.id.chars().take(8).collect::<String>());
            
        let path = log_dir.join(&filename);
        let mut file = std::fs::File::create(&path)?;
        
        writeln!(file, "# Execution Trace Report")?;
        writeln!(file, "- **Task ID**: {}", self.id)?;
        writeln!(file, "- **Category**: {}", self.category)?;
        writeln!(file, "- **Start Time**: {}", self.start_time)?;
        writeln!(file, "- **Total Steps**: {}\n", self.steps.len())?;
        
        for (i, step) in self.steps.iter().enumerate() {
            writeln!(file, "## {}. {} ({})", i+1, step.step_name, step.timestamp)?;
            writeln!(file, "{}", step.details)?;
            
            if let Some(prompt) = &step.llm_prompt {
                writeln!(file, "\n**LLM Prompt**:\n```text\n{}\n```", prompt)?;
            }
            if let Some(resp) = &step.llm_response {
                 writeln!(file, "\n**LLM Response**:\n```text\n{}\n```", resp)?;
            }
            writeln!(file, "\n---\n")?;
        }
        
        Ok(path.to_string_lossy().to_string())
    }
}

pub struct NewsAggregator {
    buffer: Arc<tokio::sync::Mutex<NewsBuffer>>,
    registry: Arc<TopicRegistry>,
    llm: Arc<LlmClient>,
    tts: Arc<TtsClient>,
    nexus: Arc<NexusClient>,
    hosts: Option<Vec<Host>>,
}

impl NewsAggregator {
    pub fn new(
        buffer: Arc<tokio::sync::Mutex<NewsBuffer>>,
        registry: Arc<TopicRegistry>,
        llm: Arc<LlmClient>,
        tts: Arc<TtsClient>,
        nexus: Arc<NexusClient>,
        hosts: Option<Vec<Host>>,
    ) -> Self {
        Self { buffer, registry, llm, tts, nexus, hosts }
    }

    /// Primary entry point: Check buffer stats, flush specific categories if ready
    /// Now works with pre-clustered data (clustering happens at push time)
    pub async fn try_process(&self) -> Result<()> {
        // Thresholds - now based on cluster count (true unique topics)
        const MIN_CLUSTERS: usize = 10;
        const MAX_WAIT_SEC: u64 = 6 * 3600; // 6 Hours (reduced from 12)
        const MIN_CLUSTERS_FOR_EPISODE: usize = 3;

        let stats = {
            let buf = self.buffer.lock().await;
            buf.get_category_stats()?
        };
        
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
        
        let mut categories_to_flush = Vec::new();

        for (category, (count, oldest_ts)) in stats {
            let wait_time = if now > oldest_ts { now - oldest_ts } else { 0 };
            
            // Flush Rule: Unique Clusters >= 10 OR Wait > 6h
            if count >= MIN_CLUSTERS || wait_time > MAX_WAIT_SEC {
                log::info!("Triggering Flush for [{}]: Clusters={}, Wait={}s", category, count, wait_time);
                categories_to_flush.push(category);
            }
        }
        
        for cat in categories_to_flush {
            let clusters = {
                let buf = self.buffer.lock().await;
                buf.get_category_clusters(&cat)?
            };
            
            if clusters.is_empty() {
                continue;
            }
            
            // Check minimum clusters for episode
            if clusters.len() < MIN_CLUSTERS_FOR_EPISODE {
                log::info!("Postponing [{}]: Only {} clusters, need at least {}", cat, clusters.len(), MIN_CLUSTERS_FOR_EPISODE);
                continue;
            }
            
            // Collect IDs for potential removal
            let cluster_ids: Vec<String> = clusters.iter().map(|c| c.id.clone()).collect();

            // Process the category (Peek -> Process)
            match self.process_clusters(&cat, &clusters).await {
                Ok(true) => {
                    // Success (Ack): Remove processed clusters
                    log::info!("Successfully processed [{}], removing {} clusters.", cat, cluster_ids.len());
                    let buf = self.buffer.lock().await;
                    if let Err(e) = buf.remove_clusters(&cat, &cluster_ids) {
                        log::error!("Failed to remove clusters from DB after processing: {}", e);
                    }
                },
                Ok(false) => {
                    // Postponed/Skipped: Do nothing (Repo keeps data)
                    log::info!("Processing [{}]: Postponed or Skipped. Data retained.", cat);
                },
                Err(e) => {
                    // Failed: Critical Error (e.g. LLM Crash)
                    // Configurable Strict Mode: Abort entire cycle to prevent cascading failures
                    log::error!("CRITICAL: Failed to process category [{}]: {}. Aborting flush cycle to trigger retry later.", cat, e);
                    return Err(e);
                }
            }
        }
        Ok(())
    }

    /// Admin Regeneration Loop (Moved from news.rs)
    pub async fn process_regenerations(&self) -> Result<()> {
        let pending = self.nexus.fetch_pending_jobs().await?;
        if pending.is_empty() { return Ok(()); }

        log::info!("Found {} pending regeneration jobs.", pending.len());

        for job in pending {
             // Treat "Title - Smart Daily" as category for reverse lookup or fallback to "Other"
             let category = if job.title.contains(" - ") {
                 job.title.split(" - ").next().unwrap_or("Other")
             } else {
                 "Other"
             };
             
             let context = job.summary.as_deref().unwrap_or("");
             
             log::info!("Regenerating [Item {}] (Category: {})", job.id.as_deref().unwrap_or("?"), category);
             
             
             // UNIFIED LOGIC: Use produce_episode
             let (final_script, _generated_title, audio_bytes, duration, _) = self.produce_episode(
                 category,
                 context,
                 None, // No items for regeneration
                 true // is_regeneration
             ).await?;
             
             // Upload Audio if present (Manual upload for Regen flow)
             let mut audio_url = String::new();
             if let Some(bytes) = audio_bytes {
                 let file_name = format!("regen_{}.mp3", uuid::Uuid::new_v4());
                 audio_url = self.nexus.upload_audio(bytes, &file_name).await.unwrap_or_default();
             }

             // Complete Job
             if let Some(id) = &job.id {
                 self.nexus.complete_job(id, &audio_url, &final_script, Some(duration)).await?;
             }
        }
        Ok(())
    }

    /// Backfill local history from Nexus (Migration Strategy)
    pub async fn backfill_history(&self) -> Result<()> {
        log::info!("Starting History Backfill from Nexus...");
        let recent_items = self.nexus.fetch_recent_items(200).await?;
        let mut count = 0;
        
        for item in recent_items {
            let summary = item.summary.clone().unwrap_or_default();
            let combined_text = format!("{} {}", item.title, summary);
            // Use new method to store full details for better follow-up detection
            self.registry.record_topic_with_details(&combined_text, &item.title, &summary)?;
            count += 1;
        }
        
        log::info!("Backfilled {} topics into local registry.", count);
        Ok(())
    }

    /// Push a new item with intelligent clustering:
    /// 1. SimHash coarse filter to find potentially similar clusters
    /// 2. LLM verification to confirm and merge
    /// 3. Store as new or merged cluster
    pub async fn push_with_clustering(&self, item: PendingNewsItem) -> Result<bool> {
        const SIMHASH_THRESHOLD: u32 = 10; // Hamming distance threshold for coarse filtering
        
        let item_hash = ClusterData::calculate_simhash(&item.title, &item.description);
        
        // 1. Find similar clusters in buffer
        let similar_clusters = {
            let buf = self.buffer.lock().await;
            buf.find_similar_clusters(&item.category, item_hash, SIMHASH_THRESHOLD)?
        };
        
        if similar_clusters.is_empty() {
            // No similar clusters, create new one
            log::info!("New cluster: {}", item.title);
            let cluster = ClusterData::new(item);
            let buf = self.buffer.lock().await;
            buf.store_cluster(&cluster)?;
            return Ok(true);  // New cluster created
        }
        
        // 2. LLM verification for the most similar cluster
        let mut best_match: Option<ClusterData> = None;
        
        // Optimization: Fast path for exact title matches (Check Main + Related)
        let mut exact_match_found = false;
        if let Some(exact_match_cluster) = similar_clusters.iter().find(|c| {
            // Check main item
            if c.main_item.title.trim().eq_ignore_ascii_case(item.title.trim()) { return true; }
            // Check related items
            c.related_items.iter().any(|r| r.title.trim().eq_ignore_ascii_case(item.title.trim()))
        }) {
            log::info!("Fast-track: Found exact title match for '{}'", item.title);
            best_match = Some(exact_match_cluster.clone());
            exact_match_found = true;
        } else {
            // Normal path: LLM verification
            for cluster in similar_clusters {
                let dist = ClusterData::hamming_distance(cluster.simhash, item_hash);
                log::info!("SimHash Candidate: '{}' (Dist: {}) vs New: '{}'", cluster.main_item.title, dist, item.title);
                
                let is_same = self.llm_verify_same_topic(&item, &cluster).await?;
                if is_same {
                    best_match = Some(cluster);
                    break;
                }
            }
        }
        
        if let Some(mut matched_cluster) = best_match {
            // 3. Merge into existing cluster
            log::info!("Merging '{}' into cluster '{}'", item.title, matched_cluster.main_item.title);
            
            // Optimization: If title is identical, skip LLM merge cost
            if exact_match_found || item.title.trim().eq_ignore_ascii_case(matched_cluster.main_item.title.trim()) {
                // Strict Duplicate Check: Check against ALL items in cluster
                // If this new item is identical (Title+Content) to ANY existing item, discard it.
                let is_strict_duplicate = 
                    (item.title.trim().eq_ignore_ascii_case(matched_cluster.main_item.title.trim()) && 
                     item.description.trim() == matched_cluster.main_item.description.trim()) ||
                    matched_cluster.related_items.iter().any(|r| 
                        r.title.trim().eq_ignore_ascii_case(item.title.trim()) && 
                        r.description.trim() == item.description.trim()
                    );

                if is_strict_duplicate {
                    log::info!("Discarding exact duplicate item (Title+Content match in cluster): {}", item.title);
                    return Ok(false); 
                }
                
                log::info!("Skipping LLM merge for identical title: {}", item.title);
                matched_cluster.add_related(item);
            } else {
                // LLM merge to create combined summary
                let merged_summary = self.llm_merge_items(&matched_cluster, &item).await?;
                
                matched_cluster.add_related(item);
                if let Some((title, summary)) = merged_summary {
                    matched_cluster.set_merged_summary(title, summary);
                }
            }
            
            // Update cluster in buffer
            let buf = self.buffer.lock().await;
            buf.store_cluster(&matched_cluster)?;
            return Ok(false);  // Merged into existing
        }
        
        // No match confirmed by LLM, create new cluster
        log::info!("New cluster (LLM verified): {}", item.title);
        let cluster = ClusterData::new(item);
        let buf = self.buffer.lock().await;
        buf.store_cluster(&cluster)?;
        Ok(true)
    }

    /// LLM verification: Are these two items about the same topic?
    async fn llm_verify_same_topic(&self, item: &PendingNewsItem, cluster: &ClusterData) -> Result<bool> {
        let prompt = format!(
            "判断以下两条新闻是否报道同一个事件/话题？\n\n\
            新闻A:\n标题: {}\n摘要: {}\n\n\
            新闻B:\n标题: {}\n摘要: {}\n\n\
            判断标准：同一事件指同一个具体事件、产品、人物动态，而非仅仅领域相似。\n\
            仅回答 YES 或 NO。",
            item.title, item.description,
            cluster.main_item.title, 
            cluster.merged_summary.as_ref().unwrap_or(&cluster.main_item.description)
        );
        
        let response = self.llm.chat(&prompt, false).await?;
        let answer = response.trim().to_uppercase();
        Ok(answer.contains("YES"))
    }

    /// LLM merge: Combine item into cluster with merged summary
    async fn llm_merge_items(&self, cluster: &ClusterData, new_item: &PendingNewsItem) -> Result<Option<(String, String)>> {
        let existing_summary = cluster.merged_summary.as_ref().unwrap_or(&cluster.main_item.description);
        
        let mut prompt = format!(
            "Role: Senior Intelligence Analyst (资深情报分析师)。\n\n任务：将以下多来源信息综合成一份权威的简报模块。\n\n【策略 - 请根据内容类型自适应】：\n- **硬新闻/财经**：准确性第一。保留所有具体数字、日期、人名、公司名。遵循 5W1H 原则。\n- **软新闻/观点**：捕捉核心论点、情感弧线或独特氛围。提炼“金句”。\n- **低质量/碎片化**：如果来源行文混乱，请将其重构为逻辑通顺、符合新闻标准的文稿。修复所有语法错误。\n\n已有内容:\n标题: {}\n摘要: {}\n\n新内容:\n标题: {}\n摘要: {}\n\n要求：\n1. 极高信息密度：拒绝废话。\n2. 输出JSON格式: {{\"title\": \"综合标题\", \"summary\": \"综合摘要\"}}",
            cluster.main_item.title, existing_summary,
            new_item.title, new_item.description
        );
        
        let mut attempts = 0;
        const MAX_RETRIES: usize = 3;

        loop {
            attempts += 1;
            let response = self.llm.chat(&prompt, false).await?;
            
            // Parse JSON
            let json_clean = response.trim().trim_matches('`').trim();
            if let Some(start) = json_clean.find('{') {
                if let Some(end) = json_clean.rfind('}') {
                    if start <= end {
                        let json_str = &json_clean[start..=end];
                        if let Ok(result) = serde_json::from_str::<serde_json::Value>(json_str) {
                            let title = result["title"].as_str().unwrap_or(&cluster.main_item.title).to_string();
                        let summary = result["summary"].as_str().unwrap_or(existing_summary).to_string();
                        
                        // Editor Review Loop for Summary
                        // Skip review if it's the first attempt and looks plausible? 
                        // No, user wants strict checking.
                        let (passed, critique) = self.review_summary(&title, &summary).await?;
                        if passed {
                            return Ok(Some((title, summary)));
                        }
                        
                        if attempts >= MAX_RETRIES {
                            log::warn!("Editor rejected summary 3 times. Accepted last draft. Critique: {}", critique);
                            return Ok(Some((title, summary)));
                        }

                        log::info!("Editor rejected summary (Attempt {}): {}. Regenerating...", attempts, critique);
                        prompt.push_str(&format!("\n\n【主编反馈】\n你的上一版摘要被打回了，原因：{}\n请保留更多细节，重新合并。", critique));
                        continue;
                    }
                }
            }
        }
            
            if attempts >= MAX_RETRIES {
                break;
            }
        }
        
        Ok(None)
    }

    /// Helper: Review merged summary quality
    async fn review_summary(&self, title: &str, summary: &str) -> Result<(bool, String)> {
        let prompt = format!(
            "Role: Executive Editor (执行主编)。\n\n任务：严格质检这条新闻摘要。\n\n【标题】{}\n【摘要】{}\n\n审核标准：\n1. **Hook (吸引力)**：第一句话是否足够吸引人？\n2. **Clarity (清晰度)**：没有任何语病、错别字或歧义。\n3. **Detail (细节)**：保留了关键数据和实体名称，没有被过度概括。\n4. **Correction (校对)**：必须充当校对员，如果发现任何错别字或语句不通，视为不合格！\n\n输出格式（JSON）：\n{{ \"pass\": true, \"critique\": \"完美\" }}\n或\n{{ \"pass\": false, \"critique\": \"第一句逻辑不通，错别字'其'应为'起'，且缺少具体金额...\" }}",
            title, summary
        );

        let response = self.llm.chat(&prompt, false).await?;
         // Simple JSON parse
        let json_str = response.trim().trim_matches('`').trim();
        let start = json_str.find('{').unwrap_or(0);
        let end = json_str.rfind('}').unwrap_or(json_str.len().saturating_sub(1));
        
        // Ensure range is valid
        let json_valid = if start <= end && end < json_str.len() {
             &json_str[start..=end]
        } else {
             "{}"
        };
        
        #[derive(serde::Deserialize)]
        struct Review {
            pass: bool,
            critique: String,
        }

        let review: Review = serde_json::from_str(json_valid)
            .unwrap_or(Review { pass: true, critique: "JSON Parse Error".to_string() });
            
        Ok((review.pass, review.critique))
    }

    /// Check if a previously reported topic has substantial new information
    /// Returns Some(update_summary) if there's new info, None if it should be discarded
    async fn check_for_updates(&self, cluster: &ClusterData, current_summary: &str, prev_record: &crate::core::topic_registry::TopicRecord) -> Result<Option<String>> {
        // Use previous summary if available, otherwise just use title
        let prev_content = if !prev_record.summary.is_empty() {
            format!("标题: {}\n摘要: {}", prev_record.title, prev_record.summary)
        } else {
            format!("标题: {}", cluster.main_item.title)
        };
        
        let prompt = format!(
            "Role: Breaking News Desk (突发新闻中心)。\n判断新内容是否构成【实质性更新】。\n\n【之前报道】\n{}\n\n【新线索】\n标题: {}\n摘要: {}\n\n判定标准：\n- **NO**: 重复信息、单纯的观点重申、无关痛痒的细节修饰。\n- **YES**: 新的数据、官方回应、事件进入下一阶段、结果反转。\n\n输出格式（仅输出JSON）：\n{{\n  \"has_update\": true或false,\n  \"update_summary\": \"如有更新，请写一段简练的后续报道（Focus on the NEW info only）\"\n}}",
            prev_content, cluster.main_item.title, current_summary
        );
        
        let response = self.llm.chat(&prompt, false).await?;
        
        // Parse JSON
        let json_clean = response.trim().trim_matches('`').trim();
        if let Some(start) = json_clean.find('{') {
            if let Some(end) = json_clean.rfind('}') {
                if start <= end {
                    let json_str = &json_clean[start..=end];
                    if let Ok(result) = serde_json::from_str::<serde_json::Value>(json_str) {
                        let has_update = result["has_update"].as_bool().unwrap_or(false);
                        if has_update {
                            if let Some(update_summary) = result["update_summary"].as_str() {
                                return Ok(Some(update_summary.to_string()));
                            }
                        }
                    }
                }
            }
        }
        
        Ok(None)
    }
    /// Process pre-clustered data for broadcast generation
    async fn process_clusters(&self, category: &str, clusters: &[ClusterData]) -> Result<bool> {
        log::info!("Processing [{}]: {} clusters", category, clusters.len());

        const MIN_UNIQUE_TOPICS: usize = 3;
        
        // Build context from clusters
        let mut source_text = String::new();
        let mut all_sources = Vec::new();
        let mut broadcast_items = Vec::new();
        let mut unique_topic_count = 0;

        for (idx, cluster) in clusters.iter().enumerate() {
            let summary = cluster.merged_summary.as_ref().unwrap_or(&cluster.main_item.description);
            let combined_text = format!("{} {}", cluster.main_item.title, summary);
            
            // Check global history for previously reported topics
            if let Ok(Some(prev_record)) = self.registry.is_duplicate(&combined_text) {
                // Topic was previously reported - check if there's new information
                match self.check_for_updates(&cluster, summary, &prev_record).await {
                    Ok(Some(update_summary)) => {
                        // Has new information - include as a follow-up story
                        log::info!("Follow-up story: {}", cluster.main_item.title);
                        unique_topic_count += 1;
                        
                        // Update the registry with new content
                        let _ = self.registry.record_topic_with_details(
                            &combined_text, &cluster.main_item.title, summary);
                        
                        let source_str = cluster.main_item.source_name.as_deref().unwrap_or("Unknown");
                        source_text.push_str(&format!(
                            "### Story {} (后续报道)\nSource: {}\nTitle: {}\nSummary: {}\n\n---\n\n", 
                            idx + 1, source_str, cluster.main_item.title, update_summary
                        ));
                        
                        all_sources.push(crate::core::nexus::SourceInfo {
                            url: cluster.main_item.link.clone(),
                            title: format!("[更新] {}", cluster.main_item.title),
                            summary: update_summary.clone(),
                        });

                        broadcast_items.push(BroadcastItem {
                            id: idx + 1,
                            title: format!("[更新] {}", cluster.main_item.title),
                            summary: update_summary.clone(),
                            source_name: cluster.main_item.source_name.as_deref().unwrap_or("Unknown").to_string(),
                            original_url: cluster.main_item.link.clone(),
                            is_update: true,
                        });
                    },
                    Ok(None) => {
                        // No substantial new information - discard
                        log::info!("Skipping (no new info): {}", cluster.main_item.title);
                        continue;
                    },
                    Err(e) => {
                        log::warn!("Update check failed: {}, skipping", e);
                        continue;
                    }
                }
            } else {
                // New topic - record with full details
                self.registry.record_topic_with_details(
                    &combined_text, &cluster.main_item.title, summary)?;
                unique_topic_count += 1;
                
                let source_str = cluster.main_item.source_name.as_deref().unwrap_or("Unknown");
                source_text.push_str(&format!("### Story {}\nSource: {}\nTitle: {}\nSummary: {}\n\n---\n\n", 
                    idx + 1, source_str, cluster.main_item.title, summary));
                
                all_sources.push(crate::core::nexus::SourceInfo {
                    url: cluster.main_item.link.clone(),
                    title: cluster.main_item.title.clone(),
                    summary: summary.clone(),
                });

                broadcast_items.push(BroadcastItem {
                    id: idx + 1,
                    title: cluster.main_item.title.clone(),
                    summary: summary.clone(),
                    source_name: cluster.main_item.source_name.as_deref().unwrap_or("Unknown").to_string(),
                    original_url: cluster.main_item.link.clone(),
                    is_update: false,
                });
            }
            
            // Add related items as sources
            for related in &cluster.related_items {
                all_sources.push(crate::core::nexus::SourceInfo {
                    url: related.link.clone(),
                    title: related.title.clone(),
                    summary: related.description.clone(),
                });
            }
        }

        if unique_topic_count < MIN_UNIQUE_TOPICS {
            log::info!("Postponing [{}]: Only {} unique topics after dedup", category, unique_topic_count);
            return Ok(false);
        }

        if source_text.is_empty() {
            return Ok(false);
        }

        log::info!("Generating episode for [{}]: {} unique topics", category, unique_topic_count);
        
        log::info!("Generating episode for [{}]: {} unique topics", category, unique_topic_count);
        
        // Call produce_episode with smart flow enabled (via items)
        let result = self.produce_episode(
            category, 
            &source_text,
            Some(&broadcast_items),
            false
        ).await;

        let (script, generated_title, audio_bytes, duration, skipped) = match result {
            Ok(r) => r,
            Err(e) => {
                log::error!("Failed to generate episode: {}. Data retained.", e);
                // No need to restore, data is still in DB
                return Err(e);
            }
        };

        if skipped {
            log::warn!("Skipped digest generation for category: {}", category);
            return Ok(false);
        }

        // 3. Push to Nexus (Multipart Atomic)
        let fallback_title = format!("{} News Digest: {} stories", category, unique_topic_count);
        let final_title = generated_title.unwrap_or(fallback_title);
        let payload = ItemPayload {
            id: None,
            title: final_title,
            summary: Some(script),
            original_url: Some(all_sources.first().map(|s| s.url.clone()).unwrap_or_default()), 
            cover_image_url: None,
            audio_url: None, // Will be filled by Nexus if file is provided
            publish_time: Some(chrono::Utc::now().timestamp()),
            duration_sec: Some(duration),
            sources: Some(all_sources),
            category: Some(category.to_string()),
        };
        
        self.nexus.push_item_multipart(payload, audio_bytes).await?;
        log::info!("Published Digest for [{}]", category);
        
        Ok(true)
    }

    // --- Core Unified Content Engine ---

    async fn produce_episode(&self, category: &str, context: &str, items: Option<&[BroadcastItem]>, is_regen: bool) -> Result<(String, Option<String>, Option<Vec<u8>>, i64, bool)> {
        // 1. Resolve Host & Voice
        let host = self.hosts.as_ref().and_then(|h| {
            h.iter().find(|host| host.categories.iter().any(|c| c == category))
        });
        let host_name = host.map(|h| h.name.clone()).unwrap_or("主播".to_string());
        let host_voice = host.map(|h| h.voice.clone());

        // 2. Resolve Holiday Context
        let holiday_context = self.get_holiday_context();

        // Initialize Tracer
        let logger = Arc::new(Mutex::new(TraceLogger::new(category)));
        logger.lock().await.log("Start", &format!("Producing Episode for [{}]. Regen: {}", category, is_regen));

        // SMART FLOW (Unified)
        // New: Structure Planning → Compress → Generate by Groups → Extract Title
        let (raw_script, generated_title) = if let Some(item_list) = items {
            log::info!("Starting Smart Episode Generation for {} items...", item_list.len());
            
            // Step A: Intelligent Structure Planning (Sort + Group)
            let tracer_clone = logger.clone();
            let groups = match self.plan_episode_structure(item_list, tracer_clone).await {
                Ok(g) => {
                    log::info!("Smart Flow: Planned {} groups", g.len());
                    g
                },
                Err(e) => {
                    logger.lock().await.log("Planning Failed", &format!("Error: {}. Fallback to simple grouping.", e));
                    log::warn!("Smart Flow Planning failed: {}, falling back.", e);
                    // Fallback: chunks of 4
                    let ids: Vec<usize> = item_list.iter().map(|i| i.id).collect();
                    ids.chunks(4).map(|c| c.to_vec()).collect()
                }
            };

            // Step A.5: Compress long summaries
            let mut all_items: Vec<BroadcastItem> = item_list.to_vec();
            if let Err(e) = self.compress_summaries(&mut all_items, 180).await {
                log::warn!("Summary compression failed: {}", e);
            }

            // Step B: Generate segments by groups
            let mut segments = Vec::new();
            let mut prev_context = String::new();
            let total_groups = groups.len();
            
            for (group_idx, group_ids) in groups.iter().enumerate() {
                let is_first = group_idx == 0;
                let is_last = group_idx == total_groups - 1;
                
                // Get items for this group
                let group_items: Vec<BroadcastItem> = group_ids.iter()
                    .filter_map(|id| all_items.iter().find(|i| i.id == *id).cloned())
                    .collect();
                
                if group_items.is_empty() {
                    continue;
                }
                
                let segment = self.generate_segment_for_group(
                    category,
                    &host_name,
                    &group_items,
                    &prev_context,
                    &holiday_context,
                    is_first,
                    is_last,
                    logger.clone()
                ).await?;
                
                // Update context for next segment (last sentence)
                prev_context = Self::extract_last_sentence(&segment);
                segments.push(segment);
            }

            log::info!("Smart Flow: Generated {} segments", segments.len());
            let script = segments.join("\n\n");
            
            // Step C: Extract title from content
            let title = match self.extract_episode_title(item_list, category).await {
                Ok(t) => Some(t),
                Err(e) => {
                    log::warn!("Title extraction failed: {}", e);
                    None
                }
            };
            
            (script, title)

        } else {
            // No Items (Regen or legacy fallback): Use simple prompt
             let prompt = self.build_prompt(category, &host_name, context, &holiday_context, is_regen);
             let response = self.llm.chat(&prompt, is_regen).await?;
             logger.lock().await.log_llm("Simple Generation", "Used legacy/regen one-shot prompt", &prompt, &response);
             (response, None) // No title extraction for legacy mode
        };
        
        // 3. (Processed above)
        
        // 4. Check for SKIP
        if raw_script.trim().contains("SKIP") || raw_script.trim().len() < 10 {
            logger.lock().await.log("Result", "LLM indicated SKIP or empty script.");
            log::info!("LLM indicated SKIP or empty script.");
            // Try saving trace even on skip
            let _ = logger.lock().await.save();
            return Ok((String::new(), None, None, 0, true));
        }

        // 5. Proofreading (Integrated into Editor Loop)
        let final_full_text = raw_script;

        // Parse TITLE if present (for legacy mode compatibility)
        let mut final_title = generated_title;
        let mut script_body = final_full_text.clone();
        if final_full_text.starts_with("TITLE:") {
            if let Some(newline_idx) = final_full_text.find('\n') {
                let title_line = &final_full_text[..newline_idx];
                final_title = Some(title_line.trim_start_matches("TITLE:").trim().to_string());
                script_body = final_full_text[newline_idx+1..].trim().to_string();
            }
        }

        // 6. TTS Generation
        let tts_text = clean_for_tts(&script_body);
        let wav_audio_bytes = if let Some(voice) = host_voice {
            self.tts.speak_with_voice(&tts_text, &voice).await?
        } else {
            self.tts.speak(&tts_text).await?
        };

        // 7. Calculate Duration (Use WAV for accuracy) & Convert to MP3
        let mut duration = 0;
        let final_audio = if !wav_audio_bytes.is_empty() {
             let cursor = std::io::Cursor::new(&wav_audio_bytes);
             if let Ok(reader) = hound::WavReader::new(cursor) {
                 duration = (reader.duration() as f64 / reader.spec().sample_rate as f64) as i64;
             }
             
             // CONVERT TO MP3
             logger.lock().await.log("Audio Processing", &format!("WAV generated ({}s). Converting to MP3 (128k)...", duration));
             match self.tts.convert_to_mp3(&wav_audio_bytes) {
                 Ok(mp3_bytes) => Some(mp3_bytes),
                 Err(e) => {
                     logger.lock().await.log("Audio Processing Error", &format!("MP3 Conversion failed: {}", e));
                     log::error!("MP3 Conversion failed: {}. Falling back to WAV.", e);
                     Some(wav_audio_bytes)
                 }
             }
        } else {
            None
        };

        if let Err(e) = logger.lock().await.save() {
            log::error!("Failed to save execution trace: {}", e);
        }

        Ok((script_body, final_title, final_audio, duration, false))
    }

    fn get_holiday_context(&self) -> String {
        let now = chrono::Local::now();
        let today_solar = now.format("%m-%d").to_string();
        let lunar = LunisolarDate::from_date(now).unwrap();
        let l_month = lunar.to_lunar_month().to_u8();
        let l_day = lunar.to_lunar_day().to_u8();

        let mut greeting = String::new();
        match today_solar.as_str() {
            "01-01" => greeting.push_str("今天是元旦节，"),
            "05-01" => greeting.push_str("今天是劳动节，"),
            "10-01" => greeting.push_str("今天是国庆节，"),
            _ => {}
        }
        if l_month == 1 && l_day == 1 { greeting.push_str("今天是农历正月初一，春节快乐！"); }
        if l_month == 1 && l_day == 15 { greeting.push_str("今天是元宵节，"); }
        if l_month == 5 && l_day == 5 { greeting.push_str("今天是端午节，"); }
        if l_month == 8 && l_day == 15 { greeting.push_str("今天是中秋节，"); }

        if !greeting.is_empty() {
            format!("特别提示：{} 请在开场问候中自然融入节日祝福。", greeting)
        } else {
            String::new()
        }
    }

    fn build_prompt(&self, category: &str, host: &str, context: &str, holiday: &str, is_regen: bool) -> String {
        let now = chrono::Local::now();
        let time_info = now.format("%Y年%-m月%-d日 %H点").to_string();
                let regen_instruction = if is_regen {
            "注意：这是一个【重新生成】请求。请专注于改进提供的具体新闻故事，保留所有有意义的细节。"
        } else {
            "注意：这是一组新闻摘要。请将它们整合成一份连贯的【新闻简报】。如果有多条不同的新闻，请逐一播报，使用过渡语连接（如“另外”，“接着来看”）。不要合并无关的事实。"
        };

        // Dynamic Prompt Construction
        format!(
            "角色：新闻主播（{}）。任务：撰写广播稿。\n\
            时间：{}\n\
            节日背景：{}\n\
            分类：{}\n\
            {}\n\
            \n\
            核心规则：\n\
            1. **结构**：标准开场 -> 逐条播报新闻 -> 标准结束语。\n\
            2. **标准开场**：必须自然融入“欢迎收听 FreshLoop [{}频道]”以及“我是主播{}”。\n\
            3. **标准结束语**：必须包含“我是{}，为您在 FreshLoop 播报”或类似强化品牌和主播身份的自然结语。\n\
            4. **细节保留**：必须保留来源中的具体**人名**、**数字**、**日期**、**地点**和**引语**。\n\
            5. **连贯性**：不同新闻之间要用自然的过渡语。\n\
            6. **风格**：专业、客观，但具有会话感。\n\
            7. **禁止幻觉**：不要编造未提供的信息。\n\
            8. **格式**：纯文本，不要使用Markdown。\n\
            9. **标题要求**：请在输出的第一行生成一个标准标题，格式严格为：`TITLE: 内容关键词`。\n\
               - **禁止通用词**：绝对禁止使用“新闻简报”、“今日要闻”、“综合报道”等无意义标题。\n\
               - **必须具体**：标题必须直接反映新闻的核心事件或关键词。\n\
               - **错例**：`TITLE: 今日科技新闻汇总` (错误！)\n\
               - **正例**：`TITLE: Apple 营收创新高 / Windows 发布新版` (正确！)\n\
               - 如果是多条新闻，使用 `TITLE: 重点1 / 重点2`。\n\
               - 第二行开始正文。\n\
            \n\
            原始素材：\n{}\n\
            \n\
            现在输出完整的广播稿（第一行必须是TITLE）。",
            host, time_info, holiday, category, regen_instruction, category, host, host, context
        )
    }

    // --- New Helper Functions ---

    /// Extract the last sentence from text for context passing
    fn extract_last_sentence(text: &str) -> String {
        text.rsplit(|c| c == '。' || c == '！' || c == '？')
            .filter(|s| !s.trim().is_empty())
            .next()
            .unwrap_or("")
            .trim()
            .chars()
            .take(100)
            .collect()
    }

    /// Compress long summaries before segmentation
    async fn compress_summaries(&self, items: &mut Vec<BroadcastItem>, max_chars: usize) -> Result<()> {
        for item in items.iter_mut() {
            let char_count = item.summary.chars().count();
            if char_count > max_chars {
                log::info!("Compressing summary ({} chars): {}", char_count, item.title);
                
                let prompt = format!(
                    "精简以下新闻摘要至{}字以内，必须保留人名、公司名、数字、日期：\n\n{}\n\n仅输出精简后摘要：",
                    max_chars, item.summary
                );
                
                let compressed = self.llm.chat(&prompt, false).await?;
                item.summary = compressed.trim().to_string();
            }
        }
        Ok(())
    }

    /// Extract episode title from news content
    async fn extract_episode_title(&self, items: &[BroadcastItem], category: &str) -> Result<String> {
        let top_titles: String = items.iter().take(5)
            .map(|i| format!("- {}", i.title))
            .collect::<Vec<_>>()
            .join("\n");
        
        let prompt = format!(
            "从以下新闻中提取2-3个最重要的关键事件，生成简洁标题。\n\n\
            频道：{}\n今日新闻：\n{}\n\n\
            要求：\n\
            1. 不超过25字，多事件用「/」分隔\n\
            2. 禁止「今日要闻」「新闻汇总」等通用词\n\
            3. 禁止「震惊」「重磅」等夸张词\n\n\
            请输出 JSON 格式：{{ \"title\": \"你的标题\" }}",
            category, top_titles
        );
        
        // Use standard chat which handles <think> stripping internally,
        // but we also need to parse JSON to contain the result cleanly.
        let response = self.llm.chat(&prompt, false).await?;
        
        // Try to parse JSON
        let json_clean = response.trim().trim_matches('`').trim();
        // Handle code blocks like ```json ... ```
        let json_str = if let Some(start) = json_clean.find('{') {
             if let Some(end) = json_clean.rfind('}') {
                if start <= end {
                    &json_clean[start..=end]
                } else {
                    json_clean
                }
             } else {
                 json_clean
             }
        } else {
            json_clean
        };

        if let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str) {
            if let Some(t) = val["title"].as_str() {
                return Ok(t.trim().to_string());
            }
        }

        // Fallback: If not JSON, use the raw response but truncate widely if it looks like a thought dump
        let raw = response.trim();
        // Heuristic: If it contains newlines or is too long, it might be a thought dump.
        if raw.len() > 100 || raw.contains('\n') {
            // Last resort: Try to find a line that looks like a title or just take the fast 25 chars?
            // Safer to just log error and return a safe fallback title.
            log::warn!("LLM returned unstructured thought dump for title: {}", raw.chars().take(50).collect::<String>());
            return Ok(format!("{} News Briefing", category));
        }

        Ok(raw.to_string())
    }

    /// Generate a single segment for a group of items (with improved prompts)
    async fn generate_segment_for_group(
        &self,
        category: &str,
        host_name: &str,
        items: &[BroadcastItem],
        prev_context: &str,
        holiday_context: &str,
        is_first: bool,
        is_last: bool,
        logger: Arc<Mutex<TraceLogger>>
    ) -> Result<String> {
        // Build content block
        let content_block: String = items.iter().map(|item| {
            format!("- 《{}》\n  摘要: {}\n  来源: {}", item.title, item.summary, item.source_name)
        }).collect::<Vec<_>>().join("\n\n");

        // Fixed templates for opening and closing
        let now = chrono::Local::now();
        let date_str = now.format("%Y年%-m月%-d日").to_string();
        let weekdays = ["一", "二", "三", "四", "五", "六", "日"];
        let weekday = weekdays[now.weekday().num_days_from_monday() as usize];
        
        let opening_template = format!(
            "大家好，欢迎收听 FreshLoop {}频道，我是{}。今天是{}，星期{}。{}",
            category, host_name, date_str, weekday,
            if !holiday_context.is_empty() { format!(" {}", holiday_context) } else { String::new() }
        );
        
        let closing_template = format!(
            "以上就是本期 FreshLoop {}频道的全部内容。感谢收听，我是{}，下期再见。",
            category, host_name
        );

        // Dynamic instructions
        let instruction = if is_first {
            format!(
                "这是节目的开场部分。请使用以下固定开场语开始：\n「{}」\n然后自然引入第一条新闻。\n⚠️ 禁止编造天气或其他未提供的信息。",
                opening_template
            )
        } else {
            "这是节目的中间部分。请自然承接上一段，继续播报本段新闻。".to_string()
        };

        let closing_instruction = if is_last {
            format!("播报完最后一条新闻后，使用以下固定结束语：\n「{}」", closing_template)
        } else {
            "用一句简短过渡语引入下一段（如「接下来还有更多资讯」）。禁止制造悬念或过度渲染。".to_string()
        };

        let mut prompt = format!(
            "Role: FreshLoop 新闻主播。\n\
            频道: {}\n\
            人设: {} (专业、客观、亲和。以清晰准确传递信息为首要目标)。\n\
            \n\
            【当前任务】\n\
            接住上文（\"{}\"），播报本段新闻。\n\
            1. {}\n\
            2. {}\n\
            \n\
            【新闻素材】\n\
            {}\n\
            \n\
            【核心要求】：\n\
            1. **信息第一**：完整保留每条新闻的5W1H（人名、数字、日期、地点）。禁止模糊化。\n\
            2. **客观传递**：仅陈述事实，禁止添加主观推测。评论/感叹每段最多1处。\n\
            3. **简洁过渡**：新闻之间用一句话过渡（如「另一边」「说到XX」）。\n\
            4. **禁止重复**：上一段已播内容不得复述。\n\
            5. **弹性字数**：以听众理解为准。快讯可简短，复杂报道需充分展开。\n\
            6. **校对**：绝不允许错别字、语病。\n\
            7. **格式**：直接输出口播稿，不要Markdown。\n\
            8. **禁止念出来源**：严禁在文稿中包含“(来源：XXX)”或“(Source: XXX)”这样的括号标注。如果必须提及，请融入句子中（如“据XXX报道”）。",
            category, host_name, prev_context, instruction, closing_instruction, content_block
        );

        // Writer-Editor Loop
        let mut attempts = 0;
        let mut current_script;
        const MAX_RETRIES: usize = 3;

        loop {
            attempts += 1;
            logger.lock().await.log("Segment Gen", &format!("Generating group of {} items, attempt {}", items.len(), attempts));
            
            current_script = self.llm.chat(&prompt, false).await?;
            logger.lock().await.log_llm("Segment Result", &format!("Group generation attempt {}", attempts), &prompt, &current_script);

            // Safety check
            if current_script.len() > 5000 {
                log::warn!("Script too long ({} chars). Rejecting.", current_script.len());
                if attempts < MAX_RETRIES {
                    prompt.push_str("\n\n【警告】上一版过长。请精简，只播报本段新闻。");
                    continue;
                } else {
                    current_script.truncate(5000);
                }
            }

            // Editor review
            let (passed, critique) = self.review_segment(&current_script, prev_context, logger.clone()).await?;
            
            if passed {
                break;
            }
            
            if attempts >= MAX_RETRIES {
                log::warn!("Editor rejected 3 times. Forcing acceptance. Critique: {}", critique);
                break;
            }

            log::info!("Editor rejected (Attempt {}): {}. Regenerating...", attempts, critique);
            prompt.push_str(&format!("\n\n【主编反馈】{}。请修正后重新撰写。", critique));
        }

        Ok(current_script)
    }

    // Proofread function removed (merged into editor loop)


    /// Step 3: Recursive Segment Generation
    async fn generate_segment(
        &self, 
        category: &str, 
        host_name: &str,
        items: &[BroadcastItem], 
        start_idx: usize, 
        batch_size: usize,
        prev_context: &str,
        holiday_context: &str,
        is_first: bool,
        logger: Arc<Mutex<TraceLogger>>
    ) -> Result<Vec<String>> {
        if start_idx >= items.len() {
            return Ok(Vec::new());
        }

        let end_idx = std::cmp::min(start_idx + batch_size, items.len());
        let current_batch = &items[start_idx..end_idx];
        let is_last = end_idx == items.len();

        // Build prompt for this segment
        let content_block: String = current_batch.iter().map(|item| {
            format!("- {}\n  摘要: {}\n  来源: {}", item.title, item.summary, item.source_name)
        }).collect::<Vec<_>>().join("\n\n");

        let instruction = if is_first {
            "这是节目的开场部分。请以热情、专业的语调做开场白（包含问候、日期、节日/天气等），然后自然引入第一条新闻。"
        } else {
            "这是节目的中间部分。请自然地承接上一段内容（不要生硬的'接下来'），继续播报本段新闻。"
        };

        let closing_instruction = if is_last {
            "播报完所有新闻后，请进行总结并做结束语（感谢收听 FreshLoop，我是[主持人]，下次见）。"
        } else {
            "播报完本段新闻后，做一个自然的过渡，准备引入下一段内容（但不要具体说下一段是什么，只做悬念或平滑过渡）。"
        };

        let mut prompt = format!(
            "Role: Host of 'FreshLoop' (顶流播客主持人)。\n\
            频道: {}\n\
            人设: {} (幽默、犀利或温暖，视内容而定。拒绝播音腔，要像真人在对话)。\n\
            节日: {}\n\
            \n\
            【当前任务】\n\
            接住上文语音流（\"{}\"），播报本段新闻。\n\
            1. {}\n\
            2. {}\n\
            \n\
            【新闻素材】\n\
            {}\n\
            \n\
            【核心要求 - 必须同时完成】：\n\
            1. **交流感**：使用第二人称（你），多用反问句、感叹句。用“signposting”技巧引导听众（如“这事儿有点意思...”）。\n\
            2. **逻辑串联**：严禁呆板的“首先、其次”。要用内在逻辑（因果、对比、层递）把新闻串起来。\n\
            3. **校对（关键）**：输出必须是【终稿】。生成时请自我检查，**绝不允许出现错别字、语病或翻译腔**。\n\
            4. **格式**：直接输出口播稿。不要Markdown。\n\
            5. **禁止念出来源**：严禁包含“(来源：XXX)”等标注。",
            category, host_name, holiday_context, prev_context, instruction, closing_instruction, content_block
        );

        // Writer-Editor Loop
        let mut attempts = 0;
        let mut current_script;
        const MAX_WRITER_RETRIES: usize = 3;

        loop {
            attempts += 1;
            // 1. Writer generates
            attempts += 1;
            // 1. Writer generates
            logger.lock().await.log("Segment Gen", &format!("Generating batch {}-{}, attempt {} (Prompt len: {})", start_idx, end_idx, attempts, prompt.len()));
            
            current_script = self.llm.chat(&prompt, false).await?;
            
            logger.lock().await.log_llm("Segment Writer Result", &format!("Segment {} Batch", start_idx), &prompt, &current_script);

            // SAFETY CHECK: Hallucination/Repetition Guard
            if current_script.len() > 5000 {
                log::warn!("Generated script too long ({} chars). Likely repetition loop. Rejecting.", current_script.len());
                if attempts < MAX_WRITER_RETRIES {
                    prompt.push_str("\n\n【系统警告】上一版生成的内容过长（可能陷入了重复）。请务必精简，只播报约定的几条新闻，不要重复。");
                    continue;
                } else {
                    // Force truncate if max retries reached to avoid crashing next steps
                    current_script.truncate(5000);
                    current_script.push_str("\n(Truncated due to excessive length)");
                }
            }

            // 2. Editor reviews (Skip for first segment as it has no prev context to mis-match, 
            // though we could still check internal flow. Let's check all.)
            // 2. Editor reviews (Skip for first segment as it has no prev context to mis-match, 
            // though we could still check internal flow. Let's check all.)
            let (passed, critique) = self.review_segment(&current_script, prev_context, logger.clone()).await?;
            
            if passed {
                break;
            }
            
            if attempts >= MAX_WRITER_RETRIES {
                log::warn!("Editor rejected segment 3 times. Forcing acceptance. Critique: {}", critique);
                break;
            }

            log::info!("Editor rejected segment (Attempt {}): {}. Regenerating...", attempts, critique);
            prompt.push_str(&format!("\n\n【主编反馈】\n你的上一版草稿被打回了，原因：{}\n请根据意见重新撰写。", critique));
        }

        // Proofreading merged into generation loop
        let refined_script = current_script;
        
        let mut segments = vec![refined_script.clone()];
        
        // Context for next segment (last 200 chars)
        // Context for next segment (last 200 chars, UTF-8 safe)
        let next_context_val = if refined_script.chars().count() > 200 {
            refined_script.chars().rev().take(200).collect::<String>().chars().rev().collect::<String>()
        } else {
            refined_script.clone()
        };
        let next_context = next_context_val.as_str();

        // Recursive call
        if !is_last {
            let mut next_segments = Box::pin(self.generate_segment(
                category, host_name, items, end_idx, batch_size, next_context, holiday_context, false, logger
            )).await?;
            segments.append(&mut next_segments);
        }

        Ok(segments)
    }

    /// Helper: Editor Review (Enhanced)
    async fn review_segment(&self, script: &str, prev_context: &str, logger: Arc<Mutex<TraceLogger>>) -> Result<(bool, String)> {
        if script.trim().len() < 10 {
            return Ok((false, "Content too short".to_string()));
        }

        let prompt = format!(
            "Role: 资深音频制作人 & 校对员。任务：质量控制。\n\n\
            【上文】\"{}\"\n\
            【稿件】\"{}\"\n\n\
            审核标准（违反任一则 Fail）：\n\
            1. **Flow**：读起来是否顺畅？衔接是否自然？\n\
            2. **Persona**：是否像专业新闻主播？有没有AI味？\n\
            3. **Proofreading**：CRITICAL - 任何错别字、病句、多字漏字必须打回！\n\
            4. **信息完整性**：每条新闻的核心数字、人名是否保留？被省略则打回！\n\
            5. **主观性**：每段超过1处主观评论/感叹应打回！\n\
            6. **重复检查**：是否复述了上一段已播内容？如有则打回！\n\
            7. **实体一致性**：人名/公司名与素材一致？（如「英伟锐」应为「英伟达」则打回）\n\
            8. **过渡质量**：段落过渡是否自然流畅？生硬则打回！\n\n\
            输出格式（JSON）：\n\
            {{ \"pass\": true, \"critique\": \"通过\" }}\n\
            或\n\
            {{ \"pass\": false, \"critique\": \"具体问题描述...\" }}",
            prev_context, script
        );

        let response = self.llm.chat(&prompt, false).await?;
        logger.lock().await.log_llm("Segment Review", "Critique Logic invoked", &prompt, &response);
        
        // Simple JSON parse
        let json_str = response.trim().trim_matches('`').trim();
        let start = json_str.find('{').unwrap_or(0);
        let end = json_str.rfind('}').unwrap_or(json_str.len().saturating_sub(1));
        
        let json_valid = if start <= end && end < json_str.len() {
             &json_str[start..=end]
        } else {
             "{}"
        };
        
        #[derive(serde::Deserialize)]
        struct Review {
            pass: bool,
            critique: String,
        }

        let review: Review = serde_json::from_str(json_valid)
            .unwrap_or(Review { pass: true, critique: "JSON Parse Error, assuming pass".to_string() });
            
        Ok((review.pass, review.critique))
    }

    /// Step 1: LLM-driven Intelligent Structure Planning
    /// Simultaneously sorts AND groups items based on content length and nature
    async fn plan_episode_structure(&self, items: &[BroadcastItem], logger: Arc<Mutex<TraceLogger>>) -> Result<Vec<Vec<usize>>> {
        // Show full content info (title + summary length + preview) so LLM can make informed decisions
        let item_list: String = items.iter()
            .map(|item| {
                let summary_preview: String = item.summary.chars().take(80).collect();
                format!(
                    "ID {}: 《{}》\n   摘要长度: {} 字\n   预览: {}...",
                    item.id, item.title, 
                    item.summary.chars().count(),
                    summary_preview
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");
            
        let mut prompt = format!(
            "Role: 节目策划 (Showrunner)。\n\n\
            任务：为这期播客编排结构，同时完成【排序】和【分组】。\n\n\
            【待编排新闻】\n{}\n\n\
            【编排原则】\n\
            1. **黄金开头**：最重磅的新闻放第一组\n\
            2. **主题聚合**：相关话题放同一组\n\
            3. **动态分组**：\n\
               - 快讯/短消息（摘要≤80字）：可以4-5条一组\n\
               - 普通新闻（摘要80-200字）：2-4条一组\n\
               - 深度报道/长内容（摘要>200字）：1-2条一组，甚至单独成组\n\
            4. **节奏感**：硬新闻和软新闻交替\n\
            5. **Kicker**：最后一组放有趣/轻松的内容\n\n\
            【输出格式】仅输出JSON二维数组，表示分组后的ID：\n\
            [[3,1,4], [2], [5,6,7]]\n\
            （表示：第一组播3,1,4号；第二组单独播2号深度内容；第三组播5,6,7号）",
            item_list
        );

        let input_ids: std::collections::HashSet<usize> = items.iter().map(|i| i.id).collect();
        let mut attempts = 0;
        const MAX_RETRIES: usize = 3;

        loop {
            attempts += 1;
            log::info!("Planning structure attempt {}/{}", attempts, MAX_RETRIES);
            
            let response = self.llm.chat(&prompt, false).await?;
            
            logger.lock().await.log_llm("Planning Structure", &format!("Grouping Attempt {}", attempts), &prompt, &response);
            
            // Parse JSON 2D array [[...], [...]]
            let json_clean = response.trim().trim_matches('`').trim();
            let start = json_clean.find('[').unwrap_or(0);
            let end = json_clean.rfind(']').unwrap_or(json_clean.len().saturating_sub(1));
            
            let parse_result: Result<Vec<Vec<usize>>, anyhow::Error> = if start <= end && end < json_clean.len() {
                serde_json::from_str(&json_clean[start..=end]).map_err(|e| e.into())
            } else {
                Err(anyhow::anyhow!("Invalid JSON range"))
            };
            
            match parse_result {
                Ok(groups) => {
                    // VALIDATION: Check all IDs are present exactly once
                    let output_ids: Vec<usize> = groups.iter().flatten().cloned().collect();
                    let output_set: std::collections::HashSet<usize> = output_ids.iter().cloned().collect();
                    
                    if output_ids.len() != items.len() {
                        log::warn!("Invalid structure: count mismatch (expected {}, got {})", items.len(), output_ids.len());
                    } else if output_set != input_ids {
                        let missing: Vec<_> = input_ids.difference(&output_set).collect();
                        let extra: Vec<_> = output_set.difference(&input_ids).collect();
                        log::warn!("Invalid structure: IDs mismatch (missing: {:?}, extra: {:?})", missing, extra);
                    } else if groups.iter().any(|g| g.is_empty()) {
                        log::warn!("Invalid structure: contains empty groups");
                    } else {
                        // All good
                        log::info!("Structure planned: {} groups, sizes: {:?}", groups.len(), groups.iter().map(|g| g.len()).collect::<Vec<_>>());
                        return Ok(groups);
                    }
                },
                Err(e) => {
                    log::warn!("Failed to parse structure JSON: {}", e);
                }
            }
            
            if attempts >= MAX_RETRIES {
                log::error!("Max retries reached for structure planning. Falling back to simple grouping.");
                break;
            }
            
            // Add hint for next attempt
            prompt.push_str("\n\n警告：上一次输出的格式或ID有误。请确保：\n1. 输出JSON二维数组格式 [[...], [...]]\n2. 包含所有且仅包含待编排新闻的ID\n3. 每个ID只出现一次\n4. 不要有空组");
        }
        
        // Fallback: Simple grouping with batch_size 4
        let mut groups = Vec::new();
        let ids: Vec<usize> = items.iter().map(|i| i.id).collect();
        for chunk in ids.chunks(4) {
            groups.push(chunk.to_vec());
        }
        Ok(groups)
    }

}

#[derive(Debug, Clone, serde::Serialize)]
struct BroadcastItem {
    id: usize, // Original index
    title: String,
    summary: String,
    source_name: String,
    original_url: String,
    is_update: bool,
}

fn clean_for_tts(input: &str) -> String {
    let mut cleaned = input.to_string();
    cleaned = cleaned.replace("**", "").replace("*", "").replace("#", "");
    let re_link = Regex::new(r"\[.*?\]\(.*?\)").unwrap();
    cleaned = re_link.replace_all(&cleaned, "").to_string();

    // Remove source citations in parentheses
    let re_source_en = Regex::new(r"(?i)[（(]\s*source[:：]\s*.*?[）)]").unwrap();
    let re_source_cn = Regex::new(r"(?i)[（(]\s*来源[:：]\s*.*?[）)]").unwrap();
    cleaned = re_source_en.replace_all(&cleaned, "").to_string();
    cleaned = re_source_cn.replace_all(&cleaned, "").to_string();

    cleaned
}
