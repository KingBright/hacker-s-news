#!/usr/bin/env python3
"""
Unified Google Colab L4 Dual-Model (VoxCPM2 + IndexTTS 2.5) TTS Orchestrator.
Supports:
  1. Single-item CLI (--engine voxcpm2|indextts25|auto --spk-wav ... --text ... --output ...)
  2. Batch manifest CLI (--manifest jobs.json)
  3. Cortex VoiceWorker job mode (--cortex-request request.json)
  4. Multi-process queue lock with warm session handoff + guaranteed `colab stop` when queue drains.
"""
from __future__ import annotations
import argparse
import datetime
import fcntl
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import wave
from pathlib import Path
from zoneinfo import ZoneInfo

COLAB_BIN = os.environ.get("COLAB_BIN") or shutil.which("colab") or str(Path.home() / ".local/bin/colab")
RUNNER_SCRIPT = str(Path(__file__).resolve().parents[1] / "examples/cloud_tts_runner.py")
DEFAULT_PROXY = os.environ.get("COLAB_PROXY") or os.environ.get("HTTPS_PROXY") or os.environ.get("HTTP_PROXY") or "http://127.0.0.1:8228"
DEFAULT_PROMPT_TEXT = "Hello 大家好，这里是 Fresh Loop 新闻速度，我们将给你带来最新鲜的新闻资讯。"
CACHE_ROOT = Path(os.environ.get("CORTEX_DATA_DIR", str(Path.home() / ".freshloop"))) / "cache"
LOCK_FILE = CACHE_ROOT / "colab-tts.lock"
QUEUE_DIR = CACHE_ROOT / "colab-queue"


def colab_env(proxy: str) -> dict:
    env = os.environ.copy()
    if proxy:
        env["http_proxy"] = proxy
        env["https_proxy"] = proxy
    return env


def run_colab(args_list: list[str], proxy: str, check: bool = True) -> subprocess.CompletedProcess:
    cmd = [COLAB_BIN] + args_list
    print(f"[colab-tts] $ {' '.join(cmd)}", flush=True)
    res = subprocess.run(cmd, env=colab_env(proxy), text=True, capture_output=True)
    if res.stdout:
        print(res.stdout.strip(), flush=True)
    if res.stderr:
        print(res.stderr.strip(), file=sys.stderr, flush=True)
    if check and res.returncode != 0:
        raise RuntimeError(f"colab command failed (exit={res.returncode}): {' '.join(cmd)}\n{res.stderr}")
    return res


def select_engine(requested: str, job_id: str = "", voice_kind: str = "") -> str:
    req = (requested or "auto").strip().lower()
    if req in ("voxcpm2", "voxcpm", "colab_voxcpm2"):
        return "voxcpm2"
    if req in ("indextts25", "indextts", "colab_indextts25"):
        return "indextts25"

    # A/B Schedule Routing:
    # - Radio Morning (00:00 - 13:59 Asia/Shanghai): voxcpm2
    # - Radio Evening (14:00 - 23:59 Asia/Shanghai): indextts25
    # - Weekly Digest (curated_weekly / weekly_digest): indextts25
    # - Reading Article (reading_article): deterministic 50/50 split by job_id SHA256 parity
    vk = (voice_kind or "").strip().lower()
    if vk in ("curated_weekly", "weekly_digest"):
        return "indextts25"
    if vk == "reading_article" and job_id:
        digest = hashlib.sha256(job_id.encode("utf-8")).digest()
        return "voxcpm2" if (digest[0] % 2 == 0) else "indextts25"

    now_sh = datetime.datetime.now(ZoneInfo("Asia/Shanghai"))
    return "voxcpm2" if now_sh.hour < 14 else "indextts25"


def has_waiting_peers() -> bool:
    if not QUEUE_DIR.exists():
        return False
    now = time.time()
    for p in QUEUE_DIR.glob("*.waiting"):
        try:
            if now - p.stat().st_mtime > 1800:
                p.unlink(missing_ok=True)
                continue
            pid = int(p.stem)
            if pid == os.getpid():
                continue
            os.kill(pid, 0)
            return True
        except (ValueError, ProcessLookupError, FileNotFoundError):
            p.unlink(missing_ok=True)
        except PermissionError:
            return True
    return False


def ensure_session(session: str, gpu: str, proxy: str) -> None:
    ls_res = run_colab(["ls"], proxy, check=False)
    if session in (ls_res.stdout or ""):
        print(f"[colab-tts] Reusing active Colab session '{session}'", flush=True)
        return
    run_colab(["new", "-s", session, "--gpu", gpu], proxy, check=True)


def strip_file_uri(path_str: str) -> str:
    if path_str.startswith("file://"):
        return path_str[len("file://") :]
    return path_str


def validate_wav_file(path: Path) -> tuple[int, int, float]:
    with wave.open(str(path), "rb") as wf:
        channels = wf.getnchannels()
        sampwidth = wf.getsampwidth()
        rate = wf.getframerate()
        frames = wf.getnframes()
        if channels != 1 or sampwidth != 2 or rate <= 0:
            raise ValueError(f"Unsupported WAV format in {path}: ch={channels}, width={sampwidth}, sr={rate}")
        dur = frames / float(rate)
        if dur < 0.4:
            raise ValueError(f"WAV too short ({dur:.2f}s) in {path}")
        return rate, frames, dur


def stitch_wavs_to_mp3(wav_paths: list[Path], output_mp3: Path, silence_ms: int = 250) -> int:
    if not wav_paths:
        raise ValueError("No WAV files to stitch")
    output_mp3.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(suffix=".wav", delete=False) as tmp_wav_file:
        combined_wav = Path(tmp_wav_file.name)

    try:
        target_rate = None
        total_frames = 0
        with wave.open(str(combined_wav), "wb") as out_wf:
            for idx, wp in enumerate(wav_paths):
                with wave.open(str(wp), "rb") as in_wf:
                    rate = in_wf.getframerate()
                    if target_rate is None:
                        target_rate = rate
                        out_wf.setnchannels(1)
                        out_wf.setsampwidth(2)
                        out_wf.setframerate(target_rate)
                    elif rate != target_rate:
                        raise ValueError(f"Sample rate mismatch: {rate} != {target_rate}")
                    if idx > 0 and silence_ms > 0:
                        sil_frames = int(target_rate * (silence_ms / 1000.0))
                        out_wf.writeframes(b"\x00\x00" * sil_frames)
                        total_frames += sil_frames
                    frames = in_wf.getnframes()
                    out_wf.writeframes(in_wf.readframes(frames))
                    total_frames += frames

        cmd = [
            "ffmpeg",
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-i",
            str(combined_wav),
            "-codec:a",
            "libmp3lame",
            "-b:a",
            "128k",
            str(output_mp3),
        ]
        subprocess.run(cmd, check=True)
        duration_sec = int((total_frames + target_rate - 1) // target_rate)
        return max(duration_sec, 1)
    finally:
        combined_wav.unlink(missing_ok=True)


def execute_remote_batch(
    session: str,
    gpu: str,
    proxy: str,
    engine: str,
    batch_items: list[dict],
) -> None:
    if not batch_items:
        return
    ensure_session(session, gpu, proxy)

    uploaded_spks: dict[str, str] = {}
    remote_items = []
    for idx, item in enumerate(batch_items):
        local_spk = str(Path(strip_file_uri(item["spk_wav"])).resolve())
        if local_spk not in uploaded_spks:
            remote_spk = f"/content/spk_{len(uploaded_spks)}_{Path(local_spk).name}"
            run_colab(["upload", "-s", session, local_spk, remote_spk], proxy)
            uploaded_spks[local_spk] = remote_spk

        remote_out = f"/content/out_{idx}_{item['id']}.wav"
        remote_items.append(
            {
                "id": item["id"],
                "engine": engine,
                "spk_audio_prompt": uploaded_spks[local_spk],
                "prompt_text": item.get("prompt_text") or DEFAULT_PROMPT_TEXT,
                "text": item["text"],
                "output_path": remote_out,
                "_local_output": str(Path(item["output_wav"]).resolve()),
            }
        )

    payload = {
        "engine": engine,
        "items": [
            {
                "id": r["id"],
                "engine": engine,
                "spk_audio_prompt": r["spk_audio_prompt"],
                "prompt_text": r["prompt_text"],
                "text": r["text"],
                "output_path": r["output_path"],
            }
            for r in remote_items
        ],
    }

    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False, encoding="utf-8") as tf:
        json.dump(payload, tf, ensure_ascii=False, indent=2)
        temp_jobs_path = tf.name

    try:
        run_colab(["upload", "-s", session, temp_jobs_path, "/content/tts_jobs.json"], proxy)
    finally:
        os.unlink(temp_jobs_path)

    run_colab(["exec", "-s", session, "-f", RUNNER_SCRIPT, "--timeout", "1200"], proxy)

    for r in remote_items:
        local_out = Path(r["_local_output"])
        local_out.parent.mkdir(parents=True, exist_ok=True)
        temp_dl = local_out.with_suffix(f".{os.getpid()}.tmp.wav")
        run_colab(["download", "-s", session, r["output_path"], str(temp_dl)], proxy)
        validate_wav_file(temp_dl)
        temp_dl.replace(local_out)
        print(f"[colab-tts] Verified & saved WAV -> {local_out}", flush=True)


def resolve_host(hosts: list[dict], category: str | None, default_voice: str, default_prompt: str) -> tuple[str, str, str]:
    if category:
        for h in hosts:
            if category in (h.get("categories") or []):
                return (
                    h.get("name", "host"),
                    h["voice"],
                    h.get("prompt_text") or default_prompt,
                )
        raise RuntimeError(f"No configured host for category '{category}'")
    if hosts:
        h = hosts[0]
        return (
            h.get("name", "lead"),
            h["voice"],
            h.get("prompt_text") or default_prompt,
        )
    return ("default", default_voice, default_prompt)


def handle_cortex_request(req_path: Path, session: str, gpu: str, proxy: str) -> int:
    req = json.loads(req_path.read_text(encoding="utf-8"))
    job_id = req.get("job_id", "job")
    voice_kind = req.get("voice_kind", "reading_article")
    engine = select_engine(req.get("engine", "auto"), job_id=job_id, voice_kind=voice_kind)
    repo_root = Path(__file__).resolve().parents[3]
    fallback_voice = repo_root / "audio/vivian.wav"
    default_voice = req.get("default_voice") or (
        str(fallback_voice) if fallback_voice.exists() else "/Users/jinliang/workspace/hacker-s-news/audio/vivian.wav"
    )
    default_prompt = req.get("default_prompt_text") or DEFAULT_PROMPT_TEXT
    output_mp3 = Path(req["output_mp3"])
    output_meta = Path(req["output_meta"])
    seg_cache_dir = Path(req.get("segment_cache_dir") or output_mp3.parent / "segments")
    seg_cache_dir.mkdir(parents=True, exist_ok=True)

    if voice_kind == "radio_program":
        plan = json.loads(req["text"])
        segments = plan.get("segments") or []
        if len(segments) < 3 or len(segments) > 32:
            raise RuntimeError(f"Invalid radio_program segment count: {len(segments)}")
        hosts = req.get("hosts") or []
        ordered_wavs: list[Path] = []
        pending_batch: list[dict] = []

        for idx, seg in enumerate(segments):
            cat = seg.get("category")
            text = (seg.get("text") or "").strip()
            if not text:
                raise RuntimeError(f"Empty radio_program segment at index {idx}")
            host_name, host_voice, host_prompt = resolve_host(hosts, cat, default_voice, default_prompt)
            ref_bytes = Path(strip_file_uri(host_voice)).read_bytes()
            ref_hash = hashlib.sha256(ref_bytes).hexdigest()
            cache_key = hashlib.sha256(
                f"colab:{engine}\n{host_voice}\n{host_prompt}\n{ref_hash}\n{text}".encode("utf-8")
            ).hexdigest()
            seg_wav = seg_cache_dir / f"{cache_key}.wav"
            ordered_wavs.append(seg_wav)

            if seg_wav.exists():
                try:
                    validate_wav_file(seg_wav)
                    print(f"[colab-tts] Segment {idx + 1}/{len(segments)} ({host_name}) hit local cache", flush=True)
                    continue
                except Exception:
                    seg_wav.unlink(missing_ok=True)

            pending_batch.append(
                {
                    "id": f"seg_{idx:02d}",
                    "spk_wav": host_voice,
                    "prompt_text": host_prompt,
                    "text": text,
                    "output_wav": str(seg_wav),
                }
            )

        if pending_batch:
            print(
                f"[colab-tts] Synthesizing {len(pending_batch)}/{len(segments)} radio segments on Colab L4 ({engine})...",
                flush=True,
            )
            execute_remote_batch(session, gpu, proxy, engine, pending_batch)

        duration_sec = stitch_wavs_to_mp3(ordered_wavs, output_mp3, silence_ms=250)
    else:
        text = (req.get("text") or "").strip()
        if not text:
            raise RuntimeError("Empty text for TTS job")
        hosts = req.get("hosts") or []
        _, lead_voice, lead_prompt = resolve_host(hosts, None, default_voice, default_prompt)
        cache_key = hashlib.sha256(f"colab:{engine}\n{lead_voice}\n{lead_prompt}\n{text}".encode("utf-8")).hexdigest()
        article_wav = seg_cache_dir / f"{cache_key}.wav"
        need_synth = True
        if article_wav.exists():
            try:
                validate_wav_file(article_wav)
                need_synth = False
            except Exception:
                article_wav.unlink(missing_ok=True)
        if need_synth:
            execute_remote_batch(
                session,
                gpu,
                proxy,
                engine,
                [
                    {
                        "id": "article",
                        "spk_wav": lead_voice,
                        "prompt_text": lead_prompt,
                        "text": text,
                        "output_wav": str(article_wav),
                    }
                ],
            )
        duration_sec = stitch_wavs_to_mp3([article_wav], output_mp3, silence_ms=0)

    meta = {
        "job_id": job_id,
        "engine": engine,
        "provider": f"colab_{engine}",
        "duration_seconds": duration_sec,
        "sample_rate": 24000,
    }
    output_meta.parent.mkdir(parents=True, exist_ok=True)
    output_meta.write_text(json.dumps(meta, ensure_ascii=False, indent=2), encoding="utf-8")
    print(f"[colab-tts] Completed job={job_id} provider=colab_{engine} duration={duration_sec}s", flush=True)
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description="Unified Colab L4 Dual-Model TTS Client (VoxCPM2 + IndexTTS 2.5)")
    parser.add_argument("--session", default="freshloop-tts-l4", help="Colab session name")
    parser.add_argument("--gpu", default="L4", help="Colab GPU type (L4, A100, T4)")
    parser.add_argument("--proxy", default=DEFAULT_PROXY, help="Local HTTP/HTTPS proxy URL")
    parser.add_argument("--engine", default="auto", help="voxcpm2 | indextts25 | auto")
    parser.add_argument("--voice-kind", default="reading_article", help="radio_program | reading_article | curated_weekly")
    parser.add_argument("--job-id", default="", help="Optional job ID for deterministic A/B routing")
    parser.add_argument("--spk-wav", help="Local reference speaker WAV file for single-item mode")
    parser.add_argument("--prompt-text", default=DEFAULT_PROMPT_TEXT, help="Reference audio transcript")
    parser.add_argument("--text", help="Text to synthesize for single-item mode")
    parser.add_argument("--output", help="Local output WAV or MP3 path for single-item mode")
    parser.add_argument("--manifest", help="Path to JSON manifest list of {id, spk_wav, prompt_text, text, output_wav}")
    parser.add_argument("--cortex-request", help="Path to Cortex voice job request JSON")
    parser.add_argument("--keep-session", action="store_true", help="Do not stop session after completion")
    args = parser.parse_args()

    CACHE_ROOT.mkdir(parents=True, exist_ok=True)
    QUEUE_DIR.mkdir(parents=True, exist_ok=True)
    waiting_marker = QUEUE_DIR / f"{os.getpid()}.waiting"
    waiting_marker.write_text(str(time.time()), encoding="utf-8")

    lock_fd = open(LOCK_FILE, "w", encoding="utf-8")
    try:
        fcntl.flock(lock_fd, fcntl.LOCK_EX)
        waiting_marker.unlink(missing_ok=True)

        if args.cortex_request:
            return handle_cortex_request(Path(args.cortex_request), args.session, args.gpu, args.proxy)

        engine = select_engine(args.engine, job_id=args.job_id, voice_kind=args.voice_kind)
        if args.manifest:
            items = json.loads(Path(args.manifest).read_text(encoding="utf-8"))
        elif args.spk_wav and args.text and args.output:
            items = [
                {
                    "id": "single",
                    "spk_wav": args.spk_wav,
                    "prompt_text": args.prompt_text,
                    "text": args.text,
                    "output_wav": args.output,
                }
            ]
        else:
            parser.error("Provide --cortex-request, --manifest, or (--spk-wav, --text, --output)")

        execute_remote_batch(args.session, args.gpu, args.proxy, engine, items)
        return 0
    finally:
        waiting_marker.unlink(missing_ok=True)
        if not args.keep_session:
            if has_waiting_peers():
                print(
                    f"[colab-tts] Another TTS job is waiting in queue; keeping session '{args.session}' warm for handoff.",
                    flush=True,
                )
            else:
                print(
                    f"[colab-tts] Queue drained; stopping session '{args.session}' immediately to save Compute Units...",
                    flush=True,
                )
                run_colab(["stop", "-s", args.session], args.proxy, check=False)
        try:
            fcntl.flock(lock_fd, fcntl.LOCK_UN)
        finally:
            lock_fd.close()


if __name__ == "__main__":
    raise SystemExit(main())
