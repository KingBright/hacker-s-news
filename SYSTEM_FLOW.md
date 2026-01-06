# System Control Flow & Intelligence Pipeline (Cortex Core)

This document provides a technical deep-dive into the control flow of the Cortex engine, using pseudo-code to illustrate the decision-making process and detailing the exact LLM prompts used at each intelligence gate.

## 1. Ingestion & Deduplication Loop (`process_rss_sources`)

**Goal**: Convert raw RSS XML into clean, deduplicated `NewsCluster` candidates.

```python
for source in rss_sources:
    if source.url in cache_db: continue  # L0 Deduplication (URL exact match)

    xml_items = fetch_rss(source.url)
    
    for item in xml_items:
        # L1: Keyword Filtering (Blocklist)
        if matches_blocklist(item.title): continue
        
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

**Goal**: Turn a cluster of related articles into a single, high-density "Executive Summary" for the broadcast.

```python
def process_cluster(cluster):
    # Strategy: Merge multiple sources into one truth
    if cluster.has_multiple_items():
        # LLM Synthesis
        merged_summary = llm_merge_items(cluster.items)
        
        # Quality Control
        review_result = review_summary(merged_summary)
        if not review_result.passed:
             merged_summary = llm_fix_summary(merged_summary, review_result.critique)
             
        cluster.final_summary = merged_summary
    
    # Check for Updates (if topic was broadcasted before)
    if topic_registry.has_seen(cluster.topic_id):
        update_info = check_for_updates(cluster.final_summary, previous_broadcast)
        if update_info.is_significant:
            mark_as_update(cluster)
        else:
            discard(cluster) # Old news
```

### 🧠 LLM Gate: Merge & Synthesize
**Function**: `llm_merge_items`
**Prompt**:
```text
Role: Senior Intelligence Analyst (资深情报分析师)。
任务：Synthesize (综合) 多源信息，输出一份高质量简报。

策略：
- **硬新闻/财经**：准确性第一，保留数字/日期/5W1H。
- **软新闻/观点**：捕捉论点和氛围。
- **冲突信息**：如果来源说法不一，请注明（"据A报道...而B则称..."）。

输入数据：
{raw_items_list}

输出格式 (JSON):
{
  "title": "...",
  "summary": "..."
}
```

---

## 3. Episode Orchestration ("Smart Flow") (`produce_episode`)

**Goal**: Convert a list of isolated clusters -> Coherent 15-minute Podcast Script.

```python
def produce_episode(clusters, category):
    # Step A: Narrative Planning
    # LLM reorders items to create a story arc (e.g., Heavy -> Light, or Thematic grouping)
    sorted_order = plan_episode_flow(clusters)
    sorted_clusters = reorder(clusters, sorted_order)
    
    # Step B: Recursive Segmentation
    # Generate script in chunks (4 items per chunk) to maintain context window & coherence
    script_segments = generate_segment(
        items=sorted_clusters,
        index=0,
        prev_context="Opening Greeting..."
    )
    
    final_script = join(script_segments)
    
    # Step C: Audio Production
    wav_bytes = tts_engine.speak(final_script)
    mp3_bytes = convert_to_mp3(wav_bytes)
    
    return final_script, mp3_bytes
```

### 🧠 LLM Gate: Narrative Planning
**Function**: `plan_episode_flow`
**Prompt**:
```text
Role: Showrunner/Producer (总策划)。
任务：编排这期 15 分钟节目的 Narrative Arc (叙事弧线)。

原则：
1. **黄金开头 (The Hook)**：把最重磅、最吸引眼球的新闻放在第一位。
2. **主题聚合 (Thematic Blocks)**：相关新闻成组（如“AI巨头混战”、“中东局势”）。
3. **节奏感 (Pacing)**：硬新闻和软故事交替，或者由重到轻。
4. **Kicker (压轴)**：把最有趣、最轻松或最令人惊讶的故事放在最后。

待排序新闻：
{item_list}

输出格式（仅JSON数组，包含重排后的ID）：
[3, 1, 4, 2, 5]
```

### 🧠 LLM Gate: Segment Generation
**Function**: `generate_segment` (Recursive)
**Prompt**:
```text
Role: Host of 'FreshLoop' (顶流播客主持人)。
频道: {category}
人设: {host_name} (幽默/犀利/温暖)。
节日: {holiday_context}

【当前任务】
接住上文语音流（"{prev_context}"），播报本段新闻。

【新闻素材】
{content_block}

【核心要求】：
1. **交流感**：使用第二人称（你），多用反问句、感叹句。用“signposting”技巧引导听众。
2. **逻辑串联**：严禁呆板的“首先、其次”。用内在逻辑（因果、对比、层递）把新闻串起来。
3. **校对**：输出必须是【终稿】，绝不允许错别字。
```

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
