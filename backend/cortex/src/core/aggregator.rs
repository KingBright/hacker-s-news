use crate::core::config::Host;

use crate::core::llm::LlmClient;
use crate::core::news_buffer::{ClusterData, NewsBuffer, PendingNewsItem};
use crate::core::nexus::{ItemPayload, NexusClient};
use crate::core::topic_registry::TopicRegistry;
use crate::core::tts::TtsClient;
use anyhow::Result;
use chrono::{Datelike, NaiveDate, Weekday};
use std::io::Write;
use std::sync::Arc;
use tokio::sync::Mutex;

const TTS_DRAFT_CACHE_VERSION: &str = "episode-date-context-v3";

// --- Trace Logger ---
#[derive(Debug, serde::Serialize)]
struct TraceStep {
    timestamp: String,
    step_name: String,
    details: String,
    llm_prompt: Option<String>,
    llm_response: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EpisodeDateContext {
    prompt_block: String,
    run_of_show_line: String,
}

fn build_episode_date_context(date: NaiveDate) -> EpisodeDateContext {
    let date_label = format!(
        "{}年{}月{}日，{}",
        date.year(),
        date.month(),
        date.day(),
        weekday_label(date.weekday())
    );

    let official_calendar_known =
        crate::core::holiday_calendar::official_calendar_known_for_year(date.year());
    let (status, guidance) = if let Some(holiday) =
        crate::core::holiday_calendar::lookup_holiday(date)
    {
        let certainty_note = match holiday.certainty {
            crate::core::holiday_calendar::HolidayCertainty::Official => "官方日历",
            crate::core::holiday_calendar::HolidayCertainty::Projected => "规则推算",
        };
        (
            format!("今天属于{}", holiday.name),
            format!(
                "开场可以自然融入{}的时间感（{}），但不要把今天说成工作日，也不要编造素材外的出行、天气或活动安排",
                holiday.name, certainty_note
            ),
        )
    } else if let Some(name) = crate::core::holiday_calendar::lookup_adjusted_workday(date) {
        (
            format!("今天是{}", name),
            "如需提及，只能说调休工作日；不要说成普通周末，也不要硬套休息日语气".to_string(),
        )
    } else {
        let festival_names = crate::core::holiday_calendar::traditional_festival_names(date);
        if !festival_names.is_empty() {
            let names = festival_names.join("、");
            (
                format!("今天是{}", names),
                format!(
                    "开场可以轻轻带到{}，但不要把它扩写成未提供的公共假期安排",
                    names
                ),
            )
        } else if matches!(date.weekday(), Weekday::Sat | Weekday::Sun) {
            if official_calendar_known {
                (
                    "今天是周末".to_string(),
                    "开场可以更从容，但不要说今天是工作日，也不要硬套通勤早高峰".to_string(),
                )
            } else {
                (
                    "今天按自然星期是周末，官方调休日历尚未内置".to_string(),
                    "可以只报日期或轻轻带过周末感；不要说今天是工作日，也不要声称这是官方休息日"
                        .to_string(),
                )
            }
        } else {
            (
                "今天没有已知节假日背景".to_string(),
                "不需要强行说工作日；可以只报日期，或直接进入新闻".to_string(),
            )
        }
    };

    let prompt_block = format!("日期语境：{}；{}。{}", date_label, status, guidance);
    let run_of_show_line = format!("{}；{}", date_label, status);

    EpisodeDateContext {
        prompt_block,
        run_of_show_line,
    }
}

fn weekday_label(weekday: Weekday) -> &'static str {
    match weekday {
        Weekday::Mon => "周一",
        Weekday::Tue => "周二",
        Weekday::Wed => "周三",
        Weekday::Thu => "周四",
        Weekday::Fri => "周五",
        Weekday::Sat => "周六",
        Weekday::Sun => "周日",
    }
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
        log::info!(
            "[Trace: {}] {}: {}",
            self.step_name_slug(),
            step_name,
            details
        );
    }

    pub fn log_llm(&mut self, step_name: &str, details: &str, prompt: &str, response: &str) {
        self.steps.push(TraceStep {
            timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
            step_name: step_name.to_string(),
            details: details.to_string(),
            llm_prompt: Some(prompt.to_string()),
            llm_response: Some(response.to_string()),
        });
        log::info!(
            "[Trace: {}] {} (LLM Invoked)",
            self.step_name_slug(),
            step_name
        );
    }

    fn step_name_slug(&self) -> String {
        self.category.chars().take(4).collect()
    }

    pub fn save(&self) -> Result<String> {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let log_dir = std::path::Path::new(&home).join(".freshloop/logs/traces");
        std::fs::create_dir_all(&log_dir)?;

        let filename = format!(
            "trace_{}_{}_{}.md",
            self.start_time.format("%Y%m%d_%H%M"),
            self.category.replace(" ", "_"),
            self.id.chars().take(8).collect::<String>()
        );

        let path = log_dir.join(&filename);
        let mut file = std::fs::File::create(&path)?;

        writeln!(file, "# Execution Trace Report")?;
        writeln!(file, "- **Task ID**: {}", self.id)?;
        writeln!(file, "- **Category**: {}", self.category)?;
        writeln!(file, "- **Start Time**: {}", self.start_time)?;
        writeln!(file, "- **Total Steps**: {}\n", self.steps.len())?;

        for (i, step) in self.steps.iter().enumerate() {
            writeln!(
                file,
                "## {}. {} ({})",
                i + 1,
                step.step_name,
                step.timestamp
            )?;
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
        Self {
            buffer,
            registry,
            llm,
            tts,
            nexus,
            hosts,
        }
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

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let mut categories_to_flush = Vec::new();

        for (category, (count, oldest_ts)) in stats {
            let wait_time = if now > oldest_ts { now - oldest_ts } else { 0 };

            // Flush Rule: Unique Clusters >= 10 OR Wait > 6h
            if count >= MIN_CLUSTERS || wait_time > MAX_WAIT_SEC {
                log::info!(
                    "Triggering Flush for [{}]: Clusters={}, Wait={}s",
                    category,
                    count,
                    wait_time
                );
                categories_to_flush.push(category);
            }
        }

        let mut last_error: Option<anyhow::Error> = None;

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
                log::info!(
                    "Postponing [{}]: Only {} clusters, need at least {}",
                    cat,
                    clusters.len(),
                    MIN_CLUSTERS_FOR_EPISODE
                );
                continue;
            }

            // Collect IDs for potential removal
            let cluster_ids: Vec<String> = clusters.iter().map(|c| c.id.clone()).collect();

            // Process the category (Peek -> Process)
            match self.process_clusters(&cat, &clusters).await {
                Ok(true) => {
                    // Success (Ack): Remove processed clusters
                    log::info!(
                        "Successfully processed [{}], removing {} clusters.",
                        cat,
                        cluster_ids.len()
                    );
                    let buf = self.buffer.lock().await;
                    if let Err(e) = buf.remove_clusters(&cat, &cluster_ids) {
                        log::error!("Failed to remove clusters from DB after processing: {}", e);
                    }
                }
                Ok(false) => {
                    // Postponed/Skipped: Do nothing (Repo keeps data)
                    log::info!("Processing [{}]: Postponed or Skipped. Data retained.", cat);
                }
                Err(e) => {
                    // Failed: Log and continue to next category instead of aborting.
                    // This ensures one category's TTS failure doesn't block others.
                    log::error!(
                        "Failed to process category [{}]: {}. Data retained, will retry later.",
                        cat,
                        e
                    );
                    last_error = Some(e);
                }
            }
        }

        // If any category failed, return the last error for visibility
        if let Some(e) = last_error {
            return Err(e);
        }
        Ok(())
    }

    /// Admin Regeneration Loop (Moved from news.rs)
    pub async fn process_regenerations(&self) -> Result<()> {
        let pending = self.nexus.fetch_pending_jobs().await?;
        if pending.is_empty() {
            return Ok(());
        }

        log::info!("Found {} pending regeneration jobs.", pending.len());

        for job in pending {
            // Treat "Title - Smart Daily" as category for reverse lookup or fallback to "Other"
            let category = if job.title.contains(" - ") {
                job.title.split(" - ").next().unwrap_or("Other")
            } else {
                "Other"
            };

            let context = job.summary.as_deref().unwrap_or("");

            log::info!(
                "Regenerating [Item {}] (Category: {})",
                job.id.as_deref().unwrap_or("?"),
                category
            );

            // UNIFIED LOGIC: Use produce_episode
            let (final_script, _generated_title, audio_bytes, duration, skipped) = self
                .produce_episode(
                    category, context, None, // No items for regeneration
                    true, // is_regeneration
                )
                .await?;

            if skipped {
                log::warn!(
                    "Regeneration skipped for [Item {}], keeping pending for next cycle.",
                    job.id.as_deref().unwrap_or("?")
                );
                continue;
            }

            // Upload Audio if present (Manual upload for Regen flow).
            // If upload fails, do NOT complete the job to avoid publishing broken items.
            let audio_url = if let Some(bytes) = audio_bytes {
                let file_name = format!("regen_{}.mp3", uuid::Uuid::new_v4());
                match self.nexus.upload_audio(bytes, &file_name).await {
                    Ok(url) => url,
                    Err(e) => {
                        log::error!(
                            "Regeneration upload failed for [Item {}]: {}. Keeping job pending.",
                            job.id.as_deref().unwrap_or("?"),
                            e
                        );
                        continue;
                    }
                }
            } else {
                log::error!(
                    "Regeneration produced no audio for [Item {}]. Keeping job pending.",
                    job.id.as_deref().unwrap_or("?")
                );
                continue;
            };

            // Complete Job
            if let Some(id) = &job.id {
                if let Err(e) = self
                    .nexus
                    .complete_job(id, &audio_url, &final_script, Some(duration))
                    .await
                {
                    log::error!(
                        "Failed to complete regeneration job [Item {}]: {}. Keeping job pending.",
                        id,
                        e
                    );
                }
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
            self.registry
                .record_topic_with_details(&combined_text, &item.title, &summary)?;
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
            return Ok(true); // New cluster created
        }

        // 2. LLM verification for the most similar cluster
        let mut best_match: Option<ClusterData> = None;

        // Optimization: Fast path for exact title matches (Check Main + Related)
        let mut exact_match_found = false;
        if let Some(exact_match_cluster) = similar_clusters.iter().find(|c| {
            // Check main item
            if c.main_item
                .title
                .trim()
                .eq_ignore_ascii_case(item.title.trim())
            {
                return true;
            }
            // Check related items
            c.related_items
                .iter()
                .any(|r| r.title.trim().eq_ignore_ascii_case(item.title.trim()))
        }) {
            log::info!("Fast-track: Found exact title match for '{}'", item.title);
            best_match = Some(exact_match_cluster.clone());
            exact_match_found = true;
        } else {
            // Normal path: LLM verification
            for cluster in similar_clusters {
                let dist = ClusterData::hamming_distance(cluster.simhash, item_hash);
                log::info!(
                    "SimHash Candidate: '{}' (Dist: {}) vs New: '{}'",
                    cluster.main_item.title,
                    dist,
                    item.title
                );

                let is_same = self.llm_verify_same_topic(&item, &cluster).await?;
                if is_same {
                    best_match = Some(cluster);
                    break;
                }
            }
        }

        if let Some(mut matched_cluster) = best_match {
            // 3. Merge into existing cluster
            log::info!(
                "Merging '{}' into cluster '{}'",
                item.title,
                matched_cluster.main_item.title
            );

            // Optimization: If title is identical, skip LLM merge cost
            if exact_match_found
                || item
                    .title
                    .trim()
                    .eq_ignore_ascii_case(matched_cluster.main_item.title.trim())
            {
                // Strict Duplicate Check: Check against ALL items in cluster
                // If this new item is identical (Title+Content) to ANY existing item, discard it.
                let is_strict_duplicate = (item
                    .title
                    .trim()
                    .eq_ignore_ascii_case(matched_cluster.main_item.title.trim())
                    && item.description.trim() == matched_cluster.main_item.description.trim())
                    || matched_cluster.related_items.iter().any(|r| {
                        r.title.trim().eq_ignore_ascii_case(item.title.trim())
                            && r.description.trim() == item.description.trim()
                    });

                if is_strict_duplicate {
                    log::info!(
                        "Discarding exact duplicate item (Title+Content match in cluster): {}",
                        item.title
                    );
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
            return Ok(false); // Merged into existing
        }

        // No match confirmed by LLM, create new cluster
        log::info!("New cluster (LLM verified): {}", item.title);
        let cluster = ClusterData::new(item);
        let buf = self.buffer.lock().await;
        buf.store_cluster(&cluster)?;
        Ok(true)
    }

    /// LLM verification: Are these two items about the same topic?
    async fn llm_verify_same_topic(
        &self,
        item: &PendingNewsItem,
        cluster: &ClusterData,
    ) -> Result<bool> {
        let prompt = format!(
            "判断以下两条新闻是否报道同一个事件/话题？\n\n\
            新闻A:\n标题: {}\n摘要: {}\n\n\
            新闻B:\n标题: {}\n摘要: {}\n\n\
            判断标准：同一事件指同一个具体事件、产品、人物动态，而非仅仅领域相似。\n\
            仅回答 YES 或 NO。",
            item.title,
            item.description,
            cluster.main_item.title,
            cluster
                .merged_summary
                .as_ref()
                .unwrap_or(&cluster.main_item.description)
        );

        let response = self.llm.chat(&prompt, false).await?;
        let answer = response.trim().to_uppercase();
        Ok(answer.contains("YES"))
    }

    /// LLM merge: Combine item into cluster with merged summary
    async fn llm_merge_items(
        &self,
        cluster: &ClusterData,
        new_item: &PendingNewsItem,
    ) -> Result<Option<(String, String)>> {
        let existing_summary = cluster
            .merged_summary
            .as_ref()
            .unwrap_or(&cluster.main_item.description);

        let mut prompt = format!(
            "Role: Senior Intelligence Analyst (资深情报分析师)。\n\n任务：将以下多来源信息综合成一份权威的简报模块。\n\n【策略 - 请根据内容类型自适应】：\n- **硬新闻/财经**：准确性第一。保留所有具体数字、日期、人名、公司名。遵循 5W1H 原则。\n- **软新闻/观点**：捕捉核心论点、情感弧线或独特氛围。提炼\"金句\"。\n- **低质量/碎片化**：如果来源行文混乱，请将其重构为逻辑通顺、符合新闻标准的文稿。修复所有语法错误。\n\n已有内容:\n标题: {}\n摘要: {}\n\n新内容:\n标题: {}\n摘要: {}\n\n要求：\n1. 极高信息密度：拒绝废话。\n2. 输出综合标题和摘要。",
            cluster.main_item.title, existing_summary,
            new_item.title, new_item.description
        );

        #[derive(serde::Deserialize, schemars::JsonSchema)]
        struct MergeResult {
            title: String,
            summary: String,
        }

        let mut attempts = 0;
        const MAX_RETRIES: usize = 3;

        loop {
            attempts += 1;
            match self
                .llm
                .chat_json::<MergeResult>(&prompt, "merge_result", false)
                .await
            {
                Ok(result) => {
                    let title = if result.title.is_empty() {
                        cluster.main_item.title.clone()
                    } else {
                        result.title
                    };
                    let summary = if result.summary.is_empty() {
                        existing_summary.to_string()
                    } else {
                        result.summary
                    };

                    // Editor Review Loop for Summary
                    let (passed, critique) = self.review_summary(&title, &summary).await?;
                    if passed {
                        return Ok(Some((title, summary)));
                    }

                    if attempts >= MAX_RETRIES {
                        log::warn!(
                            "Editor rejected summary 3 times. Accepted last draft. Critique: {}",
                            critique
                        );
                        return Ok(Some((title, summary)));
                    }

                    log::info!(
                        "Editor rejected summary (Attempt {}): {}. Regenerating...",
                        attempts,
                        critique
                    );
                    prompt.push_str(&format!("\n\n【主编反馈】\n你的上一版摘要被打回了，原因：{}\n请保留更多细节，重新合并。", critique));
                    continue;
                }
                Err(e) => {
                    log::warn!("LLM merge failed (attempt {}): {}", attempts, e);
                    if attempts >= MAX_RETRIES {
                        break;
                    }
                }
            }
        }

        Ok(None)
    }

    /// Helper: Review merged summary quality
    async fn review_summary(&self, title: &str, summary: &str) -> Result<(bool, String)> {
        let prompt = format!(
            "Role: Executive Editor (执行主编)。\n\n任务：严格质检这条新闻摘要。\n\n【标题】{}\n【摘要】{}\n\n审核标准：\n1. **Hook (吸引力)**：第一句话是否足够吸引人？\n2. **Clarity (清晰度)**：没有任何语病、错别字或歧义。\n3. **Detail (细节)**：保留了关键数据和实体名称，没有被过度概括。\n4. **Correction (校对)**：必须充当校对员，如果发现任何错别字或语句不通，视为不合格！\n\n输出审核结果。",
            title, summary
        );

        #[derive(serde::Deserialize, schemars::JsonSchema)]
        struct ReviewResult {
            pass: bool,
            critique: String,
        }

        let review = self
            .llm
            .chat_json::<ReviewResult>(&prompt, "review_result", false)
            .await
            .unwrap_or(ReviewResult {
                pass: true,
                critique: "JSON Parse Error".to_string(),
            });

        Ok((review.pass, review.critique))
    }

    /// Check if a previously reported topic has substantial new information
    /// Returns Some(update_summary) if there's new info, None if it should be discarded
    async fn check_for_updates(
        &self,
        cluster: &ClusterData,
        current_summary: &str,
        prev_record: &crate::core::topic_registry::TopicRecord,
    ) -> Result<Option<String>> {
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

        #[derive(serde::Deserialize, schemars::JsonSchema)]
        struct UpdateCheck {
            has_update: bool,
            #[serde(default)]
            update_summary: Option<String>,
        }

        match self
            .llm
            .chat_json::<UpdateCheck>(&prompt, "update_check", false)
            .await
        {
            Ok(check) => {
                if check.has_update {
                    Ok(check.update_summary)
                } else {
                    Ok(None)
                }
            }
            Err(e) => {
                log::warn!("Update check LLM failed: {}", e);
                Ok(None)
            }
        }
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
        let mut topics_to_record = Vec::new();

        for (idx, cluster) in clusters.iter().enumerate() {
            let summary = cluster
                .merged_summary
                .as_ref()
                .unwrap_or(&cluster.main_item.description);
            let combined_text = format!("{} {}", cluster.main_item.title, summary);

            // --- HARD TIME FILTER (Mechanism 1) ---
            // If main item is older than 72 hours, discard it to prevent "Timeline Paradox"
            let now_ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            if cluster.main_item.timestamp < now_ts - 72 * 3600 {
                log::info!(
                    "Time Filter: Discarding stale cluster '{}' (Age: {}h)",
                    cluster.main_item.title,
                    (now_ts - cluster.main_item.timestamp) / 3600
                );

                // Build a short vector and remove immediately
                let stale_ids = vec![cluster.id.clone()];
                let buf = self.buffer.lock().await;
                if let Err(e) = buf.remove_clusters(category, &stale_ids) {
                    log::error!("Failed to remove stale cluster from DB: {}", e);
                }

                continue;
            }
            // --------------------------------------

            // Check global history for previously reported topics
            // Relaxed threshold: 12 (catch more candidates)
            let existing_record = self.registry.is_duplicate(&combined_text, 12)?;

            let mut is_follow_up = false;
            let mut matched_record = None;

            if let Some((candidate_record, distance)) = existing_record {
                if distance < 3 {
                    // Strong match: Assume it is the same/related topic without LLM (Fast path)
                    log::info!(
                        "Strong history match (Dist: {}): {}",
                        distance,
                        candidate_record.title
                    );
                    matched_record = Some(candidate_record);
                    is_follow_up = true;
                } else {
                    // Borderline match (3 <= distance < 12): Verify with LLM
                    log::info!(
                        "Borderline history match (Dist: {}). Verifying...",
                        distance
                    );

                    // Use existing verify logic but adapted for history check
                    // Need to construct a temp pending item to reuse logic or make a new prompt?
                    // Let's make a direct prompt here for clarity.
                    let prompt = format!(
                        "判断这两条新闻是否属于同一事件或强相关后续？\n\n\
                        历史报道:\n标题: {}\n摘要: {}\n\n\
                        今日新闻:\n标题: {}\n摘要: {}\n\n\
                        判断标准：\n- YES: 同一具体事件的后续进展、同一产品的更新、同一人物的动态。\n- NO: 仅仅是同类话题（如都讲AI但不同公司）。\n\
                        仅回答 YES 或 NO。",
                        candidate_record.title, candidate_record.summary,
                        cluster.main_item.title, summary
                    );

                    let response = self.llm.chat(&prompt, false).await?;
                    if response.to_uppercase().contains("YES") {
                        log::info!("LLM confirmed history match: {}", cluster.main_item.title);
                        matched_record = Some(candidate_record);
                        is_follow_up = true;
                    } else {
                        log::info!("LLM rejected history match. Treating as NEW.");
                    }
                }
            }

            if is_follow_up {
                let prev_record = matched_record.unwrap();
                // Topic was previously reported - check if there's new information
                match self
                    .check_for_updates(&cluster, summary, &prev_record)
                    .await
                {
                    Ok(Some(update_summary)) => {
                        // Has new information - include as a follow-up story
                        log::info!("Follow-up story: {}", cluster.main_item.title);
                        unique_topic_count += 1;

                        // Queue for registry update (only commit if episode is published)
                        topics_to_record.push((
                            combined_text.clone(),
                            cluster.main_item.title.clone(),
                            summary.to_string(),
                        ));

                        let source_str = cluster
                            .main_item
                            .source_name
                            .as_deref()
                            .unwrap_or("Unknown");
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
                            source_name: cluster
                                .main_item
                                .source_name
                                .as_deref()
                                .unwrap_or("Unknown")
                                .to_string(),
                            original_url: cluster.main_item.link.clone(),
                            is_update: true,
                            publish_time: cluster.main_item.timestamp as i64,
                        });
                    }
                    Ok(None) => {
                        // No substantial new information - discard
                        log::info!("Skipping (no new info): {}", cluster.main_item.title);
                        continue;
                    }
                    Err(e) => {
                        log::warn!("Update check failed: {}, skipping", e);
                        continue;
                    }
                }
            } else {
                // New topic - queue for record with full details
                topics_to_record.push((
                    combined_text.clone(),
                    cluster.main_item.title.clone(),
                    summary.to_string(),
                ));
                unique_topic_count += 1;

                let source_str = cluster
                    .main_item
                    .source_name
                    .as_deref()
                    .unwrap_or("Unknown");
                source_text.push_str(&format!(
                    "### Story {}\nSource: {}\nTitle: {}\nSummary: {}\n\n---\n\n",
                    idx + 1,
                    source_str,
                    cluster.main_item.title,
                    summary
                ));

                all_sources.push(crate::core::nexus::SourceInfo {
                    url: cluster.main_item.link.clone(),
                    title: cluster.main_item.title.clone(),
                    summary: summary.clone(),
                });

                broadcast_items.push(BroadcastItem {
                    id: idx + 1,
                    title: cluster.main_item.title.clone(),
                    summary: summary.clone(),
                    source_name: cluster
                        .main_item
                        .source_name
                        .as_deref()
                        .unwrap_or("Unknown")
                        .to_string(),
                    original_url: cluster.main_item.link.clone(),
                    is_update: false,
                    publish_time: cluster.main_item.timestamp as i64,
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
            log::info!(
                "Postponing [{}]: Only {} unique topics after dedup",
                category,
                unique_topic_count
            );
            return Ok(false);
        }

        if source_text.is_empty() {
            return Ok(false);
        }

        log::info!(
            "Generating episode for [{}]: {} unique topics",
            category,
            unique_topic_count
        );

        log::info!(
            "Generating episode for [{}]: {} unique topics",
            category,
            unique_topic_count
        );

        // Call produce_episode with smart flow enabled (via items)
        let result = self
            .produce_episode(category, &source_text, Some(&broadcast_items), false)
            .await;

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
            original_url: Some(
                all_sources
                    .first()
                    .map(|s| s.url.clone())
                    .unwrap_or_default(),
            ),
            cover_image_url: None,
            audio_url: None, // Will be filled by Nexus if file is provided
            publish_time: Some(chrono::Utc::now().timestamp()),
            duration_sec: Some(duration),
            sources: Some(all_sources),
            category: Some(category.to_string()),
        };

        self.nexus.push_item_multipart(payload, audio_bytes).await?;
        log::info!("Published Digest for [{}]", category);

        // Finalize: Commit all recorded topics to registry now that publication succeeded
        for (text, title, sum) in topics_to_record {
            if let Err(e) = self.registry.record_topic_with_details(&text, &title, &sum) {
                log::error!("Failed to record topic in registry: {}", e);
            }
        }

        Ok(true)
    }

    // --- Core Unified Content Engine ---

    async fn produce_episode(
        &self,
        category: &str,
        context: &str,
        items: Option<&[BroadcastItem]>,
        is_regen: bool,
    ) -> Result<(String, Option<String>, Option<Vec<u8>>, i64, bool)> {
        // 1. Resolve Host & Voice
        let host = self.hosts.as_ref().and_then(|h| {
            h.iter()
                .find(|host| host.categories.iter().any(|c| c == category))
        });
        let host_name = host.map(|h| h.name.clone()).unwrap_or("主播".to_string());
        let host_voice = host.map(|h| h.voice.clone());

        // 2. Resolve date context once so prompts, traces, and cache keys agree.
        let date_context = self.get_episode_date_context();

        // Initialize Tracer
        let logger = Arc::new(Mutex::new(TraceLogger::new(category)));
        logger.lock().await.log(
            "Start",
            &format!("Producing Episode for [{}]. Regen: {}", category, is_regen),
        );

        let host_val = if let Some(hosts) = &self.hosts {
            hosts
                .iter()
                .find(|h| h.categories.contains(&category.to_string()))
        } else {
            None
        };
        let host_prompt = host_val.and_then(|h| h.prompt_text.clone());

        // Create Channel for Concurrent TTS Pipeline
        let (tx, rx) = tokio::sync::mpsc::channel::<String>(10);
        let tts_client = self.tts.clone();
        let host_voice_clone = host_voice.clone();
        let host_prompt_clone = host_prompt.clone();

        let tts_task: tokio::task::JoinHandle<Result<(Vec<u8>, i64), String>> =
            tokio::spawn(async move {
                tts_client
                    .speak_mp3_from_chunks(rx, host_voice_clone, host_prompt_clone, clean_for_tts)
                    .await
                    .map_err(|e| e.to_string())
            });

        // -- TTS DRAFT CACHE PRE-CHECK --
        use std::hash::Hasher;
        let mut tts_cache_path = None;
        let mut cached_script = None;
        let mut cached_title = None;

        if let Some(item_list) = items {
            if !is_regen {
                // Generate a deterministic hash from the sorted URLs of the items
                let mut hasher = twox_hash::XxHash64::with_seed(0);
                hasher.write(TTS_DRAFT_CACHE_VERSION.as_bytes());
                hasher.write(date_context.prompt_block.as_bytes());
                let mut urls: Vec<String> =
                    item_list.iter().map(|i| i.original_url.clone()).collect();
                urls.sort();
                for url in urls {
                    hasher.write(url.as_bytes());
                }
                let hash = hasher.finish();

                let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
                let draft_dir = home.join(".freshloop/cache/tts_drafts").join(category);
                let _ = std::fs::create_dir_all(&draft_dir);
                let draft_file = draft_dir.join(format!("{:016x}.json", hash));

                #[derive(serde::Deserialize, serde::Serialize)]
                struct TtsDraft {
                    raw_script: String,
                    generated_title: Option<String>,
                }

                if draft_file.exists() {
                    match std::fs::read_to_string(&draft_file) {
                        Ok(contents) => {
                            if let Ok(draft) = serde_json::from_str::<TtsDraft>(&contents) {
                                log::info!("TTS DRAFT CACHE HIT! Using pre-generated script for cluster hash {:016x}", hash);
                                logger
                                    .lock()
                                    .await
                                    .log("Cache Hit", "Recovered LLM generated script from disk.");
                                cached_script = Some(draft.raw_script);
                                cached_title = draft.generated_title;
                            }
                        }
                        Err(e) => log::warn!("Failed to read TTS draft cache: {}", e),
                    }
                }
                tts_cache_path = Some(draft_file);
            }
        }

        // SMART FLOW (Unified)
        // Check if we recovered from cache
        let (raw_script, generated_title) = if let Some(script) = cached_script {
            let _ = tx.send(script.clone()).await;
            (script, cached_title)
        } else if let Some(item_list) = items {
            log::info!(
                "Starting Smart Episode Generation for {} items...",
                item_list.len()
            );

            // Step A: Intelligent Structure Planning (Sort + Group)
            let tracer_clone = logger.clone();
            let mut plans = match self.plan_episode_structure(item_list, tracer_clone).await {
                Ok(p) => {
                    log::info!("Smart Flow: Planned {} segments", p.len());
                    p
                }
                Err(e) => {
                    logger.lock().await.log(
                        "Planning Failed",
                        &format!("Error: {}. Fallback to simple grouping.", e),
                    );
                    log::warn!("Smart Flow Planning failed: {}, falling back.", e);
                    // Fallback: chunks of 4 with default sequence action
                    let ids: Vec<usize> = item_list.iter().map(|i| i.id).collect();
                    ids.chunks(4)
                        .map(|c| SegmentPlan {
                            action: "sequence".to_string(),
                            ids: c.to_vec(),
                            transition_rationale: None,
                            group_theme: None,
                            merge_reason: None,
                        })
                        .collect()
                }
            };

            // Topic cap: limit to 15 groups to avoid shallow coverage
            const MAX_GROUPS: usize = 15;
            if plans.len() > MAX_GROUPS {
                log::info!(
                    "Capping episode from {} groups to {} groups",
                    plans.len(),
                    MAX_GROUPS
                );
                plans.truncate(MAX_GROUPS);
            }

            // Step A.5: Compress long summaries
            let mut all_items: Vec<BroadcastItem> = item_list.to_vec();
            if let Err(e) = self.compress_summaries(&mut all_items, 4000).await {
                log::warn!("Summary compression failed: {}", e);
            }

            // Step B (Unified): Generate Full Episode Script
            let script = self
                .generate_full_episode_script(
                    category,
                    &host_name,
                    &all_items,
                    &plans,
                    &date_context,
                    logger.clone(),
                    Some(tx.clone()),
                )
                .await?;

            // Step C: Extract title from content
            let title = match self.extract_episode_title(item_list, category).await {
                Ok(t) => Some(t),
                Err(e) => {
                    log::warn!("Title extraction failed: {}", e);
                    None
                }
            };

            // Cache the newly generated script and title to disk
            if let Some(path) = &tts_cache_path {
                #[derive(serde::Serialize)]
                struct TtsDraft<'a> {
                    raw_script: &'a str,
                    generated_title: Option<&'a String>,
                }
                let draft = TtsDraft {
                    raw_script: &script,
                    generated_title: title.as_ref(),
                };
                if let Ok(json) = serde_json::to_string(&draft) {
                    if let Err(e) = std::fs::write(path, json) {
                        log::warn!("Failed to save TTS draft to disk: {}", e);
                    } else {
                        log::info!("Saved TTS draft to disk: {:?}", path);
                    }
                }
            }

            (script, title)
        } else {
            // No Items (Regen or legacy fallback): Use simple prompt
            let prompt = self.build_prompt(category, &host_name, context, &date_context, is_regen);
            let response = self.llm.chat(&prompt, is_regen).await?;
            logger.lock().await.log_llm(
                "Simple Generation",
                "Used legacy/regen one-shot prompt",
                &prompt,
                &response,
            );
            let _ = tx.send(response.clone()).await;
            (response, None) // No title extraction for legacy mode
        };

        drop(tx); // Close the channel

        // 3. (Processed above)

        // 4. Check for SKIP
        if raw_script.trim().contains("SKIP") || raw_script.trim().len() < 10 {
            logger
                .lock()
                .await
                .log("Result", "LLM indicated SKIP or empty script.");
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
                script_body = final_full_text[newline_idx + 1..].trim().to_string();
            }
        }

        // 6. TTS Generation — Await the parallel pipeline result
        let (mp3_audio_bytes, duration) = match tts_task.await {
            Ok(Ok(result)) => result,
            Ok(Err(tts_err)) => {
                // TTS pipeline reported failure — propagate to trigger Peek-Ack retry
                let msg = format!("Parallel TTS pipeline error: {}", tts_err);
                log::error!("{}", msg);
                logger.lock().await.log("Audio Error", &msg);
                return Err(anyhow::anyhow!(msg));
            }
            Err(join_err) => {
                let msg = format!("TTS task panicked: {}", join_err);
                log::error!("{}", msg);
                logger.lock().await.log("Audio Error", &msg);
                return Err(anyhow::anyhow!(msg));
            }
        };

        // 7. MP3 bytes are produced directly by the streaming TTS pipeline.
        let final_audio = if !mp3_audio_bytes.is_empty() {
            logger.lock().await.log(
                "Audio Processing",
                &format!("MP3 generated directly from streamed PCM ({}s).", duration),
            );
            Some(mp3_audio_bytes)
        } else {
            None
        };

        if let Err(e) = logger.lock().await.save() {
            log::error!("Failed to save execution trace: {}", e);
        }

        // 8. Clean up TTS Cache on pure success
        if let Some(path) = tts_cache_path {
            if path.exists() && final_audio.is_some() {
                let _ = std::fs::remove_file(&path);
                log::info!("Cleaned up TTS draft cache file.");
            }
        }

        Ok((script_body, final_title, final_audio, duration, false))
    }

    fn get_episode_date_context(&self) -> EpisodeDateContext {
        build_episode_date_context(chrono::Local::now().date_naive())
    }

    fn build_prompt(
        &self,
        category: &str,
        host: &str,
        context: &str,
        date_context: &EpisodeDateContext,
        is_regen: bool,
    ) -> String {
        let regen_instruction = if is_regen {
            "注意：这是一个【重新生成】请求。请专注于改进提供的具体新闻故事，保留所有有意义的细节。"
        } else {
            "注意：这是一组新闻摘要。请将它们整合成一份连贯的【新闻简报】。如果有多条不同的新闻，请逐一播报。使用简洁的过渡方式，直接用新闻中的人物、地点、事件等信息进行衔接。"
        };

        format!(
            "角色：FreshLoop 信息密度型音频编辑兼主播（{}）。任务：撰写适合路上或碎片时间听的新闻音频稿。\n\
            {}\n\
            分类：{}\n\
            {}\n\
            \n\
            核心规则：\n\
            1. **结构**：一句自然开场 -> 逐条播报新闻 -> 一句自然结束语。\n\
            2. **标准开场**：自然融入\"欢迎收听 FreshLoop [{}频道]\"以及\"我是{}\"，不要寒暄太久；日期、周末、节假日、调休信息必须与【日期语境】一致，不要使用固定模板。\n\
            3. **标准结束语**：包含\"我是{}\"和 FreshLoop，但只用一句话收束。\n\
            4. **每条新闻的信息结构**：必须交代“发生了什么 / 关键事实 / 为什么和听众有关”。信息不足的低价值素材只用一句话带过。\n\
            5. **细节保留**：保留来源中的具体人名、公司名、数字、日期、地点和引语；没有给出的细节不要补。\n\
            6. **表达风格**：克制、清楚、有判断力，像聪明朋友在解释新闻，不像播报腔，也不要像营销号。\n\
            7. **禁止空话**：不要使用\"值得关注\"、\"引发热议\"、\"未来可期\"、\"意义重大\"、\"不容忽视\"这类没有信息量的形容。\n\
            8. **简洁衔接**：段落之间只用事实性衔接，禁止硬凑宏大主题。\n\
            9. **禁止幻觉**：不要编造未提供的信息。\n\
            10. **格式**：纯文本，不要使用Markdown。\n\
            11. **标题要求**：请在输出的第一行生成一个标准标题，格式严格为：`TITLE: 内容关键词`。\n\
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
            host,
            date_context.prompt_block,
            category,
            regen_instruction,
            category,
            host,
            host,
            context
        )
    }

    // --- New Helper Functions ---

    /// Compress long summaries before segmentation
    async fn compress_summaries(
        &self,
        items: &mut Vec<BroadcastItem>,
        max_chars: usize,
    ) -> Result<()> {
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

    /// Step 3 (Unified): Generate Full Episode Script
    async fn generate_full_episode_script(
        &self,
        category: &str,
        host_name: &str,
        items: &[BroadcastItem],
        plans: &[SegmentPlan],
        date_context: &EpisodeDateContext,
        logger: Arc<Mutex<TraceLogger>>,
        tx: Option<tokio::sync::mpsc::Sender<String>>,
    ) -> Result<String> {
        let total_items = items.len();
        log::info!(
            "Unified Generation: {} groups, {} items",
            plans.len(),
            total_items
        );

        // Chunking logic to avoid huge prompts (e.g. 130k chars)
        let mut chunks: Vec<Vec<SegmentPlan>> = Vec::new();
        let mut current_chunk: Vec<SegmentPlan> = Vec::new();
        let mut current_items = 0;

        for plan in plans {
            if current_items + plan.ids.len() > 10 && !current_chunk.is_empty() {
                chunks.push(current_chunk);
                current_chunk = Vec::new();
                current_items = 0;
            }
            current_chunk.push(plan.clone());
            current_items += plan.ids.len();
        }
        if !current_chunk.is_empty() {
            chunks.push(current_chunk);
        }

        let mut full_script = String::new();
        let total_chunks = chunks.len();
        let mut previous_chunk_ending: Option<String> = None;

        for (chunk_idx, plan_chunk) in chunks.iter().enumerate() {
            let is_first = chunk_idx == 0;
            let is_last = chunk_idx == total_chunks - 1;

            // 1. Build Full Run of Show (Plan) for this chunk
            let mut ros = String::new();
            if is_first {
                ros.push_str(&format!("【节目单 - {}频道】\n", category));
                ros.push_str(&format!("时间: {}\n", date_context.run_of_show_line));
                ros.push_str(&format!("主播: {}\n", host_name));
            }
            ros.push_str(&format!(
                "\n--- 第 {}/{} 部分播报流程 ---\n",
                chunk_idx + 1,
                total_chunks
            ));

            for (idx, plan) in plan_chunk.iter().enumerate() {
                let theme = plan.group_theme.as_deref().unwrap_or("新闻组");
                let action = &plan.action;
                let rationale = plan.transition_rationale.as_deref().unwrap_or("自然过渡");
                ros.push_str(&format!(
                    "{}. [{}] 主题：{} (过渡策略：{})\n",
                    idx + 1,
                    action.to_uppercase(),
                    theme,
                    rationale
                ));
            }

            // 2. Build Content Block (Grouped)
            let mut content = String::new();
            for (idx, plan) in plan_chunk.iter().enumerate() {
                content.push_str(&format!(
                    "\n=== 第 {}/{} 部分 - 第 {} 组 (主题：{}) ===\n",
                    chunk_idx + 1,
                    total_chunks,
                    idx + 1,
                    plan.group_theme.as_deref().unwrap_or("Untitled")
                ));

                for id in &plan.ids {
                    if let Some(item) = items.iter().find(|i| i.id == *id) {
                        content.push_str(&format!(
                            "--- Item ---\n标题: {}\n来源: {}\n摘要: {}\n",
                            item.title, item.source_name, item.summary
                        ));
                    }
                }
            }

            // 3. Prompt Construction
            let run_of_show = ros;
            let full_content_block = content;

            let requirement_intro_outro = if total_chunks == 1 {
                format!(
                    "必须包含 [开场白] -> [正文(按顺序串联)] -> [结束语]。\n\
                - 开场白: \"大家好，欢迎收听 FreshLoop {}频道...\" (可自然包含日期或时间语境，但必须准确)\n\
                - 结束语: \"以上就是本期内容...\"",
                    category
                )
            } else if is_first {
                format!(
                    "必须包含 [开场白] -> [正文(按顺序串联)]，不要写结束语，因为后面还有内容。\n\
                - 开场白: \"大家好，欢迎收听 FreshLoop {}频道...\" (可自然包含日期或时间语境，但必须准确)",
                    category
                )
            } else if is_last {
                "必须包含 [正文(按顺序串联)] -> [结束语]，不要写开场白，直接承接上半部分。\n\
                - 结束语: \"以上就是本期内容...\""
                    .to_string()
            } else {
                "必须只包含 [正文(按顺序串联)]，不要写开场白，不要写结束语，直接承接上半部分并为下半部分留口子。".to_string()
            };

            let previous_context_str = if let Some(ref text) = previous_chunk_ending {
                format!("\n【上一部分结尾供参考(Strict Context)】\n{}\n(请务必与上述结尾做无缝、强关联的逻辑性承接，**严禁**使用\"接下来\"、“然后”等生硬套话！)\n", text)
            } else {
                String::new()
            };

            let prompt = format!(
                "Role: FreshLoop 信息密度型音频编辑兼主播 (Host: {})。\n\
                频道: {}\n\
                {}\n\
                \n\
                【任务目标】\n\
                基于以下【第 {}/{} 部分的节目编排表】和【详细新闻素材】，撰写一份适合路上或碎片时间听的单人口播稿。\n\
                {}\
                \n\
                【节目编排表 (Run of Show)】\n\
                {}\n\
                \n\
                【详细新闻素材】\n\
                {}\n\
                \n\
                【撰写要求 (Critical)】\n\
                1. **结构安排**：{}\n\
                2. **素材忠诚度 (Strict Content Fidelity)**：\n\
                   - **以素材为准**：必须基于提供的【详细新闻素材】。禁止引入你训练记忆中可能冲突的事实。\n\
                   - **禁止幻觉**：严禁编造任何时间、地点、天气、具体数字或素材中未提及的背景故事。\n\
                3. **音频信息结构**：每条新闻都尽量回答三件事：发生了什么、关键事实是什么、为什么和听众有关。信息不足的低价值素材只用一句话带过。\n\
                4. **时间语境准确**：开场如提到日期、周末、节假日或调休，只能使用上面的【日期语境】；不要套用“工作日”“周末”“通勤早高峰”等与日期语境冲突的固定说法。\n\
                5. **简洁衔接**：段落之间的过渡必须短、自然、基于事实。\n\
                   - 可以直接进入下一条，不必每条都解释为什么相关。\n\
                   - 禁止使用\"我们先从...说起\"、\"说到...\"、“接下来我们来看”等固定模板。如果提供了【上一部分结尾供参考】，第一句要顺着语境承接。\n\
                   - 禁止使用：\"视线转向\"、\"视线转回\"、\"不妨把目光投向\"、\"同样需要\"、\"聊完…不妨…\"、\"话题延伸至\"、\"议题回归\"、\"视角转换至\"、\"从…延伸到…\"。\n\
                6. **口语化改写（必须）**：\n\
                   - 严禁逐字照搬素材摘要原文，必须用自己的话重新表述。\n\
                   - 语气像聪明朋友在解释新闻，不是照本宣科念新闻稿，也不是夸张播客腔。\n\
                   - 少用语气词，不要为了“活泼”牺牲信息密度。\n\
                7. **篇幅控制**：重要新闻 160-240 字，普通新闻 80-130 字，低价值新闻一句带过或自然省略；不要让每条听起来一样长。\n\
                8. **禁止空话**：不要使用\"值得关注\"、\"引发热议\"、\"未来可期\"、\"意义重大\"、\"不容忽视\"这类没有信息量的形容。\n\
                \n\
                【格式禁忌 (Strict Mocks)】\n\
                1. **纯文本输出**：输出必须是【纯纯的口播稿】！\n\
                2. **严禁包含任何标记**：严禁出现 【正文】、[Music]、(记者报道) 等元数据或结构标记！\n\
                3. **严禁小标题**：不要给每条新闻加标题，直接用话术过渡。\n\
                \n\
                现在，请输出克制、顺滑、信息密度高的口播稿：",
                host_name,
                category,
                date_context.prompt_block,
                chunk_idx + 1, total_chunks,
                previous_context_str,
                run_of_show,
                full_content_block,
                requirement_intro_outro
            );

            // 4. LLM Call
            logger.lock().await.log(
                "Unified Gen Chunk",
                &format!(
                    "Starting generation for chunk {}/{}... (Length: {})",
                    chunk_idx + 1,
                    total_chunks,
                    prompt.len()
                ),
            );
            let response = self.llm.chat(&prompt, false).await?;

            logger.lock().await.log_llm(
                "Unified Result Chunk",
                &format!("Script Chunk {}/{}", chunk_idx + 1, total_chunks),
                &prompt,
                &response,
            );

            let cleaned = clean_content(response);

            // Capture the tail for the next chunk's context
            let tail_chars = 150; // Extract last 150 chars
            let char_count = cleaned.chars().count();
            if char_count > tail_chars {
                previous_chunk_ending =
                    Some(cleaned.chars().skip(char_count - tail_chars).collect());
            } else {
                previous_chunk_ending = Some(cleaned.clone());
            }

            full_script.push_str(&cleaned);
            full_script.push_str("\n\n");

            if let Some(sender) = &tx {
                if let Err(e) = sender.send(cleaned.clone()).await {
                    log::warn!("Failed to send chunk to TTS pipeline: {}", e);
                }
            }
        }

        // 5. Post-Processing / Safety
        let final_script = full_script.trim().to_string();
        if final_script.len() < 300 && total_chunks == 1 {
            return Err(anyhow::anyhow!(
                "Generated script too short (<300 chars). Likely just an intro."
            ));
        }

        Ok(final_script)
    }

    /// Extract episode title from news content
    async fn extract_episode_title(
        &self,
        items: &[BroadcastItem],
        category: &str,
    ) -> Result<String> {
        let top_titles: String = items
            .iter()
            .take(5)
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
            直接输出标题文本，不要 JSON，不要引号，不要任何格式包裹。\n\
            示例：苹果发布M5芯片/SpaceX星舰第七次试飞",
            category, top_titles
        );

        let response = self.llm.chat(&prompt, false).await?;

        // Post-processing: clean up the title
        let mut title = response.trim().to_string();

        // Try to extract from JSON if LLM still returned JSON despite instructions
        if title.contains('{') {
            let json_clean = title.trim_matches('`').trim().to_string();
            if let Some(start) = json_clean.find('{') {
                if let Some(end) = json_clean.rfind('}') {
                    if start <= end {
                        if let Ok(val) =
                            serde_json::from_str::<serde_json::Value>(&json_clean[start..=end])
                        {
                            if let Some(t) = val["title"].as_str() {
                                title = t.trim().to_string();
                            }
                        }
                    }
                }
            }
        }

        // Strip wrapping quotes
        title = title.trim_matches('"').trim_matches('\'').to_string();
        for ch in ['\u{201c}', '\u{201d}', '\u{2018}', '\u{2019}'] {
            title = title.replace(ch, "");
        }
        // Strip "title:" prefix variants
        let title_lower = title.to_lowercase();
        for prefix in ["title:", "标题:", "标题："] {
            if title_lower.starts_with(prefix) {
                title = title[prefix.len()..].trim().to_string();
                break;
            }
        }

        // Validate: reject obviously bad titles
        let now = chrono::Local::now();
        let date_suffix = now.format("%m.%d").to_string();
        let is_invalid = title.is_empty()
            || title.contains("你的标题")
            || title.starts_with('{')
            || title.contains("\"title\"")
            || title.len() > 200
            || title.contains('\n')
            || title.to_lowercase().contains("news digest")
            || title.to_lowercase().contains("news briefing");

        if is_invalid {
            log::warn!(
                "Invalid title detected: \'{}\', using fallback",
                title.chars().take(50).collect::<String>()
            );
            // Build fallback from top 2-3 item titles
            let short_titles: Vec<String> = items
                .iter()
                .take(3)
                .map(|i| i.title.chars().take(12).collect::<String>())
                .collect();
            title = format!("{} {}", short_titles.join("/"), date_suffix);
        }

        // Final length guard
        if title.chars().count() > 40 {
            title = title.chars().take(40).collect();
        }

        Ok(title)
    }

    /// Step 1 (New): Entity-Based Grouping with Rich Context
    async fn group_items_by_entity(&self, items: &[BroadcastItem]) -> Result<Vec<ClusterGroup>> {
        let item_list: String = items
            .iter()
            .map(|item| format!("ID {}: {} ({})", item.id, item.title, item.source_name))
            .collect::<Vec<_>>()
            .join("\n");

        let prompt = format!(
            "Role: 新闻编辑助理 - 聚类专家。\n\
            Task: 将以下新闻按主题/实体分组，并说明分组理由。\n\
            \n\
            新闻列表:\n{}\n\
            \n\
            规则:\n\
            1. **严格分组**: 同一公司/产品/事件的新闻必须放在一起\n\
               (如 'Logitech鼠标' 和 'Logitech键盘' → 同组, 'Apple财报' 和 'Apple股价' → 同组)\n\
            2. **单条新闻**: 无关联的新闻自成一组\n\
            3. **全覆盖**: 每条新闻必须且只能出现在一个组中\n\
            \n\
            输出 JSON 数组，每个元素包含:\n\
            - ids: 新闻ID数组\n\
            - theme: 这组新闻的主题 (简短，如\"Meta核能计划\"、\"GameStop战略调整\")\n\
            - clustering_reason: 为什么这些新闻放一起 (如\"同一公司的不同产品发布\"、\"同一事件的多角度报道\")\n\
            \n\
            示例输出:\n\
            [\n\
              {{\"ids\": [1, 3], \"theme\": \"Logitech产品线\", \"clustering_reason\": \"同一公司在同一发布会上发布的多款产品\"}},\n\
              {{\"ids\": [2], \"theme\": \"Apple财报\", \"clustering_reason\": \"独立新闻\"}}\n\
            ]",
            item_list
        );

        let response = self.llm.chat(&prompt, false).await?;

        // Parse JSON
        let json_clean = response.trim().trim_matches('`').trim();

        // Safe JSON array extraction
        let json_str = match (json_clean.find('['), json_clean.rfind(']')) {
            (Some(s), Some(e)) if e >= s => &json_clean[s..=e],
            _ => "[]",
        };
        if let Ok(groups) = serde_json::from_str::<Vec<ClusterGroup>>(json_str) {
            // Validate all IDs exist
            let valid_ids: std::collections::HashSet<usize> = items.iter().map(|i| i.id).collect();
            let all_valid = groups
                .iter()
                .all(|g| g.ids.iter().all(|id| valid_ids.contains(id)));
            if all_valid && !groups.is_empty() {
                return Ok(groups);
            }
        }

        // Fallback: Each item is its own group with generic theme
        Ok(items
            .iter()
            .map(|i| ClusterGroup {
                ids: vec![i.id],
                theme: i.title.chars().take(20).collect::<String>() + "...",
                clustering_reason: "独立新闻".to_string(),
            })
            .collect())
    }

    /// Step 2: Intelligent Structure Planning (Two-Pass: Cluster -> Sequence)
    async fn plan_episode_structure(
        &self,
        items: &[BroadcastItem],
        logger: Arc<Mutex<TraceLogger>>,
    ) -> Result<Vec<SegmentPlan>> {
        // Phase 1: Clustering with Rich Context
        log::info!("Phase 1: Clustering {} items by entity...", items.len());
        let cluster_groups = match self.group_items_by_entity(items).await {
            Ok(g) => g,
            Err(e) => {
                log::warn!("Clustering failed: {}. Falling back to singletons.", e);
                items
                    .iter()
                    .map(|i| ClusterGroup {
                        ids: vec![i.id],
                        theme: i.title.chars().take(20).collect::<String>() + "...",
                        clustering_reason: "聚类失败回退".to_string(),
                    })
                    .collect()
            }
        };
        logger
            .lock()
            .await
            .log("Clustering Result", &format!("{:?}", cluster_groups));

        // Filter out low-value classified ads (rentals, job search, ads) if they have only 1 item
        let mut filtered_groups = Vec::new();
        for group in cluster_groups {
            let mut is_low_value = false;
            // Only filter if it's a singleton (a single forum post)
            if group.ids.len() == 1 {
                let theme_lower = group.theme.to_lowercase();
                if theme_lower.contains("求职")
                    || theme_lower.contains("招聘")
                    || theme_lower.contains("合租")
                    || theme_lower.contains("转让")
                    || theme_lower.contains("出售")
                    || theme_lower.contains("出二手")
                    || theme_lower.contains("接单")
                {
                    is_low_value = true;
                }
            }
            if is_low_value {
                log::info!(
                    "Filtered out low-value classified ad group: {}",
                    group.theme
                );
            } else {
                filtered_groups.push(group);
            }
        }
        let cluster_groups = filtered_groups;

        // Phase 2: Sequencing the Groups
        // Build descriptions with theme info
        let mut group_descriptions = Vec::new();
        for (idx, group) in cluster_groups.iter().enumerate() {
            let titles: Vec<String> = group
                .ids
                .iter()
                .filter_map(|id| items.iter().find(|i| i.id == *id).map(|i| i.title.clone()))
                .collect();
            group_descriptions.push(format!(
                "Group {}: 【主题: {}】{} (聚类原因: {})",
                idx,
                group.theme,
                titles.join(" / "),
                group.clustering_reason
            ));
        }

        let prompt_v2 = format!(
             "Role: 节目编排导演。\n\
             Task: 将以下新闻组编排成完整的节目结构。\n\
             \n\
             新闻组列表:\n{}\n\
             \n\
             编排指令:\n\
             1. **排序**: 按逻辑顺序排列 (如 科技巨头 -> 硬件 -> 科学)\n\
             2. **Action决策**:\n\
                - 'merge': 如果组内多条新闻是完全重复或同一事件\n\
                - 'sequence': 如果组内新闻是相关但独立的更新\n\
             3. **Transition Rationale** (非首组必填):\n\
                - 简要说明为什么这组跟在上一组后面\n\
                - 这将指导主播如何过渡\n\
             4. **输出格式**: JSON 数组\n\
                [{{\"action\": \"merge\", \"group_index\": 0, \"transition_rationale\": null}}, {{\"action\": \"sequence\", \"group_index\": 1, \"transition_rationale\": \"从软件转向硬件——两者都与AI性能相关\"}}]\n\
             \n\
             仅输出 JSON。",
             group_descriptions.join("\n")
        );

        #[derive(serde::Deserialize, schemars::JsonSchema)]
        struct Step {
            action: String,
            group_index: usize,
            #[serde(default)]
            transition_rationale: Option<String>,
        }

        let response = self.llm.chat(&prompt_v2, false).await?;
        logger
            .lock()
            .await
            .log_llm("Planning Phase 2", "Sequencing", &prompt_v2, &response);

        let json_clean = response.trim().trim_matches('`').trim();

        // Safe JSON array extraction
        let steps: Vec<Step> = match (json_clean.find('['), json_clean.rfind(']')) {
            (Some(s), Some(e)) if e >= s => {
                serde_json::from_str(&json_clean[s..=e]).unwrap_or_default()
            }
            _ => Vec::new(),
        };

        // Convert back to SegmentPlan with rich context from ClusterGroup
        let mut final_plans = Vec::new();
        for step in steps {
            if let Some(cluster) = cluster_groups.get(step.group_index) {
                final_plans.push(SegmentPlan {
                    action: step.action.clone(),
                    ids: cluster.ids.clone(),
                    transition_rationale: step.transition_rationale,
                    group_theme: Some(cluster.theme.clone()),
                    merge_reason: if step.action == "merge" {
                        Some(cluster.clustering_reason.clone())
                    } else {
                        None
                    },
                });
            }
        }

        // Fallback if empty or failed
        if final_plans.is_empty() {
            log::warn!("Planning V2 failed/empty. Using raw groups.");
            for cluster in cluster_groups {
                final_plans.push(SegmentPlan {
                    action: "sequence".to_string(),
                    ids: cluster.ids,
                    transition_rationale: None,
                    group_theme: Some(cluster.theme),
                    merge_reason: None,
                });
            }
        }

        Ok(final_plans)
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
    pub publish_time: i64,
}

fn clean_for_tts(input: &str) -> String {
    tts::normalize_for_tts(input, tts::NormalizeOptions::default())
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
struct SegmentPlan {
    action: String, // "sequence" | "merge"
    ids: Vec<usize>,
    #[serde(default)]
    transition_rationale: Option<String>, // Planner-Driven Transition Logic
    #[serde(default)]
    group_theme: Option<String>, // 这组新闻的主题 (如 "科技巨头动态")
    #[serde(default)]
    merge_reason: Option<String>, // 为什么合并 (如 "同一公司的多个产品发布")
}

/// Rich clustering result with reasoning
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
struct ClusterGroup {
    ids: Vec<usize>,
    theme: String,             // 这组的主题
    clustering_reason: String, // 为什么放在一起
}

// Helper function to clean LLM output
fn clean_content(text: String) -> String {
    use regex::Regex;
    let mut cleaned = text;

    // 1. Remove [Structure Markers] and 【Structure Markers】
    // e.g. 【正文】, [Music], 【MERGE】
    let re_brackets = Regex::new(r"【.*?】|\[.*?\]").unwrap();
    cleaned = re_brackets.replace_all(&cleaned, "").to_string();

    // 2. Remove specific banned phrases (Ending cues that might slip through)
    let banned = [
        "本条播放完毕",
        "（播报结束）",
        "播报结束",
        "谢谢收听",
        "以上是",
    ];
    for phrase in banned {
        cleaned = cleaned.replace(phrase, "");
    }

    // 3. Remove Title/Source lines often hallucinated
    // e.g. "Title: ..." or "Source: ..." or "### Segment 1"
    let re_meta =
        Regex::new(r"(?im)^(Title|Source|Category|###|Group|Segment)\s*[:：].*$").unwrap();
    cleaned = re_meta.replace_all(&cleaned, "").to_string();

    // 4. Remove standalone ### headers
    let re_h3 = Regex::new(r"(?m)^###.*$").unwrap();
    cleaned = re_h3.replace_all(&cleaned, "").to_string();

    // 5. Remove parenthetical source citations (Safe to remove all parenthesized sources)
    // format: (Source: XXX) or (来源: XXX)
    let re_source = Regex::new(r"(?i)[（\(]\s*(source|来源)[:：].*?[）\)]").unwrap();
    cleaned = re_source.replace_all(&cleaned, "").to_string();

    // 6. Final whitespace cleanup
    cleaned = cleaned.trim().to_string();
    // Remove multiple newlines
    let re_newlines = Regex::new(r"\n{3,}").unwrap();
    cleaned = re_newlines.replace_all(&cleaned, "\n\n").to_string();

    cleaned
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    fn assert_no_plain_workday_context(context: &EpisodeDateContext) {
        let forbidden = ["普通", "工作日"].join("");
        assert!(!context.prompt_block.contains(&forbidden));
    }

    #[test]
    fn date_context_marks_plain_saturday_as_weekend_not_workday() {
        let context = build_episode_date_context(date(2026, 6, 27));

        assert!(context.prompt_block.contains("周六"));
        assert!(context.prompt_block.contains("周末"));
        assert!(context.prompt_block.contains("不要说今天是工作日"));
        assert_no_plain_workday_context(&context);
    }

    #[test]
    fn date_context_does_not_force_workday_for_regular_weekday() {
        let context = build_episode_date_context(date(2026, 6, 24));

        assert!(context.prompt_block.contains("周三"));
        assert!(context.prompt_block.contains("不需要强行说工作日"));
        assert_no_plain_workday_context(&context);
    }

    #[test]
    fn date_context_marks_known_official_holiday() {
        let context = build_episode_date_context(date(2026, 2, 17));

        assert!(context.prompt_block.contains("春节假期"));
        assert!(context.prompt_block.contains("不要把今天说成工作日"));
        assert_no_plain_workday_context(&context);
    }

    #[test]
    fn date_context_marks_adjusted_weekend_workday_precisely() {
        let context = build_episode_date_context(date(2026, 2, 14));

        assert!(context.prompt_block.contains("周六"));
        assert!(context.prompt_block.contains("调休工作日"));
        assert_no_plain_workday_context(&context);
    }

    #[test]
    fn date_context_projects_2027_spring_festival_window() {
        let eve = build_episode_date_context(date(2027, 2, 5));
        let tail = build_episode_date_context(date(2027, 2, 12));

        assert!(eve.prompt_block.contains("春节假期"));
        assert!(eve.prompt_block.contains("规则推算"));
        assert!(tail.prompt_block.contains("春节假期"));
    }

    #[test]
    fn date_context_projects_2028_national_mid_autumn_combined_window() {
        let context = build_episode_date_context(date(2028, 10, 8));

        assert!(context.prompt_block.contains("国庆中秋假期"));
        assert!(context.prompt_block.contains("规则推算"));
    }

    #[test]
    fn future_weekend_without_official_calendar_stays_neutral() {
        let context = build_episode_date_context(date(2027, 6, 26));

        assert!(context.prompt_block.contains("周六"));
        assert!(context.prompt_block.contains("官方调休日历尚未内置"));
        assert!(context.prompt_block.contains("不要说今天是工作日"));
        assert!(!context.prompt_block.contains("今天是周末"));
    }
}
