#!/usr/bin/env python3
import os
import sys
import ssl
import json
import urllib.request
import urllib.error
import time
import tomllib

# Configuration & environment variables
NEXUS_URL = os.environ.get("NEXUS_URL", "https://news.hackerlife.fun:8443")
AGENT_ID = os.environ.get("AGENT_ID", "freshloop-loop-morning")
JOB_TYPE = os.environ.get("JOB_TYPE", "loop_preference_extraction")
RUN_DATE = os.environ.get("RUN_DATE", "2026-06-28")
RUN_WINDOW = os.environ.get("RUN_WINDOW", "morning")
USER_ID = os.environ.get("USER_ID", "default")
MAX_SOURCE_ITEMS = int(os.environ.get("MAX_SOURCE_ITEMS", "12"))

# Deterministic JOB_ID
JOB_ID = f"{AGENT_ID}:{JOB_TYPE}:{RUN_DATE}:{RUN_WINDOW}"

# SSL Context for local dev environment
ssl_context = ssl._create_unverified_context()

# Read NEXUS_KEY from config.toml
def load_nexus_key():
    config_path = "/Users/jinliang/workspace/hacker-s-news/config.toml"
    with open(config_path, "rb") as f:
        config = tomllib.load(f)
    return config["nexus"]["auth_key"]

NEXUS_KEY = load_nexus_key()

def call_api(path, method="GET", payload=None, extra_headers=None):
    url = f"{NEXUS_URL}{path}"
    req = urllib.request.Request(url, method=method)
    req.add_header("X-NEXUS-KEY", NEXUS_KEY)
    if payload is not None:
        req.add_header("Content-Type", "application/json")
        data = json.dumps(payload).encode('utf-8')
    else:
        data = None
        
    if extra_headers:
        for k, v in extra_headers.items():
            req.add_header(k, v)
            
    # Try with retries for 5xx and network errors
    # 5xx: Retry up to 3 times with 5s, 15s, 45s delays
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

def check_pending_posts():
    status, body = call_api("/api/internal/loop/posts/pending-preferences?limit=20")
    if status != 200:
        print(f"Failed to get pending posts, status: {status}", file=sys.stderr)
        sys.exit(1)
    return json.loads(body)

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
    post_id = artifact.get("post_id")
    user_id = artifact.get("user_id")
    status = artifact.get("status")
    
    if not post_id or not isinstance(post_id, str) or not post_id.strip():
        raise ValueError("post_id must be a non-empty string")
    if not user_id or not isinstance(user_id, str) or not user_id.strip():
        raise ValueError("user_id must be a non-empty string")
    if status not in ["processed", "skipped", "failed"]:
        raise ValueError(f"invalid status: {status}")
        
    signals = artifact.get("signals", [])
    if not isinstance(signals, list):
        raise ValueError("signals must be a list")
        
    for signal in signals:
        content = signal.get("content")
        signal_type = signal.get("signal_type")
        polarity = signal.get("polarity")
        confidence = signal.get("confidence")
        strength = signal.get("strength")
        evidence = signal.get("evidence")
        
        if not content or not isinstance(content, str) or not content.strip():
            raise ValueError("signal.content must be a non-empty string")
        if not signal_type or not isinstance(signal_type, str) or not signal_type.strip():
            raise ValueError("signal.signal_type must be a non-empty string")
        if polarity not in ["positive", "negative"]:
            raise ValueError(f"invalid signal.polarity: {polarity}")
        if not isinstance(confidence, (int, float)) or not (0.0 <= confidence <= 1.0):
            raise ValueError(f"invalid signal.confidence: {confidence}")
        if not isinstance(strength, (int, float)) or not (0.1 <= strength <= 5.0):
            raise ValueError(f"invalid signal.strength: {strength}")
        if not evidence or not isinstance(evidence, str) or not evidence.strip():
            raise ValueError("signal.evidence must be a non-empty string")
            
    # Size check
    artifact_str = json.dumps(artifact)
    if len(artifact_str.encode('utf-8')) > 80000:
        raise ValueError("Artifact size exceeds 80,000 bytes")

def extract_signals_via_llm(post_body_text, post_id, user_id):
    LLM_URL = "http://127.0.0.1:1234/v1/chat/completions"
    LLM_MODEL = "qwen/qwen3.6-35b-a3b"
    
    prompt = f"""你是一个专业的用户偏好倾向提取助手。你的任务是从用户的文字陈述中，提取出他们对特定主题、内容类型或推荐策略的偏好倾向信号（Preference Signals）。

用户陈述：
\"\"\"
{post_body_text}
\"\"\"

请严格按照以下 JSON Schema 输出提取的倾向信号，不要包含任何 Markdown 格式包裹（如 ```json 等），不要包含任何前导或后随文字，仅输出合法的 JSON 字符串本身。

目标 JSON Schema：
{{
  "post_id": "{post_id}",
  "user_id": "{user_id}",
  "status": "processed", // 如果提取出倾向，则填 "processed"。如果用户陈述中没有表达任何对内容或推荐策略的偏好倾向，填 "skipped"。
  "signals": [
    {{
      "content": "具体的倾向内容描述（中文），例如：'用户偏好关于 AI 智能体和大型语言模型在软件开发中应用的实用深度技术解析文章'",
      "signal_type": "topic_interest", // 必须填 "topic_interest"
      "polarity": "positive", // 正向偏好填 "positive"，负向偏好填 "negative"
      "confidence": 0.90, // 0.0 到 1.0 之间的浮点数，代表你对提取的准确性的信心
      "strength": 1.2, // 0.1 到 5.0 之间的浮点数，代表偏好的强弱度（如“很喜欢/强烈要求”填 1.5-2.0，“不太喜欢/完全不想看”填 1.2-1.5，“可有可无/普通提及”填 0.8-1.0）
      "evidence": "从用户陈述中直接摘录或转述的支持该倾向的具体言论（中文）"
    }}
  ]
}}

约束条件：
1. 偏好信号只能是软权重倾向，改变内容推荐的倾向性，而不是强行封禁某类主题。
2. 捕获时间敏感的词汇，如“最近”、“这个月”、“目前”等，放入 content 或 evidence 中。
3. 必须输出合法的 JSON，确保所有括号、逗号和引号都正确关闭。
4. 如果未提取出任何有效倾向，"signals" 数组填空 []，且 "status" 填 "skipped"。
"""

    payload = {
        "model": LLM_MODEL,
        "messages": [
            {"role": "system", "content": "You are a precise JSON extractor. Only output valid raw JSON matching the schema."},
            {"role": "user", "content": prompt}
        ],
        "temperature": 0.1
    }
    
    req = urllib.request.Request(LLM_URL)
    req.add_header("Content-Type", "application/json")
    
    try:
        with urllib.request.urlopen(req, data=json.dumps(payload).encode('utf-8'), timeout=90) as resp:
            resp_body = resp.read().decode('utf-8')
            data = json.loads(resp_body)
            content = data["choices"][0]["message"]["content"].strip()
            return content
    except Exception as e:
        raise RuntimeError(f"LLM API call failed: {e}")

def main():
    # 1. Check capabilities
    check_capabilities()
    
    # 2. Check pending posts first (optimization to avoid leasing if no posts)
    pending_posts = check_pending_posts()
    if not pending_posts:
        print("FreshLoop scheduled agent result")
        print(f"job_type: {JOB_TYPE}")
        print("job_id: none")
        print("status: no_job")
        print("result_ref: none")
        print("voice_job_ids: none")
        print("notes: no pending loop preference posts")
        sys.exit(0)
        
    # 3. Create or reuse the job
    job_data = {
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
    
    status, body = call_api("/api/internal/agent/jobs", method="POST", payload=job_data)
    if status not in [201, 409, 500]:
        print(f"Failed to create job, status: {status}", file=sys.stderr)
        sys.exit(1)
        
    # 4. Lease job
    lease_payload = {
        "agent_id": AGENT_ID,
        "job_types": [JOB_TYPE],
        "lease_seconds": 1800
    }
    status, body = call_api("/api/internal/agent/jobs/lease", method="POST", payload=lease_payload)
    if status != 200:
        print(f"Failed to lease job, status: {status}", file=sys.stderr)
        sys.exit(1)
        
    lease_data = json.loads(body)
    leased_job = lease_data.get("job")
    if not leased_job:
        print("FreshLoop scheduled agent result")
        print(f"job_type: {JOB_TYPE}")
        print("job_id: none")
        print("status: no_job")
        print("result_ref: none")
        print("voice_job_ids: none")
        print("notes: no leased job available")
        sys.exit(0)
        
    leased_job_id = leased_job.get("id")
    lease_token = leased_job.get("lease_token")
    
    if not lease_token:
        print("leased job missing lease_token", file=sys.stderr)
        sys.exit(1)
        
    # 5. Verify leased job id matches JOB_ID
    if leased_job_id != JOB_ID:
        print(f"Leased job ID {leased_job_id} does not match expected {JOB_ID}.", file=sys.stderr)
        print("FreshLoop scheduled agent result")
        print(f"job_type: {JOB_TYPE}")
        print(f"job_id: {leased_job_id}")
        print("status: failed")
        print("result_ref: none")
        print("voice_job_ids: none")
        print("notes: leased job id mismatch")
        sys.exit(1)
        
    # 6. Fetch target post (input prioritised, fallback to pending)
    target_post = None
    input_data = leased_job.get("input", {}) or {}
    input_post_id = input_data.get("post_id")
    input_user_id = input_data.get("user_id")
    
    if input_post_id and input_user_id:
        status, post_body = call_api(f"/api/loop/posts/{input_post_id}")
        if status == 200:
            target_post = json.loads(post_body)
            
    if not target_post and pending_posts:
        target_post = pending_posts[0]
        
    if not target_post:
        error_msg = "no pending loop preference posts after leasing"
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
        print(f"notes: {error_msg}")
        sys.exit(1)
        
    post_id = target_post.get("id")
    user_id = target_post.get("user_id")
    post_body_text = target_post.get("body", "")
    
    # 7. Extract signals via LLM and validate
    try:
        llm_output = extract_signals_via_llm(post_body_text, post_id, user_id)
        artifact = clean_and_parse_json(llm_output)
        validate_artifact(artifact)
    except Exception as e:
        error_msg = f"Extraction or validation failed: {str(e)}"
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
        print(f"notes: {error_msg}")
        sys.exit(1)
        
    # 8. Submit artifact
    submit_payload = {
        "lease_token": lease_token,
        "artifact": artifact
    }
    
    status, submit_body = call_api(f"/api/internal/agent/jobs/{leased_job_id}/submit", method="POST", payload=submit_payload)
    if status != 200:
        error_msg = f"Failed to submit artifact, status: {status}, body: {submit_body}"
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
        print(f"notes: {error_msg}")
        sys.exit(1)
        
    submit_res = json.loads(submit_body)
    result_ref = submit_res.get("result_ref", "none")
    
    print("FreshLoop scheduled agent result")
    print(f"job_type: {JOB_TYPE}")
    print(f"job_id: {leased_job_id}")
    print("status: completed")
    print(f"result_ref: {result_ref}")
    print("voice_job_ids: none")
    print("notes: loop preferences extracted and submitted successfully")

if __name__ == "__main__":
    main()
