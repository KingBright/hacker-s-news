#!/usr/bin/env python3
"""
FreshLoop Pipeline Diagnostic Tool (v2)

Analyzes trace logs and Nexus data to diagnose quality issues
in the news production pipeline.

Usage:
  python extract_trace.py                       # Interactive item selection
  python extract_trace.py <item_id_prefix>      # Direct item lookup
  python extract_trace.py --trace <id_or_path>  # Direct trace file analysis
  python extract_trace.py --batch               # Batch statistics
  python extract_trace.py --batch --days 7      # Batch for last 7 days
"""
import os
import sys
import glob
import datetime
import re
import json
import argparse
import ssl
import urllib.request
import urllib.error
import urllib.parse
from dataclasses import dataclass, field
from difflib import SequenceMatcher
from typing import Optional, List, Dict, Tuple

# ============================================================
# Configuration
# ============================================================
NEXUS_API_URL = os.environ.get("FRESHLOOP_NEXUS_URL", "https://news.hackerlife.fun:8443")
NEXUS_AUTH_KEY = os.environ.get("FRESHLOOP_NEXUS_KEY", "")
TRACE_LOG_DIR = os.path.expanduser("~/.freshloop/logs/traces")

# Old category name -> normalized new name (lowercased, no spaces/underscores)
CATEGORY_ALIASES = {
    "ai":       "ai前沿",
    "政治":     "国际时政",
    "数码":     "硬件数码",
    "财经":     "商业财经",
    "探索":     "科学探索",
    "文娱":     "影音文娱",
    "游戏影音": "游戏电竞",
    "工程技术": "技术产业",
    "生命科学": "生命健康",
    "读书":     "生活杂谈",
}


# ============================================================
# Data Classes
# ============================================================
@dataclass
class TraceStep:
    index: int
    name: str
    time: str
    details: str
    llm_prompt: Optional[str] = None
    llm_response: Optional[str] = None


@dataclass
class ClusterInfo:
    ids: List[int]
    theme: str
    reason: str
    is_fallback: bool = False


@dataclass
class PlanStep:
    action: str
    group_index: int
    transition: Optional[str] = None


@dataclass
class InputItem:
    group_idx: int
    group_theme: str
    title: str
    source: str
    summary: str


@dataclass
class ParsedTrace:
    """Fully parsed trace file."""
    file_path: str
    task_id: str
    category: str
    start_time: Optional[datetime.datetime] = None
    total_steps: int = 0
    steps: List[TraceStep] = field(default_factory=list)
    # Extracted structured data
    is_regen: bool = False
    clusters: List[ClusterInfo] = field(default_factory=list)
    clustering_failed: bool = False
    plan_steps: List[PlanStep] = field(default_factory=list)
    planning_failed: bool = False
    planning_error: str = ""
    input_items: List[InputItem] = field(default_factory=list)
    final_script: str = ""
    host_name: str = ""
    audio_duration_sec: int = 0
    step_times: Dict[str, str] = field(default_factory=dict)


@dataclass
class DiagnosticResult:
    """Computed diagnostic metrics."""
    coverage_ratio: float = 0.0
    covered_items: List[str] = field(default_factory=list)
    missing_items: List[str] = field(default_factory=list)
    copy_paste_ratio: float = 0.0
    high_copy_items: List[Tuple[str, float]] = field(default_factory=list)
    clustering_ok: bool = True
    planning_ok: bool = True
    pipeline_duration_sec: int = 0
    repeated_sentences: List[str] = field(default_factory=list)
    flags: List[str] = field(default_factory=list)


# ============================================================
# SSL helper
# ============================================================
def _ssl_ctx():
    ca_file = os.environ.get("FRESHLOOP_CA_FILE")
    ctx = ssl.create_default_context(cafile=ca_file)
    if os.environ.get("FRESHLOOP_INSECURE_TLS") == "1":
        ctx.check_hostname = False
        ctx.verify_mode = ssl.CERT_NONE
    return ctx


# ============================================================
# Nexus API Client
# ============================================================
class NexusClient:
    def __init__(self, api_url=NEXUS_API_URL, auth_key=NEXUS_AUTH_KEY):
        self.api_url = api_url.rstrip("/")
        self.auth_key = auth_key

    def _request(self, path):
        url = f"{self.api_url}{path}"
        req = urllib.request.Request(url)
        if self.auth_key:
            req.add_header("x-api-key", self.auth_key)
            req.add_header("X-NEXUS-KEY", self.auth_key)
        req.add_header("User-Agent", "FreshLoop-Diagnostic/2.0")
        try:
            with urllib.request.urlopen(req, context=_ssl_ctx()) as resp:
                return json.loads(resp.read())
        except Exception as e:
            print(f"  [Nexus] request failed {path}: {e}")
            return None

    def get_recent_items(self, limit=50, category=None):
        path = f"/api/items?limit={limit}"
        if category:
            path += f"&category={urllib.parse.quote(category)}"
        result = self._request(path)
        return result if isinstance(result, list) else []

    def get_item_sources(self, item_id):
        result = self._request(f"/api/items/{item_id}/sources")
        return result if isinstance(result, list) else []


# ============================================================
# Trace Parser
# ============================================================
class TraceParser:
    """Parse trace markdown files into structured data."""

    STEP_RE = re.compile(r"^## (\d+)\. (.+?) \((.+?)\)")

    @classmethod
    def parse_file(cls, filepath: str) -> ParsedTrace:
        with open(filepath, "r", encoding="utf-8") as f:
            content = f.read()

        trace = ParsedTrace(file_path=filepath, task_id="", category="")

        # --- header ---
        m = re.search(r"\*\*Task ID\*\*: (.+)", content)
        if m:
            trace.task_id = m.group(1).strip()
        m = re.search(r"\*\*Category\*\*: (.+)", content)
        if m:
            trace.category = m.group(1).strip()
        m = re.search(r"\*\*Start Time\*\*: (.+)", content)
        if m:
            try:
                trace.start_time = datetime.datetime.strptime(
                    m.group(1).strip()[:19], "%Y-%m-%d %H:%M:%S"
                )
            except ValueError:
                pass
        m = re.search(r"\*\*Total Steps\*\*: (\d+)", content)
        if m:
            trace.total_steps = int(m.group(1))

        # --- steps ---
        trace.steps = cls._parse_steps(content)
        cls._extract_structured(trace)
        return trace

    @classmethod
    def _parse_steps(cls, content: str) -> List[TraceStep]:
        steps: List[TraceStep] = []
        cur: Optional[TraceStep] = None
        for line in content.split("\n"):
            m = cls.STEP_RE.match(line)
            if m:
                if cur is not None:
                    steps.append(cur)
                cur = TraceStep(
                    index=int(m.group(1)),
                    name=m.group(2).strip(),
                    time=m.group(3),
                    details="",
                )
            elif cur is not None:
                cur.details += line + "\n"
        if cur is not None:
            steps.append(cur)

        # extract llm prompt / response from details
        for step in steps:
            pm = re.search(
                r"\*\*LLM Prompt\*\*:\n```text\n(.*?)\n```",
                step.details,
                re.DOTALL,
            )
            if pm:
                step.llm_prompt = pm.group(1)
            rm = re.search(
                r"\*\*LLM Response\*\*:\n```text\n(.*?)\n```",
                step.details,
                re.DOTALL,
            )
            if rm:
                step.llm_response = rm.group(1)
        return steps

    # ---- structured extraction from steps ----
    @classmethod
    def _extract_structured(cls, trace: ParsedTrace):
        for step in trace.steps:
            name = step.name

            if name == "Start":
                trace.is_regen = "Regen: true" in step.details
                trace.step_times["start"] = step.time

            elif "Clustering Result" in name:
                trace.step_times["clustering"] = step.time
                cls._parse_clustering(trace, step.details)

            elif "Planning Phase 2" in name:
                trace.step_times["planning"] = step.time
                cls._parse_planning(trace, step)

            elif "Planning Failed" in name:
                trace.planning_failed = True
                trace.planning_error = step.details.strip().split("\n")[0]
                trace.step_times["planning"] = step.time

            elif "Planning Flow" in name or "Plan Episode Flow" in name:
                trace.step_times["planning"] = step.time
                cls._parse_planning_legacy(trace, step)

            elif "Unified Result" in name:
                trace.step_times["generation"] = step.time
                if step.llm_prompt:
                    cls._parse_input_items(trace, step.llm_prompt)
                    hm = re.search(r"Host: (.+?)[）\)]", step.llm_prompt)
                    if hm:
                        trace.host_name = hm.group(1).strip()
                if step.llm_response:
                    text = step.llm_response.strip()
                    # strip possible ```json wrapper
                    text = re.sub(r"^```\w*\s*", "", text)
                    text = re.sub(r"\s*```$", "", text)
                    text = text.strip()
                    trace.final_script = (
                        (trace.final_script + "\n\n" + text)
                        if trace.final_script
                        else text
                    )

            elif "Segment Writer Result" in name or "Segment Result" in name:
                trace.step_times.setdefault("generation", step.time)
                # Legacy: extract input items + host from prompt
                if step.llm_prompt:
                    cls._parse_input_items_legacy(trace, step.llm_prompt)
                    if not trace.host_name:
                        hm = re.search(r"人设[:：]\s*(\S+)", step.llm_prompt)
                        if hm:
                            trace.host_name = hm.group(1).strip()
                if step.llm_response:
                    seg = step.llm_response.strip()
                    trace.final_script = (
                        (trace.final_script + "\n\n" + seg)
                        if trace.final_script
                        else seg
                    )

            elif "Audio Processing" in name:
                trace.step_times["audio"] = step.time
                dm = re.search(r"(\d+)s", step.details)
                if dm:
                    trace.audio_duration_sec = int(dm.group(1))

    # --- clustering ---
    @classmethod
    def _parse_clustering(cls, trace: ParsedTrace, details: str):
        pat = re.compile(
            r'ClusterGroup \{ ids: \[([^\]]*)\], theme: "(.*?)", clustering_reason: "(.*?)" \}'
        )
        for m in pat.finditer(details):
            ids = [int(x.strip()) for x in m.group(1).split(",") if x.strip()]
            theme = m.group(2)
            reason = m.group(3)
            trace.clusters.append(
                ClusterInfo(
                    ids=ids,
                    theme=theme,
                    reason=reason,
                    is_fallback="聚类失败回退" in reason,
                )
            )
        if trace.clusters and all(c.is_fallback for c in trace.clusters):
            trace.clustering_failed = True

    # --- planning (new format) ---
    @classmethod
    def _parse_planning(cls, trace: ParsedTrace, step: TraceStep):
        if not step.llm_response:
            return
        text = step.llm_response.strip()
        text = re.sub(r"^```json\s*", "", text)
        text = re.sub(r"\s*```$", "", text)
        try:
            start = text.index("[")
            end = text.rindex("]") + 1
            items = json.loads(text[start:end])
            for item in items:
                if isinstance(item, dict):
                    trace.plan_steps.append(
                        PlanStep(
                            action=item.get("action", "sequence"),
                            group_index=item.get("group_index", 0),
                            transition=item.get("transition_rationale"),
                        )
                    )
        except (ValueError, json.JSONDecodeError):
            trace.planning_failed = True
            trace.planning_error = "JSON parse failed"

    # --- planning (legacy format) ---
    @classmethod
    def _parse_planning_legacy(cls, trace: ParsedTrace, step: TraceStep):
        if not step.llm_response:
            return
        text = step.llm_response.strip()
        try:
            start = text.index("[")
            end = text.rindex("]") + 1
            id_list = json.loads(text[start:end])
            for item_id in id_list:
                if isinstance(item_id, int):
                    trace.plan_steps.append(
                        PlanStep(action="sequence", group_index=item_id, transition=None)
                    )
        except (ValueError, json.JSONDecodeError):
            pass

    # --- input items from unified prompt ---
    @classmethod
    def _parse_input_items(cls, trace: ParsedTrace, prompt: str):
        group_re = re.compile(r"=== 第 (\d+) 组 \(主题[：:](.*?)\) ===")
        cur_group_idx = 0
        cur_group_theme = ""
        cur_title = ""
        cur_source = ""
        cur_summary = ""
        in_item = False

        def _flush():
            nonlocal cur_title, cur_source, cur_summary, in_item
            if cur_title:
                trace.input_items.append(
                    InputItem(
                        group_idx=cur_group_idx,
                        group_theme=cur_group_theme,
                        title=cur_title,
                        source=cur_source,
                        summary=cur_summary,
                    )
                )
            cur_title = cur_source = cur_summary = ""
            in_item = False

        for line in prompt.split("\n"):
            gm = group_re.match(line.strip())
            if gm:
                _flush()
                cur_group_idx = int(gm.group(1))
                cur_group_theme = gm.group(2).strip()
                continue

            stripped = line.strip()
            if stripped == "--- Item ---":
                _flush()
                in_item = True
                continue

            if in_item:
                if stripped.startswith("标题:") or stripped.startswith("标题："):
                    cur_title = stripped.split(":", 1)[-1].split("：", 1)[-1].strip()
                elif stripped.startswith("来源:") or stripped.startswith("来源："):
                    cur_source = stripped.split(":", 1)[-1].split("：", 1)[-1].strip()
                elif stripped.startswith("摘要:") or stripped.startswith("摘要："):
                    cur_summary = stripped.split(":", 1)[-1].split("：", 1)[-1].strip()

        _flush()

    # --- input items from legacy Segment Writer prompt ---
    @classmethod
    def _parse_input_items_legacy(cls, trace: ParsedTrace, prompt: str):
        """Extract items from legacy 【新闻素材】 section in Segment Writer prompts."""
        mat_match = re.search(r"【新闻素材】\n(.*?)(?:\n【|$)", prompt, re.DOTALL)
        if not mat_match:
            return

        materials = mat_match.group(1)
        group_idx = 0
        if trace.input_items:
            group_idx = trace.input_items[-1].group_idx + 1

        cur_title = ""
        cur_source = ""
        cur_summary = ""
        seen_titles = {item.title for item in trace.input_items}

        def _flush_leg():
            nonlocal cur_title, cur_source, cur_summary
            if cur_title and cur_title not in seen_titles:
                trace.input_items.append(
                    InputItem(
                        group_idx=group_idx,
                        group_theme=f"Segment {group_idx + 1}",
                        title=cur_title,
                        source=cur_source,
                        summary=cur_summary,
                    )
                )
                seen_titles.add(cur_title)
            cur_title = cur_source = cur_summary = ""

        for line in materials.split("\n"):
            stripped = line.strip()
            if stripped.startswith("- ") and not stripped.startswith("- 摘要") and not stripped.startswith("- 来源"):
                _flush_leg()
                cur_title = stripped[2:].strip()
            elif stripped.startswith("摘要:") or stripped.startswith("摘要："):
                cur_summary = stripped.split(":", 1)[-1].split("：", 1)[-1].strip()
            elif stripped.startswith("来源:") or stripped.startswith("来源："):
                cur_source = stripped.split(":", 1)[-1].split("：", 1)[-1].strip()
            elif cur_summary and stripped and not stripped.startswith("- "):
                cur_summary += stripped

        _flush_leg()


# ============================================================
# Diagnostic Analyzer
# ============================================================
class DiagnosticAnalyzer:

    @staticmethod
    def analyze(trace: ParsedTrace) -> DiagnosticResult:
        d = DiagnosticResult()
        d.clustering_ok = not trace.clustering_failed
        d.planning_ok = not trace.planning_failed

        # --- coverage ---
        if trace.input_items and trace.final_script:
            for item in trace.input_items:
                if DiagnosticAnalyzer._is_covered(item, trace.final_script):
                    d.covered_items.append(item.title)
                else:
                    d.missing_items.append(item.title)
            total = len(trace.input_items)
            d.coverage_ratio = len(d.covered_items) / total if total else 0

        # --- copy-paste ---
        if trace.input_items and trace.final_script:
            ratios = []
            for item in trace.input_items:
                if not item.summary or len(item.summary) < 15:
                    continue
                vr = DiagnosticAnalyzer._verbatim_ratio(item.summary, trace.final_script)
                ratios.append(vr)
                if vr > 0.85:
                    d.high_copy_items.append((item.title, vr))
            if ratios:
                d.copy_paste_ratio = sum(ratios) / len(ratios)

        # --- pipeline duration ---
        start_t = trace.step_times.get("start")
        end_t = trace.step_times.get("audio") or trace.step_times.get("generation")
        if start_t and end_t:
            try:
                t0 = datetime.datetime.strptime(start_t, "%H:%M:%S")
                t1 = datetime.datetime.strptime(end_t, "%H:%M:%S")
                delta = int((t1 - t0).total_seconds())
                d.pipeline_duration_sec = delta if delta >= 0 else delta + 86400
            except ValueError:
                pass

        # --- repeated sentences ---
        if trace.final_script:
            d.repeated_sentences = DiagnosticAnalyzer._find_repeats(trace.final_script)

        # --- flags ---
        if not d.clustering_ok:
            d.flags.append("CLUSTERING_FALLBACK: 聚类全部回退，相关新闻未被正确分组")
        if not d.planning_ok:
            d.flags.append(f"PLANNING_FAILED: 编排失败 ({trace.planning_error})")
        if d.coverage_ratio < 0.8 and trace.input_items:
            d.flags.append(
                f"LOW_COVERAGE: 仅 {d.coverage_ratio:.0%} 的素材被写入播报稿"
            )
        if d.copy_paste_ratio > 0.85:
            d.flags.append(
                f"HIGH_COPY_PASTE: 平均 {d.copy_paste_ratio:.0%} 的内容直接复制摘要"
            )
        if d.high_copy_items:
            d.flags.append(
                f"VERBATIM_COPY: {len(d.high_copy_items)} 条新闻被逐字复制"
            )
        if (
            trace.audio_duration_sec > 0
            and trace.audio_duration_sec < 60
            and len(trace.input_items) > 3
        ):
            d.flags.append(
                f"TOO_SHORT: 音频仅 {trace.audio_duration_sec}s，素材有 {len(trace.input_items)} 条"
            )
        if d.repeated_sentences:
            d.flags.append(
                f"REPEATED_CONTENT: 发现 {len(d.repeated_sentences)} 处重复语句"
            )
        return d

    @staticmethod
    def _is_covered(item: InputItem, script: str) -> bool:
        # Strategy: extract 3-4 char n-grams from title and check if enough appear in script.
        # Also check summary fragments as fallback.
        clean = re.sub(r"^\d+版\w+ - ", "", item.title).strip()
        # Remove generic words
        clean = re.sub(r"^(图片新闻|要闻|简讯)\s*[-—]?\s*", "", clean).strip()

        # Extract English words and numbers
        en_kw = re.findall(r"[A-Za-z]{3,}|\d{3,}", clean)
        # Extract Chinese n-grams (3-char sliding window over CJK runs)
        cjk_runs = re.findall(r"[\u4e00-\u9fff]+", clean)
        cn_kw = []
        for run in cjk_runs:
            if len(run) <= 4:
                cn_kw.append(run)
            else:
                for i in range(len(run) - 2):
                    cn_kw.append(run[i : i + 3])

        keywords = en_kw + cn_kw
        if not keywords:
            # Fallback: check summary head
            if item.summary and len(item.summary) > 20:
                return item.summary[:20] in script
            return True

        matched = sum(1 for kw in keywords if kw in script)
        if matched >= max(1, len(keywords) * 0.3):
            return True

        # Secondary check: does the summary content appear in script?
        if item.summary and len(item.summary) > 20:
            return DiagnosticAnalyzer._verbatim_ratio(item.summary, script) > 0.3

        return False

    @staticmethod
    def _verbatim_ratio(summary: str, script: str) -> float:
        chunk_size = min(30, len(summary) // 2)
        if chunk_size < 10:
            return 1.0 if summary in script else 0.0
        chunks = [summary[i : i + chunk_size] for i in range(0, len(summary) - chunk_size + 1, chunk_size)]
        if not chunks:
            return 0.0
        return sum(1 for c in chunks if c in script) / len(chunks)

    @staticmethod
    def _find_repeats(script: str) -> List[str]:
        sentences = re.split(r"[。！？\n]", script)
        sentences = [s.strip() for s in sentences if len(s.strip()) > 15]
        seen: Dict[str, int] = {}
        for s in sentences:
            seen[s] = seen.get(s, 0) + 1
        return [s for s, c in seen.items() if c > 1]


# ============================================================
# Trace Finder
# ============================================================
class TraceFinder:
    FN_RE = re.compile(r"trace_(\d{8})_(\d{4})_(.+)_([a-f0-9]+)\.md")

    @staticmethod
    def _normalize(cat: str) -> str:
        c = cat.lower().replace("_", "").replace(" ", "")
        return CATEGORY_ALIASES.get(c, c)

    @classmethod
    def parse_filename(cls, filepath: str):
        m = cls.FN_RE.match(os.path.basename(filepath))
        if m:
            dt = datetime.datetime.strptime(
                f"{m.group(1)} {m.group(2)}", "%Y%m%d %H%M"
            )
            return dt, m.group(3), m.group(4)
        return None, None, None

    @classmethod
    def find_by_trace_id(cls, prefix: str) -> Optional[str]:
        for f in glob.glob(os.path.join(TRACE_LOG_DIR, "trace_*.md")):
            _, _, tid = cls.parse_filename(f)
            if tid and tid.startswith(prefix):
                return f
        return None

    @classmethod
    def find_matching(cls, category: str, publish_ts: int) -> Optional[str]:
        pub_dt = datetime.datetime.fromtimestamp(publish_ts)
        norm = cls._normalize(category)
        candidates = []
        for f in glob.glob(os.path.join(TRACE_LOG_DIR, "trace_*.md")):
            dt, cat, _ = cls.parse_filename(f)
            if not dt:
                continue
            if cls._normalize(cat) != norm:
                continue
            if dt > pub_dt:
                continue
            delta = (pub_dt - dt).total_seconds()
            if delta > 14400:  # 4h
                continue
            candidates.append((delta, f))
        if not candidates:
            return None
        candidates.sort(key=lambda x: x[0])
        return candidates[0][1]

    @classmethod
    def list_all(cls, days: Optional[int] = None) -> List[str]:
        files = glob.glob(os.path.join(TRACE_LOG_DIR, "trace_*.md"))
        if days is not None:
            cutoff = datetime.datetime.now() - datetime.timedelta(days=days)
            files = [f for f in files if (cls.parse_filename(f)[0] or datetime.datetime.min) >= cutoff]
        return sorted(files)


# ============================================================
# Report Generator
# ============================================================
class ReportGenerator:

    @staticmethod
    def single(
        trace: ParsedTrace,
        diag: DiagnosticResult,
        nexus_item: Optional[dict] = None,
        nexus_sources: Optional[list] = None,
    ) -> str:
        lines: List[str] = []

        title = nexus_item.get("title", trace.category) if nexus_item else trace.category
        lines.append(f"# FreshLoop 管线诊断报告: {title}\n")

        # ── Ch.1 概览 ──
        lines.append("## Ch.1 概览\n")
        lines.append("| 字段 | 值 |")
        lines.append("|------|-----|")
        lines.append(f"| 分类 | {trace.category} |")
        lines.append(f"| 主播 | {trace.host_name or '未知'} |")
        if nexus_item:
            lines.append(f"| Nexus ID | `{nexus_item.get('id', 'N/A')}` |")
            pts = nexus_item.get("publish_time", 0)
            if pts:
                lines.append(f"| 发布时间 | {datetime.datetime.fromtimestamp(pts)} |")
            lines.append(f"| 音频时长 | {nexus_item.get('duration_sec', 0)}s |")
        else:
            if trace.start_time:
                lines.append(f"| Trace 开始 | {trace.start_time} |")
            lines.append(f"| 音频时长 | {trace.audio_duration_sec}s |")
        lines.append(f"| Trace ID | `{trace.task_id[:8]}` |")
        lines.append(f"| Trace 文件 | `{os.path.basename(trace.file_path)}` |")
        lines.append(f"| 输入素材数 | {len(trace.input_items)} |")
        lines.append(f"| 管线总耗时 | {diag.pipeline_duration_sec}s |")
        lines.append(f"| 聚类状态 | {'正常' if diag.clustering_ok else '回退 (FALLBACK)'} |")
        lines.append(f"| 编排状态 | {'正常' if diag.planning_ok else '失败 (FALLBACK)'} |")
        lines.append(f"| 素材覆盖率 | {diag.coverage_ratio:.0%} |")
        lines.append(f"| 复制粘贴率 | {diag.copy_paste_ratio:.0%} |")
        lines.append("")

        if diag.flags:
            lines.append("### 问题标记\n")
            for f in diag.flags:
                lines.append(f"- **{f}**")
            lines.append("")
        lines.append("---\n")

        # ── Ch.2 原始素材 ──
        lines.append("## Ch.2 原始素材\n")
        lines.append("> LLM 收到的全部新闻输入，按编排分组显示。\n")
        if trace.input_items:
            cur_g = -1
            for item in trace.input_items:
                if item.group_idx != cur_g:
                    cur_g = item.group_idx
                    lines.append(f"### 第 {item.group_idx} 组: {item.group_theme}\n")
                marker = "+" if item.title in diag.covered_items else "MISS"
                lines.append(f"- [{marker}] **{item.title}** ({item.source})")
                lines.append(f"  > {item.summary}\n")
        else:
            lines.append("*未从 trace 中提取到输入素材。*\n")

        if nexus_sources:
            lines.append("### Nexus DB 原始来源\n")
            for src in nexus_sources:
                lines.append(f"- **{src.get('source_title', 'N/A')}**")
                lines.append(f"  URL: {src.get('source_url', '')}")
                s = src.get("source_summary", "")
                if s:
                    lines.append(f"  > {s}")
                lines.append("")
        lines.append("---\n")

        # ── Ch.3 聚类分析 ──
        lines.append("## Ch.3 聚类分析\n")
        if trace.clusters:
            if trace.clustering_failed:
                lines.append(
                    "> **[WARNING] 聚类全部回退!** LLM 聚类调用失败，每条新闻被独立成组。\n"
                )
            lines.append(f"共 {len(trace.clusters)} 个聚类组:\n")
            for i, cl in enumerate(trace.clusters):
                tag = "FALLBACK" if cl.is_fallback else "OK"
                ids = ", ".join(str(x) for x in cl.ids)
                lines.append(f"**组 {i}: {cl.theme}** [{tag}]")
                lines.append(f"- Items: [{ids}]")
                lines.append(f"- 原因: {cl.reason}\n")
        else:
            lines.append("*Trace 中未找到聚类步骤（可能是旧版格式或 regen 模式）。*\n")
        lines.append("---\n")

        # ── Ch.4 编排分析 ──
        lines.append("## Ch.4 编排分析\n")
        if trace.planning_failed:
            lines.append(f"> **[WARNING] 编排失败!** {trace.planning_error}\n")
            lines.append("> 系统使用了简单分组作为回退方案。\n")
        if trace.plan_steps:
            lines.append("| 序号 | Action | 组索引 | 过渡策略 |")
            lines.append("|------|--------|--------|----------|")
            for i, ps in enumerate(trace.plan_steps):
                t = ps.transition or "自然过渡"
                lines.append(f"| {i+1} | {ps.action} | {ps.group_index} | {t} |")
            lines.append("")
        else:
            lines.append("*未找到编排步骤。*\n")
        lines.append("---\n")

        # ── Ch.5 最终文稿 + 质量对比 ──
        lines.append("## Ch.5 最终文稿 + 质量对比\n")
        if trace.final_script:
            lines.append("### 播报稿全文\n")
            lines.append("```")
            lines.append(trace.final_script)
            lines.append("```\n")

            if trace.input_items:
                lines.append("### 素材覆盖矩阵\n")
                lines.append("| 素材标题 | 状态 | 复制率 |")
                lines.append("|----------|------|--------|")
                for item in trace.input_items:
                    covered = "已覆盖" if item.title in diag.covered_items else "**未覆盖**"
                    cr = DiagnosticAnalyzer._verbatim_ratio(item.summary, trace.final_script) if item.summary else 0
                    cr_s = f"{cr:.0%}"
                    if cr > 0.85:
                        cr_s = f"**{cr_s} (逐字复制)**"
                    short = item.title[:40] + ("..." if len(item.title) > 40 else "")
                    lines.append(f"| {short} | {covered} | {cr_s} |")
                lines.append("")

            if diag.repeated_sentences:
                lines.append("### 重复内容检测\n")
                lines.append("> 以下语句在播报稿中出现了多次:\n")
                for s in diag.repeated_sentences:
                    display = f'"{s[:60]}..."' if len(s) > 60 else f'"{s}"'
                    lines.append(f"- {display}")
                lines.append("")
        else:
            lines.append("*未从 trace 中提取到最终文稿。*\n")
        lines.append("---\n")

        # ── 诊断摘要 ──
        lines.append("## 诊断摘要\n")
        if not diag.flags:
            lines.append("未发现明显问题。\n")
        else:
            for f in diag.flags:
                parts = f.split(": ", 1)
                tag = parts[0]
                desc = parts[1] if len(parts) > 1 else ""
                lines.append(f"- **{tag}**: {desc}")
            lines.append("")

        return "\n".join(lines)

    @staticmethod
    def batch(results: List[Tuple[ParsedTrace, DiagnosticResult]]) -> str:
        lines: List[str] = []
        lines.append("# FreshLoop 管线批量诊断报告\n")
        lines.append(f"- 分析时间: {datetime.datetime.now().strftime('%Y-%m-%d %H:%M')}")
        lines.append(f"- Trace 数量: {len(results)}\n")
        if not results:
            lines.append("未找到 trace 文件。\n")
            return "\n".join(lines)

        total = len(results)
        c_ok = sum(1 for _, d in results if d.clustering_ok)
        p_ok = sum(1 for _, d in results if d.planning_ok)
        covs = [d.coverage_ratio for _, d in results if d.coverage_ratio > 0]
        cprs = [d.copy_paste_ratio for _, d in results if d.copy_paste_ratio > 0]
        durs = [d.pipeline_duration_sec for _, d in results if d.pipeline_duration_sec > 0]

        lines.append("## 总体统计\n")
        lines.append("| 指标 | 值 |")
        lines.append("|------|-----|")
        lines.append(f"| 聚类成功率 | {c_ok}/{total} ({c_ok/total:.0%}) |")
        lines.append(f"| 编排成功率 | {p_ok}/{total} ({p_ok/total:.0%}) |")
        if covs:
            lines.append(f"| 平均素材覆盖率 | {sum(covs)/len(covs):.0%} |")
        if cprs:
            lines.append(f"| 平均复制粘贴率 | {sum(cprs)/len(cprs):.0%} |")
        if durs:
            lines.append(f"| 平均管线耗时 | {sum(durs)//len(durs)}s |")
        lines.append("")

        # per-category
        cats: Dict[str, dict] = {}
        for trace, diag in results:
            c = trace.category
            if c not in cats:
                cats[c] = {"n": 0, "c_ok": 0, "p_ok": 0, "covs": [], "flags": 0}
            cats[c]["n"] += 1
            if diag.clustering_ok:
                cats[c]["c_ok"] += 1
            if diag.planning_ok:
                cats[c]["p_ok"] += 1
            if diag.coverage_ratio > 0:
                cats[c]["covs"].append(diag.coverage_ratio)
            cats[c]["flags"] += len(diag.flags)

        lines.append("## 分类明细\n")
        lines.append("| 分类 | 次数 | 聚类OK | 编排OK | 平均覆盖率 | 问题数 |")
        lines.append("|------|------|--------|--------|------------|--------|")
        for cat in sorted(cats):
            s = cats[cat]
            ac = f"{sum(s['covs'])/len(s['covs']):.0%}" if s["covs"] else "N/A"
            lines.append(
                f"| {cat} | {s['n']} | {s['c_ok']}/{s['n']} | {s['p_ok']}/{s['n']} | {ac} | {s['flags']} |"
            )
        lines.append("")

        flagged = [(t, d) for t, d in results if d.flags]
        if flagged:
            lines.append("## 问题 Trace 清单\n")
            for trace, diag in flagged:
                lines.append(f"### `{os.path.basename(trace.file_path)}`")
                lines.append(f"- 分类: {trace.category}")
                for f in diag.flags:
                    lines.append(f"- {f}")
                lines.append("")

        return "\n".join(lines)


# ============================================================
# CLI Modes
# ============================================================
def _print_summary(d: DiagnosticResult):
    print("\n--- 快速诊断 ---")
    print(f"  聚类: {'OK' if d.clustering_ok else 'FALLBACK'}")
    print(f"  编排: {'OK' if d.planning_ok else 'FAILED'}")
    print(f"  覆盖率: {d.coverage_ratio:.0%}")
    print(f"  复制率: {d.copy_paste_ratio:.0%}")
    if d.flags:
        print(f"  问题数: {len(d.flags)}")
        for f in d.flags:
            print(f"    - {f}")
    else:
        print("  状态: 未发现问题")


def run_batch(args):
    print("--- FreshLoop 批量管线诊断 ---\n")
    files = TraceFinder.list_all(days=args.days)
    label = f" (最近 {args.days} 天)" if args.days else ""
    print(f"找到 {len(files)} 个 trace 文件{label}")

    results: List[Tuple[ParsedTrace, DiagnosticResult]] = []
    for i, fp in enumerate(files):
        try:
            t = TraceParser.parse_file(fp)
            d = DiagnosticAnalyzer.analyze(t)
            results.append((t, d))
            tag = "OK" if not d.flags else f"ISSUES({len(d.flags)})"
            print(f"  [{i+1}/{len(files)}] {os.path.basename(fp)}: {tag}")
        except Exception as e:
            print(f"  [{i+1}/{len(files)}] {os.path.basename(fp)}: ERROR ({e})")

    report = ReportGenerator.batch(results)
    out = args.output or "BATCH_DIAGNOSTIC.md"
    with open(out, "w", encoding="utf-8") as f:
        f.write(report)
    print(f"\n报告已保存: {os.path.abspath(out)}")

    if results:
        flagged = sum(1 for _, d in results if d.flags)
        print(f"\n总计: {len(results)} 个 trace, {flagged} 个有问题 ({flagged/len(results):.0%})")


def run_trace_direct(args):
    trace_arg = args.trace
    if os.path.isfile(trace_arg):
        filepath = trace_arg
    else:
        filepath = TraceFinder.find_by_trace_id(trace_arg)
        if not filepath:
            candidate = os.path.join(TRACE_LOG_DIR, trace_arg)
            if os.path.isfile(candidate):
                filepath = candidate
    if not filepath:
        print(f"未找到匹配的 trace 文件: {trace_arg}")
        print(f"搜索目录: {TRACE_LOG_DIR}")
        sys.exit(1)

    print(f"分析 trace: {filepath}")
    trace = TraceParser.parse_file(filepath)
    diag = DiagnosticAnalyzer.analyze(trace)
    report = ReportGenerator.single(trace, diag)

    out = args.output or f"DIAG_{trace.task_id[:8]}.md"
    with open(out, "w", encoding="utf-8") as f:
        f.write(report)
    print(f"\n报告已保存: {os.path.abspath(out)}")
    _print_summary(diag)


def run_item_mode(args):
    print("--- FreshLoop 管线诊断工具 ---\n")
    nexus = NexusClient()
    items = nexus.get_recent_items(limit=args.limit)
    if not items:
        print("无法从 Nexus 获取 items。")
        sys.exit(1)

    target = None
    if args.item_id:
        target = next((i for i in items if i["id"].startswith(args.item_id)), None)
        if not target:
            print(f"未找到 ID 前缀为 '{args.item_id}' 的 item。")
            sys.exit(1)
    else:
        print("最近的 Items:")
        for i, item in enumerate(items[:20]):
            cat = item.get("category", "?")
            ttl = item.get("title", "No Title")
            dur = item.get("duration_sec", 0)
            print(f"  [{i:2d}] [{cat}] {ttl} ({dur}s)")
        try:
            sel = input("\n选择序号 [0]: ").strip()
            idx = int(sel) if sel else 0
            target = items[idx]
        except (ValueError, IndexError):
            print("无效选择。")
            sys.exit(1)

    print(f"\n选中: {target.get('title')}")

    category = target.get("category", "")
    pub_ts = target.get("publish_time", 0)
    trace_path = TraceFinder.find_matching(category, pub_ts) if pub_ts else None

    if not trace_path:
        print(f"\n未找到匹配的 trace 文件。")
        pub_dt = datetime.datetime.fromtimestamp(pub_ts) if pub_ts else "N/A"
        print(f"分类: {category}, 发布时间: {pub_dt}")
        print(f"搜索目录: {TRACE_LOG_DIR}")
        sys.exit(1)

    print(f"匹配 trace: {os.path.basename(trace_path)}")
    trace = TraceParser.parse_file(trace_path)

    sources = nexus.get_item_sources(target.get("id", ""))
    print(f"从 Nexus DB 获取了 {len(sources)} 条原始来源。")

    diag = DiagnosticAnalyzer.analyze(trace)
    report = ReportGenerator.single(trace, diag, nexus_item=target, nexus_sources=sources)

    out = args.output or f"DIAG_{target.get('id', 'unknown')[:8]}.md"
    with open(out, "w", encoding="utf-8") as f:
        f.write(report)
    print(f"\n报告已保存: {os.path.abspath(out)}")
    _print_summary(diag)


# ============================================================
# Entry point
# ============================================================
def main():
    parser = argparse.ArgumentParser(
        description="FreshLoop Pipeline Diagnostic Tool",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
示例:
  python extract_trace.py                        # 交互式选择 Nexus item
  python extract_trace.py 2f9ed215               # 按 item ID 前缀查找
  python extract_trace.py --trace e12d7c08       # 按 trace ID 直接分析
  python extract_trace.py --trace ./trace.md     # 指定 trace 文件路径
  python extract_trace.py --batch                # 批量统计所有 trace
  python extract_trace.py --batch --days 7       # 最近 7 天统计
        """,
    )
    parser.add_argument("item_id", nargs="?", help="Nexus Item ID prefix")
    parser.add_argument("--trace", help="Trace ID prefix or file path")
    parser.add_argument("--batch", action="store_true", help="Batch analysis mode")
    parser.add_argument("--days", type=int, help="Limit batch to last N days")
    parser.add_argument("--limit", type=int, default=50, help="Nexus item fetch limit")
    parser.add_argument("--output", "-o", help="Output file path")

    args = parser.parse_args()

    if args.batch:
        run_batch(args)
    elif args.trace:
        run_trace_direct(args)
    else:
        run_item_mode(args)


if __name__ == "__main__":
    main()
