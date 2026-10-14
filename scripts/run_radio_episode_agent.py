#!/usr/bin/env python3
import os
import sys
import ssl
import json
import urllib.request
import urllib.error
import time
from datetime import datetime
import tomllib

# Configuration & environment variables
NEXUS_URL = os.environ.get("NEXUS_URL", "https://news.hackerlife.fun:8443")
AGENT_ID = os.environ.get("AGENT_ID", "freshloop-radio-evening")
JOB_TYPE = os.environ.get("JOB_TYPE", "radio_episode")
RUN_DATE = os.environ.get("RUN_DATE") or datetime.now().strftime("%Y-%m-%d")
RUN_WINDOW = os.environ.get("RUN_WINDOW", "evening")
USER_ID = os.environ.get("USER_ID", "default")
MAX_SOURCE_ITEMS = int(os.environ.get("MAX_SOURCE_ITEMS", "12"))

# Deterministic JOB_ID
JOB_ID = f"{AGENT_ID}:{JOB_TYPE}:{RUN_DATE}:{RUN_WINDOW}"

# SSL Context for HTTPS requests
ssl_context = ssl._create_unverified_context()

# Read NEXUS_KEY from config.toml
def load_nexus_key():
    config_path = "/Users/jinliang/workspace/hacker-s-news/config.toml"
    with open(config_path, "rb") as f:
        config = tomllib.load(f)
    return config["nexus"]["auth_key"]

NEXUS_KEY = load_nexus_key()

def get_chinese_date_and_weekday(date_str):
    dt = datetime.strptime(date_str, "%Y-%m-%d")
    digits = {"0": "零", "1": "一", "2": "二", "3": "三", "4": "四", "5": "五", "6": "六", "7": "七", "8": "八", "9": "九"}
    year_ch = "".join(digits[c] for c in str(dt.year))
    month_ch = digits[str(dt.month)] if dt.month < 10 else "十" + (digits[str(dt.month)[1]] if dt.month > 10 else "")
    day = dt.day
    if day < 10:
        day_ch = digits[str(day)]
    elif day == 10:
        day_ch = "十"
    elif day < 20:
        day_ch = "十" + digits[str(day)[1]]
    elif day == 20:
        day_ch = "二十"
    elif day < 30:
        day_ch = "二十" + digits[str(day)[1]]
    elif day == 30:
        day_ch = "三十"
    else:
        day_ch = "三十" + digits[str(day)[1]]
        
    weekdays = ["星期一", "星期二", "星期三", "星期四", "星期五", "星期六", "星期日"]
    weekday_ch = weekdays[dt.weekday()]
    
    simple_ch = f"{dt.year}年{dt.month}月{dt.day}日"
    spoken_ch = f"{year_ch}年{month_ch}月{day_ch}日，{weekday_ch}"
    
    return simple_ch, spoken_ch, weekday_ch


def call_api(path, method="GET", payload=None):
    url = f"{NEXUS_URL}{path}"
    req = urllib.request.Request(url, method=method)
    req.add_header("X-NEXUS-KEY", NEXUS_KEY)
    if payload is not None:
        req.add_header("Content-Type", "application/json")
        data = json.dumps(payload).encode('utf-8')
    else:
        data = None
        
    delays = [5, 15, 45]
    for attempt in range(4):
        try:
            with urllib.request.urlopen(req, data=data, context=ssl_context, timeout=30) as resp:
                body = resp.read().decode('utf-8')
                return resp.status, body
        except urllib.error.HTTPError as e:
            status = e.code
            body = e.read().decode('utf-8')
            if status >= 500:
                if attempt < len(delays):
                    time.sleep(delays[attempt])
                    continue
            return status, body
        except Exception as e:
            if attempt < len(delays):
                time.sleep(delays[attempt])
                continue
            raise e
    raise RuntimeError("Max retries exceeded")

def check_capabilities():
    status, body = call_api("/api/internal/agent/capabilities")
    if status != 200:
        print(f"Failed to check capabilities, status: {status}", file=sys.stderr)
        sys.exit(1)
    data = json.loads(body)
    if JOB_TYPE not in data.get("agent_job_types", []):
        print(f"Nexus does not support job type: {JOB_TYPE}", file=sys.stderr)
        sys.exit(1)

def call_llm(prompt):
    LLM_URL = "http://127.0.0.1:1234/v1/chat/completions"
    LLM_MODEL = "qwen/qwen3.6-35b-a3b"
    
    payload = {
        "model": LLM_MODEL,
        "messages": [
            {"role": "system", "content": "You are an expert AI assistant that processes articles into structured database payloads. You strictly output raw JSON without markdown wrapping or code blocks."},
            {"role": "user", "content": prompt}
        ],
        "temperature": 0.2
    }
    
    req = urllib.request.Request(LLM_URL)
    req.add_header("Content-Type", "application/json")
    
    try:
        with urllib.request.urlopen(req, data=json.dumps(payload).encode('utf-8'), timeout=600) as resp:
            resp_body = resp.read().decode('utf-8')
            data = json.loads(resp_body)
            return data["choices"][0]["message"]["content"].strip()
    except Exception as e:
        raise RuntimeError(f"LLM API call failed: {e}")

def clean_and_parse_json(raw_text):
    cleaned = raw_text.strip()
    if cleaned.startswith("```"):
        first_nl = cleaned.find("\n")
        if first_nl != -1:
            cleaned = cleaned[first_nl:]
        if cleaned.endswith("```"):
            cleaned = cleaned[:-3]
        cleaned = cleaned.strip()
        
    start_idx = cleaned.find("{")
    end_idx = cleaned.rfind("}")
    if start_idx != -1 and end_idx != -1:
        cleaned = cleaned[start_idx:end_idx+1]
    return json.loads(cleaned)

def validate_artifact(artifact):
    title = artifact.get("title")
    category = artifact.get("category")
    script = artifact.get("script")
    
    if not title or not isinstance(title, str) or not title.strip():
        raise ValueError("title is required and must be non-empty")
    if len(title) > 500:
        raise ValueError("title exceeds 500 characters")
        
    if not category or not isinstance(category, str) or not category.strip():
        raise ValueError("category is required and must be non-empty")
        
    if not script or not isinstance(script, str) or not script.strip():
        raise ValueError("script is required and must be non-empty")
    if len(script.encode('utf-8')) > 80000:
        raise ValueError("script exceeds 80,000 bytes")
        
    original_url = artifact.get("original_url")
    if original_url:
        if not (original_url.startswith("http://") or original_url.startswith("https://")):
            raise ValueError(f"invalid original_url format: {original_url}")
            
    sources = artifact.get("sources", [])
    if sources:
        for s in sources:
            url = s.get("url")
            if not url or not (url.startswith("http://") or url.startswith("https://")):
                raise ValueError(f"invalid source url format: {url}")
                
    artifact_str = json.dumps(artifact)
    if len(artifact_str.encode('utf-8')) > 80000:
        raise ValueError("Artifact size exceeds 80,000 bytes")

def main():
    print("Checking capabilities...", file=sys.stderr)
    check_capabilities()
    
    # Create or reuse job
    print(f"Ensuring job {JOB_ID} exists...", file=sys.stderr)
    create_payload = {
        "id": JOB_ID,
        "job_type": JOB_TYPE,
        "priority": 0,
        "max_attempts": 3,
        "context": {
            "run_date": RUN_DATE,
            "run_window": RUN_WINDOW,
            "agent_id": AGENT_ID
        },
        "input": {}
    }
    c_status, c_body = call_api("/api/internal/agent/jobs", method="POST", payload=create_payload)
    if c_status not in [201, 200, 500]:
        print(f"Failed to create job, status: {c_status}, body: {c_body}", file=sys.stderr)
        sys.exit(1)
        
    # Lease the job
    print("Leasing job...", file=sys.stderr)
    lease_payload = {
        "agent_id": AGENT_ID,
        "job_types": [JOB_TYPE],
        "lease_seconds": 1800
    }
    l_status, l_body = call_api("/api/internal/agent/jobs/lease", method="POST", payload=lease_payload)
    if l_status != 200:
        print(f"Failed to lease job, status: {l_status}", file=sys.stderr)
        sys.exit(1)
        
    lease_data = json.loads(l_body)
    job = lease_data.get("job")
    if job is None:
        print("FreshLoop scheduled agent result")
        print(f"job_type: {JOB_TYPE}")
        print("job_id: none")
        print("status: no_job")
        print("result_ref: none")
        print("voice_job_ids: none")
        print("notes: no leased job available")
        sys.exit(0)
        
    leased_job_id = job.get("id")
    lease_token = job.get("lease_token")
    
    if not lease_token:
        print(f"Lease token is missing for job {leased_job_id}", file=sys.stderr)
        print("FreshLoop scheduled agent result")
        print(f"job_type: {JOB_TYPE}")
        print(f"job_id: {leased_job_id}")
        print("status: failed")
        print("result_ref: none")
        print("voice_job_ids: none")
        print("notes: leased job missing lease_token")
        sys.exit(1)
        
    if leased_job_id != JOB_ID:
        print(f"Leased job ID {leased_job_id} does not match deterministic JOB_ID {JOB_ID}", file=sys.stderr)
        print("FreshLoop scheduled agent result")
        print(f"job_type: {JOB_TYPE}")
        print(f"job_id: {leased_job_id}")
        print("status: failed")
        print("result_ref: none")
        print("voice_job_ids: none")
        print(f"notes: leased job id mismatch")
        sys.exit(1)
        
    # Fetch job context
    print(f"Fetching context for job {leased_job_id}...", file=sys.stderr)
    ctx_status, ctx_body = call_api(f"/api/internal/agent/jobs/{leased_job_id}/context")
    if ctx_status != 200:
        print(f"Failed to fetch job context, status: {ctx_status}", file=sys.stderr)
        call_api(f"/api/internal/agent/jobs/{leased_job_id}/fail", method="POST", payload={
            "lease_token": lease_token,
            "error": "Failed to fetch job context"
        })
        sys.exit(1)
        
    # Get candidate source articles
    print(f"Retrieving candidate articles up to {MAX_SOURCE_ITEMS} items...", file=sys.stderr)
    f_status, f_body = call_api(f"/api/feed/items?product_line=curated_feed&item_type=article&limit={MAX_SOURCE_ITEMS}")
    if f_status != 200:
        error_msg = f"Failed to retrieve feed items, status: {f_status}"
        print(error_msg, file=sys.stderr)
        call_api(f"/api/internal/agent/jobs/{leased_job_id}/fail", method="POST", payload={
            "lease_token": lease_token,
            "error": error_msg
        })
        sys.exit(1)
        
    feed_items = json.loads(f_body)
    if not feed_items:
        error_msg = "No curated feed items found to summarize"
        print(error_msg, file=sys.stderr)
        call_api(f"/api/internal/agent/jobs/{leased_job_id}/fail", method="POST", payload={
            "lease_token": lease_token,
            "error": error_msg
        })
        sys.exit(1)
        
    # Let's read the full content of the top 4 articles (since radio episodes are usually compact summaries)
    selected_items = feed_items[:4]
    articles_data = []
    
    for item in selected_items:
        item_id = item["id"]
        c_status, c_body = call_api(f"/api/feed/items/{item_id}/content")
        if c_status == 200:
            content_data = json.loads(c_body)
            articles_data.append({
                "id": item["id"],
                "title": item["title"],
                "original_url": item["original_url"],
                "tags": item.get("tags", ""),
                "compressed_markdown": content_data.get("compressed_markdown") or "",
                "key_points": content_data.get("key_points") or []
            })
            
    if not articles_data:
        error_msg = "Failed to retrieve content for any selected article"
        print(error_msg, file=sys.stderr)
        call_api(f"/api/internal/agent/jobs/{leased_job_id}/fail", method="POST", payload={
            "lease_token": lease_token,
            "error": error_msg
        })
        sys.exit(1)
        
    # Generate Radio Episode using LLM
    print("Generating radio episode payload via LLM...", file=sys.stderr)
    
    simple_date_ch, spoken_date_ch, weekday_ch = get_chinese_date_and_weekday(RUN_DATE)
    publish_time_val = int(datetime.strptime(RUN_DATE, "%Y-%m-%d").timestamp()) + 18 * 3600

    articles_summary_prompt = ""
    for idx, art in enumerate(articles_data):
        articles_summary_prompt += f"--- 文章 {idx+1} ---\n"
        articles_summary_prompt += f"标题: {art['title']}\n"
        articles_summary_prompt += f"链接: {art['original_url']}\n"
        articles_summary_prompt += f"要点:\n"
        if isinstance(art["key_points"], list):
            for kp in art["key_points"]:
                articles_summary_prompt += f" - {kp}\n"
        else:
            articles_summary_prompt += f" - {art['key_points']}\n"
        articles_summary_prompt += f"内容总结:\n{art['compressed_markdown']}\n\n"
        
    prompt = f"""你是一个专业的内容生成与播音稿写作助手。你需要将以下几篇精选的科技文章，融合成一篇 FreshLoop 晚间科技要闻电台播音稿。

今天日期的上下文：今天是 {simple_date_ch}。

待播报的科技新闻文章数据：
{articles_summary_prompt}

请生成满足以下 JSON Schema 的结果。输出必须是唯一合法的 JSON 字符串本身，不要用 markdown 代码块 (如 ```json) 包裹，也不要包含任何前导或后随文字。

JSON Schema:
{{
  "title": "晚间科技要闻简报标题，需大方、概括核心，例如 'FreshLoop 晚间科技要闻简报 ({RUN_DATE})'",
  "category": "播报所属类别，例如 '科技要闻' 或 '热点科技'",
  "script": "完整的中文口语化播音稿（ spoken script ）。字数需在 600 - 1000 字之间。注意：你必须以 '听众朋友们，傍晚好。这里是鲜回路新闻广播。今天是{spoken_date_ch}。我们将为您梳理今天的科技行业要闻。' 开头。然后顺畅自然地引出并播报上面提供的每一篇新闻的核心内容与启发。各条新闻之间的过渡必须顺滑，避免生硬地列举（例如：'首先关注...'、'接下来，我们把目光转向...'、'最后关注...' 等口语化过渡词）。稿件听起来必须自然、平实、冷静，具有 FreshLoop 独特的极简科技质感。绝对不要在稿件中加入任何虚假、未确认的日期/工作日/调休描述，不要祝大家周末愉快（今天是{weekday_ch}）。",
  "publish_time": {publish_time_val}, // 设为 {RUN_DATE} 傍晚时段的时间戳（如 {publish_time_val}，即 {RUN_DATE} 18:00:00）
  "original_url": "主新闻或者第一篇新闻的原始链接",
  "tags": ["小写英文或中文标签1", "标签2"],
  "sources": [
    {{
      "url": "这篇新闻的原始链接",
      "title": "新闻原始标题",
      "summary": "为什么这篇新闻重要（中文一句话简述）"
    }}
  ]
}}

约束条件：
1. `script` 必须是高质量的口语化中文播音稿，包含顺畅自然的过渡，严格控制在 600 - 1000 字，且开头必须按照规定句子播报。
2. `sources` 必须包含且仅包含上面播报到的全部 4 篇新闻的真实信息与链接。不要伪造或虚构任何源头信息。
3. 必须输出合法的 JSON，确保所有 JSON 字符转义（例如换行符转为 \\n）正确无误。
"""
    try:
        raw_response = call_llm(prompt)
        artifact = clean_and_parse_json(raw_response)
        
        # Keep sources truthful & preserve original URLs
        # Let's map LLM outputs to original items or enforce original URLs if needed
        # We can just double check if the URLs match or fallback to real values
        real_sources = []
        for idx, art in enumerate(articles_data):
            real_sources.append({
                "url": art["original_url"],
                "title": art["title"],
                "summary": f"收录于今日科技简报，主要关于：{art['title']}"
            })
        artifact["sources"] = real_sources
        artifact["original_url"] = articles_data[0]["original_url"]
        artifact["publish_time"] = int(datetime.strptime(RUN_DATE, "%Y-%m-%d").timestamp()) + 18 * 3600
        
        # Validate locally
        validate_artifact(artifact)
    except Exception as e:
        error_msg = f"Failed to generate or validate artifact: {e}"
        print(error_msg, file=sys.stderr)
        call_api(f"/api/internal/agent/jobs/{leased_job_id}/fail", method="POST", payload={
            "lease_token": lease_token,
            "error": error_msg
        })
        print("FreshLoop scheduled agent result")
        print(f"job_type: {JOB_TYPE}")
        print(f"job_id: {leased_job_id}")
        print("status: failed")
        print("result_ref: none")
        print("voice_job_ids: none")
        print(f"notes: generation or validation failed")
        sys.exit(1)
        
    # Submit artifact
    print("Submitting artifact to Nexus...", file=sys.stderr)
    s_status, s_body = call_api(f"/api/internal/agent/jobs/{leased_job_id}/submit", method="POST", payload={
        "lease_token": lease_token,
        "artifact": artifact
    })
    
    if s_status != 200:
        error_msg = f"Failed to submit artifact, status: {s_status}, body: {s_body}"
        print(error_msg, file=sys.stderr)
        call_api(f"/api/internal/agent/jobs/{leased_job_id}/fail", method="POST", payload={
            "lease_token": lease_token,
            "error": error_msg
        })
        print("FreshLoop scheduled agent result")
        print(f"job_type: {JOB_TYPE}")
        print(f"job_id: {leased_job_id}")
        print("status: failed")
        print("result_ref: none")
        print("voice_job_ids: none")
        print("notes: submit failed")
        sys.exit(1)
        
    submit_res = json.loads(s_body)
    result_ref = submit_res.get("result_ref", "none")
    voice_job_ids = ",".join(submit_res.get("voice_job_ids", [])) or "none"
    
    # Output contract
    print("FreshLoop scheduled agent result")
    print(f"job_type: {JOB_TYPE}")
    print(f"job_id: {leased_job_id}")
    print("status: completed")
    print(f"result_ref: {result_ref}")
    print(f"voice_job_ids: {voice_job_ids}")
    print("notes: radio episode successfully generated and submitted")

if __name__ == "__main__":
    main()
