#!/usr/bin/env python3
"""Deployment-blocking TTS ASR closed-loop verification.

The gate synthesizes a long Chinese news-style script with the production
Cortex TTS binary, transcribes each generated chunk, and checks that the late
chunks do not collapse relative to the early chunks.
"""

from __future__ import annotations

import argparse
import dataclasses
import datetime as dt
import json
import os
import subprocess
import sys
from pathlib import Path
from typing import Any


DEFAULT_LONG_TEXT = """
今天的 FreshLoop 长文本闭环测试，模拟一段真实的早间新闻节目。我们先从人工智能行业说起，多个团队正在把研究智能体接入日常工作流，让信息筛选、资料整理和实验记录都变得更连续。系统不再只是回答一个问题，而是持续追踪来源、过滤噪音，并把重要变化放进用户自己的知识库。
接下来是技术产业，开发者工具正在从单点补全走向端到端协作。重点不再只是生成代码，而是理解上下文、保留决策记录，并在部署前主动暴露风险。成熟的产品会把测试、回滚、日志和用户反馈连接起来，减少那种上线之后才发现音频坏掉的情况。
商业财经方面，企业对自动化的期待已经从节省人力转向提升判断质量。管理者更关心系统能不能解释来源，能不能复盘误判，能不能持续学习偏好。一个每天运行的信息系统，如果只能把内容压缩成摘要，还不够；它需要知道什么对具体的人有用。
国际时政方面，监管机构开始要求更清晰的模型使用披露。尤其是在新闻、教育、医疗和金融场景，系统必须说明生成内容的边界，也要保留可审计的证据链。对个人信息产品来说，这意味着推荐理由、评价记录和历史偏好都应该可以追溯。
最后回到产品体验，真正有用的个人信息流不应该让用户被动刷新，而应该把阅读、标注、评价和后续行动连在一起。用户可以发布自己的心得，也可以对系统推荐的内容写下个人判断。系统再从这些判断里学习，而不是只收集一个简单的喜欢或者不喜欢。
这段测试的后半部分故意保持连续叙事，因为我们要捕获一种很隐蔽的问题：音频开头听起来正常，但模型在后续分块里逐渐退化，出现杂音、乱读、重复或和原文无关的声音。部署前必须把这种问题挡住，而不是上线后再让用户用耳朵发现。
如果整个闭环通过，说明当前生产配置至少能稳定读完一段较长文本，并且后半段没有明显掉线。如果失败，报告会指出具体 chunk、原文、识别文本和相似度，方便直接打开对应音频定位问题。
""".strip()


@dataclasses.dataclass(frozen=True)
class Thresholds:
    min_chunks: int
    min_chunk_similarity: float
    min_average_similarity: float
    min_late_average_similarity: float
    max_late_drop: float
    min_prompt_leak_similarity: float
    max_prompt_advantage: float


def configure_proxy(proxy: str | None) -> None:
    if proxy is None:
        proxy = os.environ.get("FRESHLOOP_ASR_PROXY", "http://127.0.0.1:8228")
    if not proxy:
        return
    os.environ.setdefault("http_proxy", proxy)
    os.environ.setdefault("https_proxy", proxy)


def get_pinyin_chars(text: str) -> list[str]:
    import pypinyin

    chars: list[str] = []
    for word in pypinyin.pinyin(text, style=pypinyin.Style.NORMAL, errors="default"):
        token = "".join(ch for ch in word[0].lower() if ch.isalnum())
        chars.extend(token)
    return chars


def levenshtein_distance(seq1: list[str], seq2: list[str]) -> int:
    if len(seq1) < len(seq2):
        seq1, seq2 = seq2, seq1
    previous = list(range(len(seq2) + 1))
    for i, ch1 in enumerate(seq1, start=1):
        current = [i]
        for j, ch2 in enumerate(seq2, start=1):
            insert_cost = current[j - 1] + 1
            delete_cost = previous[j] + 1
            replace_cost = previous[j - 1] + (0 if ch1 == ch2 else 1)
            current.append(min(insert_cost, delete_cost, replace_cost))
        previous = current
    return previous[-1]


def pinyin_similarity(asr_text: str, reference_text: str) -> float:
    asr_chars = get_pinyin_chars(asr_text)
    ref_chars = get_pinyin_chars(reference_text)
    if not asr_chars and not ref_chars:
        return 1.0
    if not asr_chars or not ref_chars:
        return 0.0

    n_asr = len(asr_chars)
    n_ref = len(ref_chars)
    if n_ref <= n_asr:
        distance = levenshtein_distance(asr_chars, ref_chars)
        return max(0.0, 1.0 - distance / max(n_asr, n_ref))

    max_similarity = 0.0
    min_len = max(10, n_asr - 8)
    max_len = min(n_ref, n_asr + 8)
    for length in range(min_len, max_len + 1):
        for start in range(0, n_ref - length + 1):
            ref_window = ref_chars[start : start + length]
            distance = levenshtein_distance(asr_chars, ref_window)
            similarity = 1.0 - distance / max(len(asr_chars), len(ref_window))
            max_similarity = max(max_similarity, similarity)
    return max(0.0, max_similarity)


def run_google_asr(wav_path: Path, language: str) -> str | None:
    import speech_recognition as sr

    recognizer = sr.Recognizer()
    try:
        with sr.AudioFile(str(wav_path)) as source:
            audio = recognizer.record(source)
        return recognizer.recognize_google(audio, language=language)
    except sr.UnknownValueError:
        return ""
    except sr.RequestError as exc:
        print(f"ASR request failed for {wav_path.name}: {exc}", file=sys.stderr)
        return None


def write_default_text(work_dir: Path) -> Path:
    text_path = work_dir / "long_text.txt"
    text_path.write_text(DEFAULT_LONG_TEXT, encoding="utf-8")
    return text_path


def run_synthesis(binary: Path, config: Path, output_dir: Path, text_path: Path) -> Path:
    cmd = [
        str(binary),
        "tts-asr-loop-synthesize",
        str(config),
        str(output_dir),
        str(text_path),
    ]
    print(">>> Synthesizing TTS ASR loop chunks...")
    print(" ".join(cmd))
    subprocess.run(cmd, check=True)
    manifest_path = output_dir / "manifest.json"
    if not manifest_path.exists():
        raise RuntimeError(f"Cortex did not write manifest: {manifest_path}")
    return manifest_path


def load_manifest(manifest_path: Path) -> dict[str, Any]:
    return json.loads(manifest_path.read_text(encoding="utf-8"))


def transcribe_chunks(manifest: dict[str, Any], language: str) -> list[dict[str, Any]]:
    results: list[dict[str, Any]] = []
    chunks = manifest.get("chunks", [])
    prompt_text = manifest.get("prompt_text") or ""
    for chunk in chunks:
        wav_path = Path(chunk["wav_path"])
        print(f">>> ASR chunk {chunk['index']}/{len(chunks)}: {wav_path.name}")
        asr_text = run_google_asr(wav_path, language)
        similarity = 0.0 if asr_text is None else pinyin_similarity(asr_text, chunk["text"])
        prompt_similarity = (
            0.0 if asr_text is None or not prompt_text else pinyin_similarity(asr_text, prompt_text)
        )
        result = {
            "index": chunk["index"],
            "wav_file": chunk["wav_file"],
            "wav_path": chunk["wav_path"],
            "text": chunk["text"],
            "chars": chunk["chars"],
            "duration_seconds": chunk["duration_seconds"],
            "asr_text": asr_text,
            "pinyin_similarity": similarity,
            "prompt_similarity": prompt_similarity,
        }
        results.append(result)
        rendered_asr = "<request failed>" if asr_text is None else asr_text
        print(
            f"    similarity={similarity:.2%} "
            f"prompt_similarity={prompt_similarity:.2%} asr={rendered_asr}"
        )
    return results


def average(values: list[float]) -> float:
    if not values:
        return 0.0
    return sum(values) / len(values)


def evaluate_results(results: list[dict[str, Any]], thresholds: Thresholds) -> dict[str, Any]:
    similarities = [float(item["pinyin_similarity"]) for item in results]
    midpoint = max(1, len(similarities) // 2)
    first_half = similarities[:midpoint]
    late_half = similarities[midpoint:]
    first_average = average(first_half)
    late_average = average(late_half)
    overall_average = average(similarities)
    minimum_similarity = min(similarities) if similarities else 0.0
    late_drop = max(0.0, first_average - late_average)

    failures: list[str] = []
    if len(results) < thresholds.min_chunks:
        failures.append(
            f"only {len(results)} chunks generated; need at least {thresholds.min_chunks} to test late degradation"
        )
    for item in results:
        if item["asr_text"] is None:
            failures.append(f"chunk {item['index']} ASR request failed")
        elif item["asr_text"] == "":
            failures.append(f"chunk {item['index']} ASR returned empty transcription")
        if item["pinyin_similarity"] < thresholds.min_chunk_similarity:
            failures.append(
                f"chunk {item['index']} similarity {item['pinyin_similarity']:.2%} < {thresholds.min_chunk_similarity:.2%}"
            )
        prompt_similarity = item.get("prompt_similarity", 0.0)
        if (
            prompt_similarity >= thresholds.min_prompt_leak_similarity
            and prompt_similarity > item["pinyin_similarity"] + thresholds.max_prompt_advantage
        ):
            failures.append(
                f"chunk {item['index']} looks like prompt leakage: "
                f"prompt={prompt_similarity:.2%}, target={item['pinyin_similarity']:.2%}"
            )
    if overall_average < thresholds.min_average_similarity:
        failures.append(
            f"overall similarity {overall_average:.2%} < {thresholds.min_average_similarity:.2%}"
        )
    if late_average < thresholds.min_late_average_similarity:
        failures.append(
            f"late-half similarity {late_average:.2%} < {thresholds.min_late_average_similarity:.2%}"
        )
    if late_drop > thresholds.max_late_drop:
        failures.append(
            f"late-half drop {late_drop:.2%} > {thresholds.max_late_drop:.2%} "
            f"(first={first_average:.2%}, late={late_average:.2%})"
        )

    return {
        "passed": not failures,
        "failures": failures,
        "summary": {
            "chunk_count": len(results),
            "overall_average_similarity": overall_average,
            "first_half_average_similarity": first_average,
            "late_half_average_similarity": late_average,
            "minimum_similarity": minimum_similarity,
            "late_drop": late_drop,
        },
    }


def write_report(
    report_path: Path,
    manifest_path: Path,
    manifest: dict[str, Any],
    thresholds: Thresholds,
    results: list[dict[str, Any]],
    evaluation: dict[str, Any],
) -> None:
    report = {
        "generated_at": dt.datetime.now().astimezone().isoformat(),
        "manifest_path": str(manifest_path),
        "thresholds": dataclasses.asdict(thresholds),
        "manifest": {
            "engine": manifest.get("engine"),
            "device": manifest.get("device"),
            "prompt_text": manifest.get("prompt_text"),
            "prompt_wav_path": manifest.get("prompt_wav_path"),
            "sample_rate": manifest.get("sample_rate"),
            "source_chars": manifest.get("source_chars"),
            "chunk_count": manifest.get("chunk_count"),
        },
        "evaluation": evaluation,
        "chunks": results,
    }
    report_path.write_text(json.dumps(report, indent=2, ensure_ascii=False), encoding="utf-8")


def run_self_test() -> None:
    assert pinyin_similarity("今天我们讨论人工智能产品", "今天我们讨论人工智能产品") > 0.95
    assert pinyin_similarity("天气很好适合散步", "量子计算出现新的突破") < 0.45

    thresholds = Thresholds(
        min_chunks=6,
        min_chunk_similarity=0.35,
        min_average_similarity=0.55,
        min_late_average_similarity=0.45,
        max_late_drop=0.20,
        min_prompt_leak_similarity=0.45,
        max_prompt_advantage=0.12,
    )
    good = [
        {"index": i + 1, "asr_text": "ok", "pinyin_similarity": sim, "prompt_similarity": 0.10}
        for i, sim in enumerate([0.72, 0.74, 0.70, 0.66, 0.68, 0.65])
    ]
    bad = [
        {"index": i + 1, "asr_text": "ok", "pinyin_similarity": sim, "prompt_similarity": 0.10}
        for i, sim in enumerate([0.78, 0.76, 0.74, 0.34, 0.31, 0.28])
    ]
    prompt_leak = [
        {"index": i + 1, "asr_text": "ok", "pinyin_similarity": 0.40, "prompt_similarity": 0.75}
        for i in range(6)
    ]

    assert evaluate_results(good, thresholds)["passed"]
    bad_eval = evaluate_results(bad, thresholds)
    assert not bad_eval["passed"]
    assert any("late-half drop" in failure for failure in bad_eval["failures"])
    leak_eval = evaluate_results(prompt_leak, thresholds)
    assert not leak_eval["passed"]
    assert any("prompt leakage" in failure for failure in leak_eval["failures"])
    print("self-test passed")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Verify production TTS with an ASR closed loop.")
    parser.add_argument("--binary", type=Path, default=Path("backend/target-local/release/cortex"))
    parser.add_argument("--config", type=Path, default=Path("config.toml"))
    parser.add_argument("--text-file", type=Path)
    parser.add_argument("--work-dir", type=Path)
    parser.add_argument("--language", default="zh-CN")
    parser.add_argument("--proxy", default=None, help="ASR proxy URL; set to empty string to disable")
    parser.add_argument("--skip-synthesize", action="store_true")
    parser.add_argument("--manifest", type=Path, help="Use an existing manifest when --skip-synthesize is set")
    parser.add_argument("--min-chunks", type=int, default=6)
    parser.add_argument("--min-chunk-similarity", type=float, default=0.35)
    parser.add_argument("--min-average-similarity", type=float, default=0.55)
    parser.add_argument("--min-late-average-similarity", type=float, default=0.45)
    parser.add_argument("--max-late-drop", type=float, default=0.20)
    parser.add_argument("--min-prompt-leak-similarity", type=float, default=0.45)
    parser.add_argument("--max-prompt-advantage", type=float, default=0.12)
    parser.add_argument("--self-test", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if args.self_test:
        run_self_test()
        return 0

    configure_proxy(args.proxy)
    thresholds = Thresholds(
        min_chunks=args.min_chunks,
        min_chunk_similarity=args.min_chunk_similarity,
        min_average_similarity=args.min_average_similarity,
        min_late_average_similarity=args.min_late_average_similarity,
        max_late_drop=args.max_late_drop,
        min_prompt_leak_similarity=args.min_prompt_leak_similarity,
        max_prompt_advantage=args.max_prompt_advantage,
    )

    if args.work_dir is None:
        stamp = dt.datetime.now().strftime("%Y%m%d-%H%M%S")
        args.work_dir = Path(os.environ.get("FRESHLOOP_TTS_ASR_LOOP_DIR", "/tmp/freshloop-tts-asr-loop")) / stamp
    args.work_dir.mkdir(parents=True, exist_ok=True)

    text_path = args.text_file or write_default_text(args.work_dir)
    synth_dir = args.work_dir / "synthesis"
    synth_dir.mkdir(parents=True, exist_ok=True)

    if args.skip_synthesize:
        if args.manifest is None:
            raise RuntimeError("--manifest is required with --skip-synthesize")
        manifest_path = args.manifest
    else:
        if not args.binary.exists():
            raise RuntimeError(f"Cortex binary not found: {args.binary}")
        if not args.config.exists():
            raise RuntimeError(f"Config not found: {args.config}")
        manifest_path = run_synthesis(args.binary, args.config, synth_dir, text_path)

    manifest = load_manifest(manifest_path)
    results = transcribe_chunks(manifest, args.language)
    evaluation = evaluate_results(results, thresholds)
    report_path = args.work_dir / "asr-loop-report.json"
    write_report(report_path, manifest_path, manifest, thresholds, results, evaluation)

    print(f">>> Report: {report_path}")
    summary = evaluation["summary"]
    print(
        ">>> Summary: "
        f"chunks={summary['chunk_count']} "
        f"avg={summary['overall_average_similarity']:.2%} "
        f"first={summary['first_half_average_similarity']:.2%} "
        f"late={summary['late_half_average_similarity']:.2%} "
        f"drop={summary['late_drop']:.2%} "
        f"min={summary['minimum_similarity']:.2%}"
    )
    if evaluation["passed"]:
        print("RESULT: PASS")
        return 0

    print("RESULT: FAIL")
    for failure in evaluation["failures"]:
        print(f"  - {failure}")
    return 2


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        raise SystemExit(1)
