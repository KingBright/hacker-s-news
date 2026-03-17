# System Control Flow & Intelligence Pipeline (Cortex Core)

This document provides a technical deep-dive into the control flow of the Cortex engine, using pseudo-code to illustrate the decision-making process and detailing the exact LLM prompts used at each intelligence gate.

## 1. Ingestion & Deduplication Loop (`process_rss_sources`)

**Goal**: Convert raw RSS XML into clean, deduplicated `NewsCluster` candidates. **Preserve original article text** for downstream generation.

```python
for source in rss_sources:
    if source.url in cache_db: continue  # L0 Deduplication (URL exact match)

    xml_items = fetch_rss(source.url)
    
    for item in xml_items:
        # L1: Keyword Filtering (Blocklist)
        if matches_blocklist(item.title): continue
        
        # Clean and preserve original text (truncated to 5000 chars)
        clean_desc = clean_text(item.description, max_chars=5000)
        
        # LLM Classification & Summary (2-sentence summary for dedup)
        analysis = llm_classify(item.title, clean_desc)
        
        pending_item = PendingNewsItem(
            title=analysis.title,
            description=analysis.summary,      # 2-sentence summary (for dedup)
            original_text=clean_desc,           # Full RSS text (for generation)
            category=analysis.category,
            ...
        )
        
        # L2: SimHash Clustering (Fuzzy Match)
        simhash = calculate_simhash(item.title + item.description)
        candidate_cluster = find_closest_cluster(simhash, threshold=10)
        
        if candidate_cluster:
            # L3: Semantic Verification (LLM)
            if llm_verify_same_topic(item, candidate_cluster.main_item):
                merge_into_cluster(candidate_cluster, item)
            else:
                create_new_cluster(item)
        else:
            create_new_cluster(item)
```

> **Key Design**: `original_text` preserves the full RSS article content through the entire pipeline. The 2-sentence `description` is only used for deduplication and quick reference. Final generation uses `original_text` as the primary source material.

### 🧠 LLM Gate: Topic Verification
**Function**: `llm_verify_same_topic`
**Prompt**:
```text
判断以下两条新闻是否报道同一个事件/话题？

新闻A：{item_a.title}
新闻B：{item_b.title}

判断标准：
1. 同一事件指同一个具体事件、产品、人物动态。
2. 即使领域相同（如都是AI新闻），如果事件不同（OpenAI发布新模型 vs Google发布新模型），也算 NO。

仅回答 YES 或 NO。
```

---

## 2. Intelligence Clustering & Synthesis (`process_clusters`)

**Goal**: Turn a cluster of related articles into a single, high-density summary for the broadcast. Preserve original text for downstream generation.

```python
def process_cluster(cluster):
    # Strategy: Merge multiple sources into one truth (single LLM call)
    if cluster.has_multiple_items():
        merged_summary = llm_merge_items(cluster.items)  # One-pass, no review loop
        cluster.final_summary = merged_summary
    
    # Collect original text from all sources in cluster (dynamic budget)
    max_chars_per_item = min(3000, max(500, 20000 / total_cluster_count))
    cluster.original_text = collect_original_text(cluster, max_chars_per_item)
    
    # Check for Updates (if topic was broadcasted before)
    if topic_registry.has_seen(cluster.topic_id):
        update_info = check_for_updates(cluster.final_summary, previous_broadcast)
        if update_info.is_significant:
            mark_as_update(cluster)
        else:
            discard(cluster) # Old news
```

### LLM Gate: Merge & Synthesize
**Function**: `llm_merge_items` (Single-pass, no review loop)
**Prompt**:
```text
任务：将以下多来源新闻合并为一条综合简报。

已有内容:
标题: {existing_title}
摘要: {existing_summary}

新内容:
标题: {new_title}
摘要: {new_summary}

要求：
1. 100% 基于提供的素材，禁止添加任何素材中没有的信息。
2. 保留所有具体数字、日期、人名、机构名。
3. 客观陈述，禁止主观评价词。
4. 极高信息密度，拒绝废话。
5. 输出JSON格式: {"title": "综合标题", "summary": "综合摘要"}
```

> **Design Change**: Previously used a 3-round review loop (`review_summary`) + editor feedback. Now simplified to single LLM call with 1 retry on JSON parse failure. Saves 2-5 LLM calls per merge operation.

---

## 3. Episode Orchestration ("Smart Flow") (`produce_episode`)

**Goal**: Convert a list of isolated clusters -> Coherent ~5-minute Podcast Script, using original article text as primary source.

```python
def produce_episode(clusters, category):
    # Step A: Narrative Planning (LLM groups and orders items)
    plans = plan_episode_flow(clusters)  # Returns SegmentPlan with themes & transitions
    
    # Step B: Build content block with original text
    content_block = ""
    for plan in plans:
        for item in plan.items:
            if item.original_text:
                content_block += f"标题: {item.title}\n摘要: {item.summary}\n原文: {item.original_text}\n"
            else:
                content_block += f"标题: {item.title}\n内容: {item.summary}\n"  # Fallback
    
    # Step C: Unified Generation (single LLM call for entire episode)
    final_script = generate_full_episode_script(
        run_of_show=plans,
        content=content_block,
        host=host_name,
        category=category
    )
    
    # Step D: Post-processing (TTS-safe cleanup)
    final_script = clean_content(final_script)  # Remove all brackets, markdown, etc.
    
    # Step E: Audio Production
    wav_bytes = tts_engine.speak(final_script)
    mp3_bytes = convert_to_mp3(wav_bytes)
    
    return final_script, mp3_bytes
```

### LLM Gate: Unified Episode Generation
**Function**: `generate_full_episode_script`
**Core Principles**:
```text
Role: FreshLoop 新闻播报员

【核心原则 - 100% 内容忠诚度】
- 客观第三方新闻播报员，不是评论员
- 严格基于素材中的「原文」字段提取事实（原文 = RSS 全文）
- 如果素材只有「内容」字段而没有「原文」，则基于「内容」播报
- 严禁编造素材中未提及的任何信息
- 严禁加入训练记忆中的知识

【撰写要求】
- 客观第三方视角：禁止主观评价词、禁止揣测动机
- 篇幅分配（总时长不超过 5 分钟）：
  - 重要新闻（突发、重大政策）：120-180 字
  - 一般新闻：60-100 字
- 简洁过渡：用事实性语句衔接，禁止模板句

【格式要求】
- 纯文本口播稿，禁止任何标记符号（括号、markdown等）
- TTS-safe：输出直接用于语音合成
```

> **Design Changes**:
> - Original article text (`original_text`) now passed as primary source material, not just the 2-sentence summary
> - `compress_summaries()` removed - no longer needed since original text provides sufficient detail
> - Importance-based length allocation: LLM decides which stories deserve more words
> - Strict objective tone: no subjective evaluation, no motive speculation

---

## 4. Nexus Synchronization

**Goal**: Store the final artifact and make it available to the Frontend.

```python
payload = {
    "title": generated_title,
    "summary": script_text,
    "audio_url": ..., # Nexus handles upload
    "sources": [s.url for s in clusters],
    "duration": mp3_duration
}

nexus_client.push_item(payload, mp3_file)
```
