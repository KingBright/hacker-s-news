#!/usr/bin/env python3
import os
import sys
import glob
import datetime
import re
import json
import urllib.request
import urllib.error

# Configuration
NEXUS_API_URL = "https://news.hackerlife.fun:8443" # Remote Server
NEXUS_AUTH_KEY = "sk-secure-hackerlife-2026"
TRACE_LOG_DIR = os.path.expanduser("~/.freshloop/logs/traces")

def get_recent_items(limit=5):
    """Fetch recent items from Nexus using urllib with Auth."""
    url = f"{NEXUS_API_URL}/api/items?limit={limit}"
    req = urllib.request.Request(url)
    req.add_header("x-api-key", NEXUS_AUTH_KEY)
    req.add_header("User-Agent", "FreshLoop-Extractor/1.0")

    # Create SSL context that ignores self-signed cert errors (just in case for testing)
    import ssl
    ctx = ssl.create_default_context()
    ctx.check_hostname = False
    ctx.verify_mode = ssl.CERT_NONE
    
    try:
        with urllib.request.urlopen(req, context=ctx) as response:
            data = response.read()
            return json.loads(data)
    except urllib.error.URLError as e:
        print(f"Error fetching items from Nexus: {e}")
        return []
    except Exception as e:
        print(f"Unexpected error: {e}")
        return []

def parse_trace_filename(filename):
    """
    Parse filename like trace_20260107_1229_AI_86eafa6b.md
    Returns: (datetime, category, trace_id)
    """
    basename = os.path.basename(filename)
    # Regex for: trace_YYYYMMDD_HHMM_Category_ID.md
    match = re.match(r"trace_(\d{8})_(\d{4})_(.+)_([a-f0-9]+)\.md", basename)
    if match:
        date_str = match.group(1)
        time_str = match.group(2)
        category = match.group(3)
        trace_id = match.group(4)
        
        dt_str = f"{date_str} {time_str}"
        dt = datetime.datetime.strptime(dt_str, "%Y%m%d %H%M")
        return dt, category, trace_id
    return None, None, None

def find_matching_trace(target_item):
    """
    Find the trace file that likely produced this item.
    Matching logic:
    1. Category matches (case-insensitive)
    2. Trace time is BEFORE Item Publish Time
    3. Trace time is closest to Publish Time (within 1 hour window)
    """
    target_category = target_item.get('category', '').replace(' ', '_')
    # Nexus publish_time is Unix timestamp (seconds)
    publish_ts = target_item.get('publish_time')
    if not publish_ts:
        print("Item has no publish_time.")
        return None

    publish_dt = datetime.datetime.fromtimestamp(publish_ts)
    
    print(f"Searching for trace matching: Category='{target_category}', PublishTime={publish_dt}")

    candidates = []
    
    # List all trace files
    trace_files = glob.glob(os.path.join(TRACE_LOG_DIR, "trace_*.md"))
    
    for f in trace_files:
        dt, category, tid = parse_trace_filename(f)
        if not dt:
            continue
            
        # 1. Category Check
        if category.lower() != target_category.lower():
            continue
            
        # 2. Time Check (Trace must be before Push)
        if dt > publish_dt:
            continue
            
        # 3. Window Check (Within 60 mins)
        delta = publish_dt - dt
        if delta.total_seconds() > 3600:
            continue
            
        candidates.append((delta.total_seconds(), f))
        
    if not candidates:
        return None
        
    # Sort by closest time (smallest delta)
    candidates.sort(key=lambda x: x[0])
    return candidates[0][1]

def main():
    print(f"--- FreshLoop Data Journey Extractor ---")
    
    # 1. Get Items
    items = get_recent_items()
    if not items:
        print("No items found in Nexus. Ensure Backend is running at http://localhost:8000")
        sys.exit(1)
        
    print(f"\nRecent Items (Nexus):")
    for i, item in enumerate(items):
        title = item.get('title', 'No Title')
        cat = item.get('category', 'Unknown')
        print(f"[{i}] {title} ({cat})")
        
    # 2. Select
    try:
        selection = input("\nSelect Item index [0]: ").strip()
        idx = int(selection) if selection else 0
        target_item = items[idx]
    except (ValueError, IndexError):
        print("Invalid selection.")
        sys.exit(1)
        
    print(f"\nSelected: {target_item.get('title')}")
    
    # 3. Find Trace
    trace_path = find_matching_trace(target_item)
    
    if trace_path:
        print(f"\n✅ FOUND TRACE FILE: {trace_path}")
        
        # 4. Fetch Full Sources from Nexus (DB)
        print(f"Fetching raw sources for Item {target_item.get('id')}...")
        sources = get_item_sources(target_item.get('id'))
        print(f"✅ Retrieved {len(sources)} source articles from Nexus DB.")
        
        # 5. Parse Trace
        with open(trace_path, 'r') as f:
            trace_content = f.read()
            
        report = generate_story_report(target_item, trace_content, sources)
        
        # Output Report
        print("\n" + "="*80)
        print("GENERATED DATA JOURNEY REPORT")
        print("="*80 + "\n")
        print(report)
        
        # Save to file
        report_filename = f"STORY_REPORT_{target_item.get('id')[:8]}.md"
        with open(report_filename, "w") as f:
            f.write(report)
        print(f"\n✅ Report saved to: {os.path.abspath(report_filename)}")
        
    else:
        print("\n❌ NO MATCHING TRACE FILE FOUND.")
        print(f"Checked directory: {TRACE_LOG_DIR}")
        print("Ensure 'cortex' backend was running locally and generated this item.")

def get_item_sources(item_id):
    """Fetch original sources for an item from Nexus DB."""
    if not item_id:
        return []
    url = f"{NEXUS_API_URL}/api/items/{item_id}/sources"
    
    # SSL Context
    import ssl
    ctx = ssl.create_default_context()
    ctx.check_hostname = False
    ctx.verify_mode = ssl.CERT_NONE
    
    try:
        req = urllib.request.Request(url)
        # Note: get_sources is public, but sending auth key doesn't hurt.
        req.add_header("x-api-key", NEXUS_AUTH_KEY)
        
        with urllib.request.urlopen(req, context=ctx) as response:
            data = response.read()
            return json.loads(data)
    except Exception as e:
        print(f"Warning: Failed to fetch sources: {e}")
        return []

def generate_story_report(item, trace_content, sources):
    """
    Reconstruct the story flow: Audio -> Script -> Plan -> Raw.
    """
    
    # --- Helper: Parse Steps ---
    steps = []
    current_step = None
    
    # Regex for step headers: ## 21. Plan Episode Flow (12:44:59)
    step_pattern = re.compile(r"^## (\d+)\. (.+?) \((.+?)\)")
    
    lines = trace_content.split('\n')
    for line in lines:
        match = step_pattern.match(line)
        if match:
            if current_step:
                steps.append(current_step)
            current_step = {
                'id': match.group(1),
                'name': match.group(2).strip(),
                'time': match.group(3),
                'content': []
            }
        elif current_step:
            current_step['content'].append(line)
            
    if current_step:
        steps.append(current_step)
        
    # --- extract data ---
    
    # 1. Final Audio / Metadata
    report = f"# 🎙️ FreshLoop Story Journey: {item.get('title')}\n\n"
    report += f"- **Category**: {item.get('category')}\n"
    report += f"- **Publish Time**: {datetime.datetime.fromtimestamp(item.get('publish_time', 0))}\n"
    report += f"- **Duration**: {item.get('duration_sec')}s\n"
    report += f"- **Nexus ID**: `{item.get('id')}`\n\n"
    
    report += "---\n\n"
    
    # 2. The Final Script (Reconstructed from Segments)
    report += "## 🎬 Chapter 1: The Final Script\n\n"
    report += "> This is the final audio content delivered to the user.\n\n"
    
    # Look for "Segment Writer Result" steps
    script_segments = []
    for s in steps:
        if "Segment Writer Result" in s['name']:
            # Extract LLM Response content
            content_str = "\n".join(s['content'])
            # Simple regex to find content block inside ```text ... ```
            # Note: The trace format has **LLM Response**:\n```text\n(content)\n```
            resp_match = re.search(r"\*\*LLM Response\*\*:\n```text\n(.*?)\n```", content_str, re.DOTALL)
            if resp_match:
               script_segments.append(resp_match.group(1).strip())
    
    if script_segments:
        for i, seg in enumerate(script_segments):
            report += f"### Segment {i+1}\n\n{seg}\n\n"
    else:
        report += "*No script segments found in trace log.*\n\n"
        
        
    report += "---\n\n"
    
    # 3. The Blueprint (Planning)
    report += "## 📐 Chapter 2: The Blueprint (Planning)\n\n"
    report += "> How the AI decided to arrange the stories.\n\n"
    
    plan_step = next((s for s in steps if "Plan Episode Flow" in s['name']), None)
    
    item_map = {} # ID -> Title mapping from Planning Prompt
    
    if plan_step:
        content_str = "\n".join(plan_step['content'])
        
        # Extract Prompt Logic
        prompt_match = re.search(r"\*\*LLM Prompt\*\*:.*?(Role:.*?)```", content_str, re.DOTALL)
        if prompt_match:
            prompt_text = prompt_match.group(1)
            # Extract logic definitions (Principles)
            principles = re.findall(r"(\d+\. \*\*.*?\*\*:.*)", prompt_text)
            if principles:
                report += "### Core Principles Used:\n" + "\n".join([f"- {p}" for p in principles]) + "\n\n"
                
            # Extract ID -> Title Map from prompt
            # Pattern: - 01dcf2a6: [Source] Title
            # We use this to map Short IDs to Titles
            raw_items = re.findall(r"- ([a-f0-9]+): (.*)", prompt_text)
            for rid, rtitle in raw_items:
                item_map[rid] = rtitle.strip()
                
        # Extract JSON Decision
        json_match = re.search(r"\*\*LLM Response\*\*:\n```text\n(\[.*?\])\n```", content_str, re.DOTALL)
        if json_match:
            try:
                order = json.loads(json_match.group(1))
                report += "### Planned Sequence:\n"
                for i, short_id in enumerate(order):
                    title = item_map.get(short_id, "Unknown Item")
                    report += f"{i+1}. **{title}** (`{short_id}`)\n"
            except:
                report += "Failed to parse planning JSON.\n"
    else:
        report += "*Planning step not found in trace.*\n"
        
    report += "\n---\n\n"
    
    # 4. Production Log (Deep Dive)
    report += "## 🏭 Chapter 3: Production Log (Deep Dive)\n\n"
    report += "> Detailed view of sources and transformation for each segment.\n\n"
    
    # We iterate through Segments again, but this time show Inputs -> Draft -> Final
    # Note: Structure is "Segment Writer Result" contains the Prompt (with inputs) and Response (Draft).
    
    writer_steps = [s for s in steps if "Segment Writer Result" in s['name']]
    
    for i, step in enumerate(writer_steps):
        content_str = "\n".join(step['content'])
        
        # Identify Batch Items from Prompt
        report += f"### Segment {i+1} Production\n"
        
        # Extract Prompt Materials
        materials_match = re.search(r"【新闻素材】\n(.*?)\n【核心要求", content_str, re.DOTALL)
        if materials_match:
            materials_raw = materials_match.group(1)
            report += "**Input Sources:**\n"
            
            # Robust Parsing for Multi-line Bullet Points
            # Structure:
            # - Title
            #   摘要: Summary
            #   来源: Source
            
            raw_items = []
            current_item = None
            
            for line in materials_raw.strip().split('\n'):
                line = line.strip()
                if not line: continue
                
                if line.startswith("- "):
                    # Start of new item
                    if current_item:
                         raw_items.append(current_item)
                    current_item = {
                        'title_line': line[2:], # Remove "- "
                        'summary': "",
                        'source': ""
                    }
                elif current_item:
                    # Content of current item
                    if line.startswith("摘要:"):
                        current_item['summary'] += line.replace("摘要:", "").strip()
                    elif line.startswith("来源:"):
                        current_item['source'] += line.replace("来源:", "").strip()
                    else:
                        # Append to summary if not explicitly source? or just generic content
                        current_item['summary'] += " " + line
            
            if current_item:
                raw_items.append(current_item)
                
            # Now Match against DB
            for item in raw_items:
                title = item['title_line']
                # Try cleanup [Tag] from title for matching
                clean_title = re.sub(r"^\[.*?\]\s*", "", title).strip()
                
                matched_source = None
                
                # Check against Nexus Sources
                for ns in sources:
                    ns_title = ns.get('source_title', '') or ''
                    if ns_title and (ns_title in title or clean_title in ns_title or title in ns_title):
                        matched_source = ns
                        break
                        
                if matched_source:
                    report += f"- 🟢 **{matched_source.get('source_title')}**\n"
                    
                    # Use DB Summary if available, else Fallback to Trace Summary
                    db_summary = matched_source.get('source_summary', '').replace('\n', ' ')
                    trace_summary = item['summary']
                    
                    final_summary = db_summary if db_summary and len(db_summary) > len(trace_summary) else trace_summary
                    
                    report += f"  > {final_summary}\n\n"
                    report += f"  *Source: {matched_source.get('source_url')}*\n"
                else:
                    # Fallback: Show data from Trace
                    report += f"- ⚪ **{title}** *(From Log)*\n"
                    if item['summary']:
                        report += f"  > {item['summary']}\n\n"
                    if item['source']:
                        report += f"  *Source: {item['source']}*\n"
            report += "\n"
        
        # Extract Draft/Output
        report += "**LLM Output (Final Draft)**:\n"
        resp_match = re.search(r"\*\*LLM Response\*\*:\n```text\n(.*?)\n```", content_str, re.DOTALL)
        if resp_match:
             report += "```text\n" + resp_match.group(1).strip() + "\n```\n"
        
        report += "\n"

    return report

if __name__ == "__main__":
    main()
