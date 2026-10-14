#!/usr/bin/env python3
import os
import sys
import ssl
import json
import urllib.request
import urllib.parse
import urllib.error
import time
import re
from datetime import datetime
import calendar
import tomllib
import xml.etree.ElementTree as ET
from html.parser import HTMLParser
from html import unescape
import concurrent.futures

# Configuration & environment variables
NEXUS_URL = os.environ.get("NEXUS_URL", "https://news.hackerlife.fun:8443")
AGENT_ID = os.environ.get("AGENT_ID", "freshloop-reading-morning")
JOB_TYPE = os.environ.get("JOB_TYPE", "reading_article")
RUN_DATE = os.environ.get("RUN_DATE", "2026-06-29")
RUN_WINDOW = os.environ.get("RUN_WINDOW", "morning")
USER_ID = os.environ.get("USER_ID", "default")
MAX_SOURCE_ITEMS = int(os.environ.get("MAX_SOURCE_ITEMS", "12"))

# Deterministic JOB_ID
JOB_ID = f"{AGENT_ID}:{JOB_TYPE}:{RUN_DATE}:{RUN_WINDOW}"

# SSL Context for local dev environment and HTTPS requests
ssl_context = ssl._create_unverified_context()

class HTMLToMarkdown(HTMLParser):
    def __init__(self):
        super().__init__()
        self.result = []
        self.links = []
        self.in_pre = False
        self.in_code = False
        self.list_stack = []
        self.ol_counters = []

    def handle_starttag(self, tag, attrs):
        attrs_dict = dict(attrs)
        if tag == 'p':
            self.result.append('\n\n')
        elif tag in ['strong', 'b']:
            self.result.append('**')
        elif tag in ['em', 'i']:
            self.result.append('*')
        elif tag == 'a':
            href = attrs_dict.get('href', '')
            self.result.append('[')
            self.links.append(href)
        elif tag == 'ul':
            self.list_stack.append('ul')
            self.result.append('\n')
        elif tag == 'ol':
            self.list_stack.append('ol')
            self.ol_counters.append(1)
            self.result.append('\n')
        elif tag == 'li':
            if self.list_stack and self.list_stack[-1] == 'ol':
                count = self.ol_counters[-1]
                self.result.append(f'\n{count}. ')
                self.ol_counters[-1] += 1
            else:
                self.result.append('\n- ')
        elif tag in ['h1', 'h2', 'h3', 'h4', 'h5', 'h6']:
            level = int(tag[1])
            self.result.append(f'\n\n{"#" * level} ')
        elif tag == 'pre':
            self.in_pre = True
            self.result.append('\n```\n')
        elif tag == 'code':
            self.in_code = True
            if not self.in_pre:
                self.result.append('`')
        elif tag == 'br':
            self.result.append('\n')

    def handle_endtag(self, tag):
        if tag in ['strong', 'b']:
            self.result.append('**')
        elif tag in ['em', 'i']:
            self.result.append('*')
        elif tag == 'a':
            if self.links:
                href = self.links.pop()
                self.result.append(f']({href})')
            else:
                self.result.append(']')
        elif tag == 'ul':
            if self.list_stack:
                self.list_stack.pop()
            self.result.append('\n')
        elif tag == 'ol':
            if self.list_stack:
                self.list_stack.pop()
            if self.ol_counters:
                self.ol_counters.pop()
            self.result.append('\n')
        elif tag == 'pre':
            self.in_pre = False
            self.result.append('\n```\n')
        elif tag == 'code':
            self.in_code = False
            if not self.in_pre:
                self.result.append('`')

    def handle_data(self, data):
        self.result.append(data)

    def get_markdown(self):
        text = ''.join(self.result)
        text = re.sub(r'\n{3,}', '\n\n', text)
        return unescape(text.strip())

class HTMLToPlainText(HTMLParser):
    def __init__(self):
        super().__init__()
        self.result = []

    def handle_data(self, data):
        self.result.append(data)

    def get_text(self):
        text = ''.join(self.result)
        text = re.sub(r'\s+', ' ', text)
        return unescape(text.strip())

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

def fetch_feed_xml(url):
    # Try direct first with User-Agent, then try with local proxy
    headers = {
        'User-Agent': 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36'
    }
    
    # Direct fetch
    try:
        req = urllib.request.Request(url, headers=headers)
        with urllib.request.urlopen(req, context=ssl_context, timeout=20) as resp:
            return resp.read().decode('utf-8')
    except Exception as e:
        print(f"Direct fetch of feed failed: {e}. Retrying with proxy...", file=sys.stderr)
        
    # Proxy fallback
    try:
        proxy_handler = urllib.request.ProxyHandler({
            'http': 'http://127.0.0.1:8228',
            'https': 'http://127.0.0.1:8228'
        })
        opener = urllib.request.build_opener(proxy_handler)
        req = urllib.request.Request(url, headers=headers)
        with opener.open(req, timeout=20) as resp:
            return resp.read().decode('utf-8')
    except Exception as e:
        raise RuntimeError(f"Failed to fetch feed {url} (both direct and proxy): {e}")

def parse_iso_to_epoch(iso_str):
    if not iso_str:
        return int(time.time())
    if iso_str.endswith('Z'):
        iso_str = iso_str[:-1] + '+00:00'
    try:
        dt = datetime.fromisoformat(iso_str.split('.')[0])
        return int(calendar.timegm(dt.utctimetuple()))
    except Exception:
        try:
            dt = datetime.strptime(iso_str.split('+')[0].split('-')[0].strip(), "%Y-%m-%dT%H:%M:%S")
            return int(calendar.timegm(dt.utctimetuple()))
        except Exception:
            return int(time.time())

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

def call_llm(prompt):
    LLM_URL = "http://127.0.0.1:1234/v1/chat/completions"
    LLM_MODEL = "qwen/qwen3.6-35b-a3b"
    
    payload = {
        "model": LLM_MODEL,
        "messages": [
            {"role": "system", "content": "You are an expert AI assistant that processes articles into structured database payloads. You strictly output raw JSON without markdown wrapping or code blocks."},
            {"role": "user", "content": prompt}
        ],
        "temperature": 0.1
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

def generate_article_metadata(title, entry_url, markdown_content, personalization_context=None, source_name="Unknown Source"):
    personalization_block = ""
    if personalization_context:
        personalization_block = f"""用户偏好上下文:
{personalization_context}

个性化使用规则:
- 用这些偏好调整 should_publish 和 quality_score，优先保留用户明确关注、正在研究、反复正向评价的主题。
- 对用户明确排除、反复负面评价、低信号来源或泛泛炒作内容，要更严格降权或不发布。
- 个性化只影响选择、优先级和主题标签；不要因为用户偏好改写作者观点。
- compressed_markdown、audio_script 和 key_points 必须仍然忠实于正文，只能呈现作者在文中表达的信息。

"""

    prompt = f"""你是 FreshLoop 精选阅读频道的信息压缩编辑。请判断这篇订阅文章是否进入每日精选，并生成忠实于作者原意的中文压缩稿和单独收听稿。

来源: {source_name}
标题: {title}
URL: {entry_url}
正文:
{markdown_content}

{personalization_block}要求:
1. quality_score 用 0-10 评分，重点看原创性、信息密度、长期价值、技术/思想含量；如果有用户偏好上下文，还要评估它和用户当前偏好的贴合度。
2. should_publish 只有在值得读、且没有明显命中用户排除偏好时才为 true。
3. compressed_markdown 必须是信息压缩，不是评论。只还原作者的主张、论证、事实、例子和结论，不写 FreshLoop 的评价，不写“适合谁读”，不写“值得/不值得”，不替作者扩展观点。
4. compressed_markdown 用中文 Markdown，建议结构为：作者主张、论证脉络、关键事实与例子、原文结论。若原文没有某类信息，可省略对应小节。
5. 不要使用 LaTeX、$...$、$$...$$ 或反斜杠数学命令。若必须保留公式，请改写成普通文本，例如 `M = E - e sin E`。
6. audio_script 是给耳朵听的信息压缩版，不朗读原文，不保留 Markdown 符号；保持作者观点顺序，口语自然，2-4 分钟。
7. key_points 给出 3-7 条作者在文中表达的关键观点或事实。
8. 只能使用正文信息，不要编造正文外的信息，不要加入你的判断。

请输出符合以下 JSON 格式的字符串本身，不要包含任何前导或后随文字，也不要用 markdown 代码块 (如 ```json) 包裹。

JSON 格式:
{{
  "should_publish": true,
  "quality_score": 8,
  "title": "中文文章标题（精准、通俗且吸引人，保持原意，不超过200字）",
  "subtitle": "中文副标题（可选，提炼核心主题，如无则为 null）",
  "compressed_markdown": "精炼压缩稿",
  "audio_script": "精简口语化的听众广播稿（spoken script），去除任何书面化符号或括号标记，字数需在 600 - 1000 字之间。注意不要提及任何时间/日期敏感的话语（保持日期安全），直接切入主题。",
  "key_points": [
    "要点一",
    "要点二",
    "要点三"
  ],
  "tags": [
    "标签一",
    "标签二"
  ]
}}
"""
    raw_response = call_llm(prompt)
    artifact = clean_and_parse_json(raw_response)
    return artifact

def validate_artifact(artifact):
    # Required strings
    for field in ["title", "original_url", "reader_markdown", "plain_text", "compressed_markdown", "audio_script"]:
        val = artifact.get(field)
        if not val or not isinstance(val, str) or not val.strip():
            raise ValueError(f"Required field {field} is missing or empty")
            
    # URL checks
    for field in ["original_url", "canonical_url", "source_url"]:
        url = artifact.get(field)
        if url:
            if not (url.startswith("http://") or url.startswith("https://")):
                raise ValueError(f"Invalid URL format for {field}")
                
    score = artifact.get("quality_score")
    if score is not None:
        if not (0 <= score <= 10):
            raise ValueError("quality_score must be between 0 and 10")
            
    # Key points check
    kps = artifact.get("key_points")
    if not kps or not isinstance(kps, list) or len(kps) < 3 or len(kps) > 8:
        raise ValueError("key_points must be a list containing 3 to 8 points")
        
    for kp in kps:
        if not isinstance(kp, str) or not kp.strip():
            raise ValueError("key_points list must not contain empty strings")

def load_config():
    config_path = "/Users/jinliang/workspace/hacker-s-news/config.toml"
    with open(config_path, "rb") as f:
        return tomllib.load(f)

def fetch_url_content_via_urllib(url, config):
    headers = {
        'User-Agent': 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36'
    }
    use_proxy = config.get("curated_feed", {}).get("prefer_proxy", False)
    proxy_url = config.get("http_proxy")
    
    if use_proxy and proxy_url:
        try:
            proxy_handler = urllib.request.ProxyHandler({'http': proxy_url, 'https': proxy_url})
            opener = urllib.request.build_opener(proxy_handler)
            req = urllib.request.Request(url, headers=headers)
            with opener.open(req, timeout=25) as resp:
                return resp.read().decode('utf-8', errors='replace')
        except Exception as e:
            print(f"Proxy fetch of {url} failed: {e}. Trying direct...", file=sys.stderr)
            
    try:
        req = urllib.request.Request(url, headers=headers)
        with urllib.request.urlopen(req, context=ssl_context, timeout=20) as resp:
            return resp.read().decode('utf-8', errors='replace')
    except Exception as e:
        if not use_proxy and proxy_url:
            try:
                print(f"Direct fetch of {url} failed: {e}. Retrying with proxy...", file=sys.stderr)
                proxy_handler = urllib.request.ProxyHandler({'http': proxy_url, 'https': proxy_url})
                opener = urllib.request.build_opener(proxy_handler)
                req = urllib.request.Request(url, headers=headers)
                with opener.open(req, timeout=25) as resp:
                    return resp.read().decode('utf-8', errors='replace')
            except Exception as e2:
                raise RuntimeError(f"Failed to fetch {url} (both direct and proxy): {e2}")
        raise RuntimeError(f"Failed to fetch {url} directly: {e}")

def parse_opml_rss_urls(xml_data):
    # Clean all namespaces and prefixes to prevent ElementTree parse errors
    xml_data_clean = re.compile(r'\sxmlns[^=]*="[^"]*"', re.IGNORECASE).sub('', xml_data)
    xml_data_clean = re.sub(r'<([a-zA-Z0-9._-]+):([a-zA-Z0-9._-]+)', r'<\2', xml_data_clean)
    xml_data_clean = re.sub(r'</([a-zA-Z0-9._-]+):([a-zA-Z0-9._-]+)', r'</\2', xml_data_clean)
    xml_data_clean = re.sub(r'\s([a-zA-Z0-9._-]+):([a-zA-Z0-9._-]+)=', r' \2=', xml_data_clean)
    root = ET.fromstring(xml_data_clean)
    urls = []
    for outline in root.iter('outline'):
        xml_url = outline.attrib.get('xmlUrl') or outline.attrib.get('xmlurl')
        if xml_url:
            title = outline.attrib.get('title') or outline.attrib.get('text')
            urls.append({
                "name": (title or xml_url).strip(),
                "url": xml_url.strip()
            })
    return urls

def parse_feed_entries(xml_data):
    # Clean all namespaces and prefixes to prevent ElementTree parse errors
    xml_data_clean = re.compile(r'\sxmlns[^=]*="[^"]*"', re.IGNORECASE).sub('', xml_data)
    xml_data_clean = re.sub(r'<([a-zA-Z0-9._-]+):([a-zA-Z0-9._-]+)', r'<\2', xml_data_clean)
    xml_data_clean = re.sub(r'</([a-zA-Z0-9._-]+):([a-zA-Z0-9._-]+)', r'</\2', xml_data_clean)
    xml_data_clean = re.sub(r'\s([a-zA-Z0-9._-]+):([a-zA-Z0-9._-]+)=', r' \2=', xml_data_clean)
    root = ET.fromstring(xml_data_clean)
    
    # Try RSS
    channel = root.find('channel')
    if channel is not None:
        source_title = channel.findtext('title') or ""
        entries = []
        for item in channel.findall('item'):
            title = item.findtext('title') or ""
            link = item.findtext('link') or ""
            description = item.findtext('description') or ""
            content_encoded = None
            for child in item:
                if 'content' in child.tag or 'encoded' in child.tag:
                    content_encoded = child.text
            if content_encoded and len(content_encoded) > len(description):
                description = content_encoded
            pub_date = item.findtext('pubDate')
            entries.append({
                "title": title.strip(),
                "link": link.strip(),
                "description": description,
                "pub_date": pub_date,
                "source_name": source_title.strip() if source_title else None
            })
        return entries

    # Try Atom
    if root.tag.endswith('feed') or root.find('entry') is not None:
        source_title = root.findtext('title') or ""
        entries = []
        for entry in root.findall('entry'):
            title = entry.findtext('title') or ""
            link = ""
            for l in entry.findall('link'):
                rel = l.attrib.get('rel')
                href = l.attrib.get('href')
                if href:
                    if not rel or rel == 'alternate':
                        link = href
                        break
                    link = href
            summary = entry.findtext('summary') or ""
            content = entry.findtext('content') or ""
            description = content if len(content) > len(summary) else summary
            pub_date = entry.findtext('published') or entry.findtext('updated')
            entries.append({
                "title": title.strip(),
                "link": link.strip(),
                "description": description,
                "pub_date": pub_date,
                "source_name": source_title.strip() if source_title else None
            })
        return entries
    return []

def publish_additional_article(candidate, artifact_data, agent_id, job_type, run_date, run_window, index):
    sub_job_id = f"{agent_id}:{job_type}:{run_date}:{run_window}:sub-{index}"
    print(f"Creating sub-job {sub_job_id} for additional article: {artifact_data['title']}...", file=sys.stderr)
    
    # 1. Create job
    create_payload = {
        "id": sub_job_id,
        "job_type": job_type,
        "priority": 0,
        "context": {},
        "input": {}
    }
    c_status, c_body = call_api("/api/internal/agent/jobs", method="POST", payload=create_payload)
    if c_status not in (200, 201):
        print(f"Sub-job creation status: {c_status}. Proceeding to lease.", file=sys.stderr)
        
    # 2. Lease job
    lease_payload = {
        "agent_id": agent_id,
        "job_types": [job_type],
        "lease_seconds": 1800
    }
    l_status, l_body = call_api("/api/internal/agent/jobs/lease", method="POST", payload=lease_payload)
    if l_status != 200:
        print(f"Failed to lease sub-job, status: {l_status}", file=sys.stderr)
        return False
        
    try:
        lease_info = json.loads(l_body).get("job", {})
        sub_leased_id = lease_info.get("id")
        sub_lease_token = lease_info.get("lease_token")
        
        if not sub_leased_id or not sub_lease_token:
            print("Failed to parse lease details from lease response", file=sys.stderr)
            return False
            
        if sub_leased_id != sub_job_id:
            print(f"Warning: leased job ID {sub_leased_id} does not match expected sub-job ID {sub_job_id}", file=sys.stderr)
            
        # 3. Submit artifact
        submit_payload = {
            "lease_token": sub_lease_token,
            "artifact": artifact_data
        }
        s_status, s_body = call_api(f"/api/internal/agent/jobs/{sub_leased_id}/submit", method="POST", payload=submit_payload)
        if s_status == 200:
            print(f"Successfully published additional article: {artifact_data['title']} via job {sub_leased_id}", file=sys.stderr)
            return True
        else:
            print(f"Failed to submit additional article to {sub_leased_id}, status: {s_status}, body: {s_body}", file=sys.stderr)
            # Fail the sub job so it is not stuck
            call_api(f"/api/internal/agent/jobs/{sub_leased_id}/fail", method="POST", payload={
                "lease_token": sub_lease_token,
                "error": f"Submit failed: {s_body}"
            })
            return False
    except Exception as e:
        print(f"Exception during sub-job processing: {e}", file=sys.stderr)
        return False

def main():
    # 1. Check capabilities
    check_capabilities()
    
    env_lease_token = os.environ.get("LEASE_TOKEN")
    if env_lease_token:
        print(f"Using pre-leased token from environment", file=sys.stderr)
        lease_token = env_lease_token
        leased_job_id = JOB_ID
    else:
        # 2. Create deterministic job
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
        if c_status != 201 and "unique" not in c_body.lower() and "duplicate" not in c_body.lower():
            print(f"Failed to create job, status: {c_status}, body: {c_body}", file=sys.stderr)
            sys.exit(1)
            
        # 3. Lease job
        lease_payload = {
            "agent_id": AGENT_ID,
            "job_types": [JOB_TYPE],
            "lease_seconds": 1800
        }
        l_status, l_body = call_api("/api/internal/agent/jobs/lease", method="POST", payload=lease_payload)
        if l_status != 200:
            print(f"Failed to lease job, status: {l_status}, body: {l_body}", file=sys.stderr)
            sys.exit(1)
            
        lease_data = json.loads(l_body)
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
        
    # 4. Check deterministic ID mismatch
    if leased_job_id != JOB_ID:
        print(f"Mismatch: Leased job ID {leased_job_id} does not match deterministic JOB_ID {JOB_ID}. Stopping.", file=sys.stderr)
        call_api(f"/api/internal/agent/jobs/{leased_job_id}/fail", method="POST", payload={
            "lease_token": lease_token,
            "error": f"Job ID mismatch. Expected {JOB_ID}, got {leased_job_id}."
        })
        print("FreshLoop scheduled agent result")
        print(f"job_type: {JOB_TYPE}")
        print(f"job_id: {leased_job_id}")
        print("status: failed")
        print("result_ref: none")
        print("voice_job_ids: none")
        print("notes: deterministic job id mismatch")
        sys.exit(1)
        
    # 5. Fetch job context
    ctx_status, ctx_body = call_api(f"/api/internal/agent/jobs/{leased_job_id}/context")
    if ctx_status != 200:
        print(f"Failed to fetch job context: {ctx_status}", file=sys.stderr)
        sys.exit(1)
        
    ctx_data = json.loads(ctx_body)
    job_input = ctx_data.get("input", {})
    
    # 6. Load config.toml configuration parameters
    print("Loading global configuration from config.toml...", file=sys.stderr)
    try:
        config = load_config()
    except Exception as e:
        error_msg = f"Failed to load config.toml: {e}"
        print(error_msg, file=sys.stderr)
        call_api(f"/api/internal/agent/jobs/{leased_job_id}/fail", method="POST", payload={
            "lease_token": lease_token,
            "error": error_msg
        })
        sys.exit(1)
        
    curated_config = config.get("curated_feed", {})
    max_age_days = curated_config.get("max_age_days", 90)
    min_quality_score = curated_config.get("min_quality_score", 5)
    proxy_url = config.get("http_proxy")
    
    # 7. Fetch personalization profile context
    pref_config = config.get("loop_preferences", {})
    personalization_user_id = pref_config.get("personalization_user_id") or USER_ID
    personalization_context = None
    if pref_config.get("enabled", True) and personalization_user_id:
        print(f"Fetching memory profile for user: {personalization_user_id}...", file=sys.stderr)
        status, body = call_api(f"/api/internal/memory/profile/{urllib.parse.quote(personalization_user_id)}")
        if status == 200:
            try:
                prof_data = json.loads(body)
                personalization_context = prof_data.get("prompt_context", "").strip()
                if personalization_context:
                    # Truncate context if too long
                    max_chars = pref_config.get("profile_context_max_chars", 3200)
                    personalization_context = personalization_context[:max_chars]
                    print(f"Successfully loaded {len(personalization_context)} chars of user preference context.", file=sys.stderr)
            except Exception as e:
                print(f"Warning: failed to parse memory profile: {e}", file=sys.stderr)
        else:
            print(f"Warning: memory profile endpoint returned status {status}", file=sys.stderr)

    target_url = job_input.get("original_url") or job_input.get("source_url")
    title = None
    pub_time = None
    html_content = None
    source_name = "Unknown Source"
    source_url = "https://karpathy.bearblog.dev"
    tags = ["lifestyle", "hn"]
    
    # If a specific article is targeted
    if target_url:
        print(f"Using user-specified target URL: {target_url}", file=sys.stderr)
        try:
            html_content = fetch_url_content_via_urllib(target_url, config)
            title = target_url.split('/')[-1] or "Curated Article"
            pub_time = int(time.time())
            source_url = "/".join(target_url.split('/')[:3])
            
            # Check if details can be matched from config feeds to give it better metadata
            for feed in curated_config.get("feeds", []):
                if feed.get("url") and "/".join(feed.get("url").split('/')[:3]) in target_url:
                    source_name = feed.get("name", source_name)
                    source_url = feed.get("url")
                    tags = feed.get("tags", tags)
                    break
        except Exception as e:
            error_msg = f"Failed to retrieve specified URL: {e}"
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
            print(f"notes: {error_msg[:100]}")
            sys.exit(1)
            
    else:
        # Crawl all subscription feeds defined in config.toml
        feeds = curated_config.get("feeds", [])
        if not feeds:
            error_msg = "No curation feeds configured in config.toml"
            print(error_msg, file=sys.stderr)
            call_api(f"/api/internal/agent/jobs/{leased_job_id}/fail", method="POST", payload={
                "lease_token": lease_token,
                "error": error_msg
            })
            sys.exit(1)
            
        print(f"Resolving {len(feeds)} feeds from configuration...", file=sys.stderr)
        resolved_feeds = []
        for f in feeds:
            f_name = f.get("name")
            f_url = f.get("url")
            f_kind = f.get("kind", "rss")
            f_tags = f.get("tags", [])
            f_group = f.get("source_group")
            
            if f_kind == "opml":
                print(f"Resolving OPML source: {f_name} ({f_url})...", file=sys.stderr)
                try:
                    xml_data = fetch_url_content_via_urllib(f_url, config)
                    rss_urls = parse_opml_rss_urls(xml_data)
                    for item in rss_urls:
                        resolved_feeds.append({
                            "name": item["name"],
                            "url": item["url"],
                            "tags": f_tags + ["hn-popular-blogs"],
                            "source_group": f_group
                        })
                except Exception as e:
                    print(f"Warning: failed to resolve OPML {f_name}: {e}", file=sys.stderr)
            else:
                resolved_feeds.append({
                    "name": f_name,
                    "url": f_url,
                    "tags": f_tags,
                    "source_group": f_group
                })
                
        print(f"Total resolved RSS feeds to crawl: {len(resolved_feeds)}", file=sys.stderr)
        if not resolved_feeds:
            error_msg = "No RSS feeds resolved from sources"
            call_api(f"/api/internal/agent/jobs/{leased_job_id}/fail", method="POST", payload={
                "lease_token": lease_token,
                "error": error_msg
            })
            sys.exit(1)
            
        # Fetch entries from resolved feeds in parallel using ThreadPoolExecutor
        def crawl_feed(feed):
            try:
                xml_data = fetch_url_content_via_urllib(feed["url"], config)
                entries = parse_feed_entries(xml_data)
                for entry in entries:
                    entry["feed_tags"] = feed["tags"]
                    entry["source_group"] = feed["source_group"]
                    if not entry.get("source_name"):
                        entry["source_name"] = feed["name"]
                return entries
            except Exception as e:
                # Log warning and skip
                print(f"Warning: failed to crawl feed {feed['name']}: {e}", file=sys.stderr)
                return []
                
        candidate_entries = []
        with concurrent.futures.ThreadPoolExecutor(max_workers=8) as executor:
            futures = [executor.submit(crawl_feed, rf) for rf in resolved_feeds]
            for future in concurrent.futures.as_completed(futures):
                candidate_entries.extend(future.result())
                
        print(f"Crawled {len(candidate_entries)} total raw candidate entries.", file=sys.stderr)
        
        # Filter entries by age
        now_epoch = int(time.time())
        age_limit_secs = max_age_days * 24 * 3600
        recent_candidates = []
        seen_links = set()
        
        for entry in candidate_entries:
            link = entry.get("link")
            if not link or link in seen_links:
                continue
            seen_links.add(link)
            
            pub_epoch = parse_iso_to_epoch(entry.get("pub_date"))
            # Fallback if parsing returned current time but RSS had text
            if pub_epoch == now_epoch and entry.get("pub_date"):
                try:
                    dt = datetime.strptime(entry.get("pub_date").split('+')[0].split('-')[0].strip(), "%a, %d %b %Y %H:%M:%S")
                    pub_epoch = int(calendar.timegm(dt.utctimetuple()))
                except Exception:
                    pass
            
            entry["pub_epoch"] = pub_epoch
            if (now_epoch - pub_epoch) <= age_limit_secs:
                recent_candidates.append(entry)
                
        print(f"Filtered to {len(recent_candidates)} recent candidates (< {max_age_days} days old).", file=sys.stderr)
        if not recent_candidates:
            print("No recent candidates found in configured feeds. Exiting cleanly.", file=sys.stderr)
            print("FreshLoop scheduled agent result")
            print(f"job_type: {JOB_TYPE}")
            print(f"job_id: {leased_job_id}")
            print("status: completed")
            print("result_ref: none")
            print("voice_job_ids: none")
            print("notes: no recent candidate articles to curate")
            sys.exit(0)
            
        # Sort candidates newest first
        recent_candidates.sort(key=lambda x: x.get("pub_epoch", 0), reverse=True)
        
        # Keep at most 1 item per feed to ensure source diversity and prevent a single feed from crowding
        diverse_candidates = []
        seen_sources = set()
        for c in recent_candidates:
            src = c.get("source_name") or "Unknown"
            if src not in seen_sources:
                diverse_candidates.append(c)
                seen_sources.add(src)
                
        # Batch deduplication check
        urls_to_check = [c["link"] for c in diverse_candidates]
        dedup_status, dedup_body = call_api("/api/internal/dedup/check", method="POST", payload={"urls": urls_to_check})
        if dedup_status != 200:
            error_msg = f"Dedup check failed, status: {dedup_status}, body: {dedup_body}"
            call_api(f"/api/internal/agent/jobs/{leased_job_id}/fail", method="POST", payload={
                "lease_token": lease_token,
                "error": error_msg
            })
            sys.exit(1)
            
        existing_urls = set(json.loads(dedup_body).get("existing_urls", []))
        uncurated_candidates = [c for c in diverse_candidates if c["link"] not in existing_urls]
        
        # Limit evaluation count to 10 to keep execution times fast and safe,
        # but stop as soon as we have successfully published max_items_per_cycle (8) articles.
        max_items = curated_config.get("max_items_per_cycle", 8)
        eval_candidates = uncurated_candidates[:10]
        print(f"Deduplication finished: {len(eval_candidates)} diverse uncurated candidates selected for evaluation (max_items_per_cycle={max_items}).", file=sys.stderr)
        
        if not eval_candidates:
            print("All recent candidates have already been curated. Exiting cleanly.", file=sys.stderr)
            print("FreshLoop scheduled agent result")
            print(f"job_type: {JOB_TYPE}")
            print(f"job_id: {leased_job_id}")
            print("status: completed")
            print("result_ref: none")
            print("voice_job_ids: none")
            print("notes: all recent articles are already curated")
            sys.exit(0)
            
        # Loop through candidates to find ones that meet quality/should_publish filters
        selected_articles = []
        for candidate in eval_candidates:
            if len(selected_articles) >= max_items:
                print(f"Reached max_items_per_cycle limit ({max_items}). Stopping evaluation.", file=sys.stderr)
                break
                
            # Send heartbeat to extend the main job lease and avoid timeout during LLM calls
            try:
                h_status, h_body = call_api(f"/api/internal/agent/jobs/{leased_job_id}/heartbeat", method="POST", payload={
                    "lease_token": lease_token,
                    "lease_seconds": 1800
                })
                if h_status != 200:
                    print(f"Warning: heartbeat failed with status {h_status}, body: {h_body}", file=sys.stderr)
            except Exception as hb_e:
                print(f"Warning: failed to send heartbeat: {hb_e}", file=sys.stderr)
                
            c_url = candidate["link"]
            c_title = candidate["title"]
            print(f"Evaluating candidate: {c_title} ({c_url})...", file=sys.stderr)
            
            try:
                # Fetch full HTML content
                c_html = fetch_url_content_via_urllib(c_url, config)
                
                # Convert HTML to markdown/text locally
                md_parser = HTMLToMarkdown()
                md_parser.feed(c_html)
                c_reader_markdown = md_parser.get_markdown()
                
                txt_parser = HTMLToPlainText()
                txt_parser.feed(c_html)
                c_plain_text = txt_parser.get_text()
                
                if len(c_plain_text.strip()) < 80:
                    print("Skipping: extracted text too short (< 80 chars)", file=sys.stderr)
                    continue
                    
                # Call LLM to evaluate quality and should_publish
                print("Running LLM analysis on candidate...", file=sys.stderr)
                meta = generate_article_metadata(
                    c_title, c_url, c_reader_markdown[:12000],
                    personalization_context=personalization_context,
                    source_name=candidate["source_name"]
                )
                
                should_pub = meta.get("should_publish", True)
                score = meta.get("quality_score", 8)
                print(f"LLM evaluation: should_publish={should_pub}, quality_score={score} (min threshold={min_quality_score})", file=sys.stderr)
                
                if should_pub and score >= min_quality_score:
                    # Construct article artifact
                    words_count = len(c_plain_text.split())
                    reading_time_min = max(1, int(round(words_count / 180.0)))
                    source_url = "/".join(c_url.split('/')[:3])
                    
                    artifact = {
                        "title": meta.get("title") or c_title,
                        "subtitle": meta.get("subtitle"),
                        "source_name": candidate["source_name"],
                        "source_url": source_url,
                        "original_url": c_url,
                        "canonical_url": c_url,
                        "publish_time": int(time.time()),
                        "reader_markdown": c_reader_markdown,
                        "plain_text": c_plain_text,
                        "compressed_markdown": meta.get("compressed_markdown"),
                        "audio_script": meta.get("audio_script"),
                        "key_points": meta.get("key_points"),
                        "reading_time_min": reading_time_min,
                        "quality_score": score,
                        "tags": meta.get("tags") or candidate["feed_tags"]
                    }
                    
                    validate_artifact(artifact)
                    selected_articles.append((candidate, artifact))
                    print(f"Selected candidate: {c_title} (Current count: {len(selected_articles)})", file=sys.stderr)
                else:
                    print(f"Skipping candidate '{c_title}' (quality score or publish status too low)", file=sys.stderr)
                    
            except Exception as e:
                print(f"Warning: failed to process candidate '{c_title}': {e}", file=sys.stderr)
                continue
                
        if not selected_articles:
            print("No candidates met the publication criteria. Exiting cleanly.", file=sys.stderr)
            print("FreshLoop scheduled agent result")
            print(f"job_type: {JOB_TYPE}")
            print(f"job_id: {leased_job_id}")
            print("status: completed")
            print("result_ref: none")
            print("voice_job_ids: none")
            print("notes: no candidates met publication criteria")
            sys.exit(0)

        # Submit first artifact to main leased job
        first_candidate, first_artifact = selected_articles[0]
        print(f"Submitting first article '{first_artifact['title']}' to main leased job {leased_job_id}...", file=sys.stderr)
        s_status, s_body = call_api(f"/api/internal/agent/jobs/{leased_job_id}/submit", method="POST", payload={
            "lease_token": lease_token,
            "artifact": first_artifact
        })
        
        if s_status != 200:
            error_msg = f"Failed to submit main artifact, status: {s_status}, body: {s_body}"
            print(error_msg, file=sys.stderr)
            call_api(f"/api/internal/agent/jobs/{leased_job_id}/fail", method="POST", payload={
                "lease_token": lease_token,
                "error": error_msg
            })
            sys.exit(1)
            
        submit_res = json.loads(s_body)
        result_ref = submit_res.get("result_ref", "none")
        voice_job_ids = submit_res.get("voice_job_ids", [])
        
        # Submit subsequent articles to dynamically created jobs
        additional_published_count = 0
        for idx, (sub_candidate, sub_artifact) in enumerate(selected_articles[1:], start=1):
            success = publish_additional_article(
                sub_candidate, sub_artifact, AGENT_ID, JOB_TYPE, RUN_DATE, RUN_WINDOW, idx
            )
            if success:
                additional_published_count += 1
                
        # Output result contract
        print("FreshLoop scheduled agent result")
        print(f"job_type: {JOB_TYPE}")
        print(f"job_id: {leased_job_id}")
        print("status: completed")
        print(f"result_ref: {result_ref}")
        print(f"voice_job_ids: {','.join(voice_job_ids) or 'none'}")
        print(f"notes: successfully published main article and {additional_published_count} additional articles")

if __name__ == "__main__":
    main()
