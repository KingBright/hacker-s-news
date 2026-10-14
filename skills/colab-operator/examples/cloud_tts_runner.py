"""
Colab L4 双模型 (VoxCPM2 + IndexTTS 2.5) 云端语音克隆统一执行器
支持：
  1. voxcpm2 (openbmb/VoxCPM2, bfloat16, Ultimate Cloning, 48kHz -> 24kHz PCM_16)
  2. indextts25 (IndexTeam/IndexTTS-2.5, bf16/fp16, 口播年份与英文缩略词优化 -> 24kHz PCM_16)
  3. 自动长文本分句、坏块重试、广播级响度归一化与防爆音淡入淡出
  4. Google Drive 缓存恢复与归档 (/content/drive/MyDrive/FreshLoop_Colab_Cache)
"""
import json
import os
import subprocess
import threading
import time

DRIVE_CACHE_DIR = "/content/drive/MyDrive/FreshLoop_Colab_Cache"
INDEXTTS_TAR = f"{DRIVE_CACHE_DIR}/indextts25_checkpoints.tar"
VOXCPM2_TAR = f"{DRIVE_CACHE_DIR}/voxcpm2_checkpoints.tar"


def run_cmd_heartbeat(cmd: str, cwd: str = "/content") -> str:
    print(f"[CMD] {cmd}", flush=True)
    t0 = time.time()
    p = subprocess.Popen(
        cmd,
        shell=True,
        cwd=cwd,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )
    lines = []

    def reader():
        for line in p.stdout:
            lines.append(line)

    t = threading.Thread(target=reader, daemon=True)
    t.start()
    while p.poll() is None:
        time.sleep(5)
        print(f"  ... running ({time.time() - t0:.0f}s elapsed, {len(lines)} lines)", flush=True)
    t.join(timeout=2)
    dt = time.time() - t0
    out = "".join(lines)
    print(f"[DONE in {dt:.1f}s, exit={p.returncode}]", flush=True)
    if p.returncode != 0:
        print(out[-4000:], flush=True)
        raise RuntimeError(f"Command failed: {cmd}")
    return out


with open("/content/tts_jobs.json", "r", encoding="utf-8") as f:
    job_spec = json.load(f)

if isinstance(job_spec, list):
    engine = (job_spec[0].get("engine") if job_spec else "indextts25") or "indextts25"
else:
    engine = job_spec.get("engine", "indextts25")

engine = engine.strip().lower()
print(f"[CLOUD-TTS] Selected engine: {engine}", flush=True)

if engine == "voxcpm2":
    # 1. Ensure voxcpm package is installed in system Python
    check_voxcpm = subprocess.run(
        ["python3", "-c", "import voxcpm, soundfile, scipy"],
        capture_output=True,
        text=True,
    )
    if check_voxcpm.returncode != 0:
        run_cmd_heartbeat("pip install -q voxcpm soundfile scipy huggingface_hub")

    # 2. Ensure openbmb/VoxCPM2 weights are present
    voxcpm_dir = "/content/voxcpm2_checkpoints"
    if not os.path.exists(f"{voxcpm_dir}/model.safetensors"):
        if os.path.exists(VOXCPM2_TAR):
            print("[CACHE] Restoring VoxCPM2 checkpoints from Google Drive...", flush=True)
            os.makedirs(voxcpm_dir, exist_ok=True)
            run_cmd_heartbeat(f"tar -xf '{VOXCPM2_TAR}' -C '{voxcpm_dir}'")
        else:
            dl_cmd = (
                'python3 -c "'
                "from huggingface_hub import snapshot_download; "
                f"snapshot_download(repo_id='openbmb/VoxCPM2', local_dir='{voxcpm_dir}')"
                '"'
            )
            run_cmd_heartbeat(dl_cmd)
            if os.path.isdir("/content/drive/MyDrive"):
                os.makedirs(DRIVE_CACHE_DIR, exist_ok=True)
                print("[CACHE] Archiving VoxCPM2 checkpoints to Google Drive...", flush=True)
                run_cmd_heartbeat(f"tar -cf '{VOXCPM2_TAR}' -C '{voxcpm_dir}' .")

    voxcpm_worker = r'''
import json
import math
import os
import re
import time
import wave
import numpy as np
import soundfile as sf
from scipy.signal import resample_poly
from voxcpm import VoxCPM

TARGET_SR = 24000
MAX_CHUNK_CHARS = 150
DEFAULT_PROMPT_TEXT = "Hello 大家好，这里是 Fresh Loop 新闻速度，我们将给你带来最新鲜的新闻资讯。"


def split_sentences(text: str, max_chars: int = MAX_CHUNK_CHARS) -> list[str]:
    text = re.sub(r"\s+", " ", text).strip()
    if not text:
        return []
    parts = re.split(r"(?<=[。！？!?；;\n])", text)
    chunks = []
    current = ""
    for part in parts:
        part = part.strip()
        if not part:
            continue
        if len(current) + len(part) <= max_chars:
            current = f"{current} {part}".strip() if current else part
        else:
            if current:
                chunks.append(current)
            if len(part) <= max_chars:
                current = part
            else:
                subparts = re.split(r"(?<=[，,、：:])", part)
                subcur = ""
                for sp in subparts:
                    sp = sp.strip()
                    if not sp:
                        continue
                    if len(subcur) + len(sp) <= max_chars:
                        subcur = f"{subcur}{sp}"
                    else:
                        if subcur:
                            chunks.append(subcur)
                        subcur = sp
                current = subcur
    if current:
        chunks.append(current)
    return chunks


def postprocess_chunk(audio: np.ndarray, orig_sr: int, target_sr: int = TARGET_SR) -> np.ndarray:
    audio = np.asarray(audio, dtype=np.float32).reshape(-1)
    audio = np.nan_to_num(audio, nan=0.0, posinf=0.0, neginf=0.0)
    if audio.size == 0:
        return audio
    audio = audio - float(np.mean(audio))
    if orig_sr != target_sr:
        g = math.gcd(int(orig_sr), int(target_sr))
        audio = resample_poly(audio, int(target_sr) // g, int(orig_sr) // g).astype(np.float32)
    fade_in = min(int(0.015 * target_sr), audio.size // 4)
    fade_out = min(int(0.025 * target_sr), audio.size // 4)
    if fade_in > 0:
        curve = 0.5 * (1.0 - np.cos(np.linspace(0, np.pi, fade_in, dtype=np.float32)))
        audio[:fade_in] *= curve
    if fade_out > 0:
        curve = 0.5 * (1.0 + np.cos(np.linspace(0, np.pi, fade_out, dtype=np.float32)))
        audio[-fade_out:] *= curve
    return audio


def normalize_segment_loudness(audio: np.ndarray, target_rms: float = 0.085, max_peak: float = 0.891) -> np.ndarray:
    if audio.size == 0:
        return audio
    rms = float(np.sqrt(np.mean(np.square(audio))))
    if rms > 1e-4:
        gain = min(target_rms / rms, 4.0)
        audio = audio * gain
    peak = float(np.max(np.abs(audio)))
    if peak > max_peak:
        audio = audio * (max_peak / peak)
    return np.clip(audio, -1.0, 1.0).astype(np.float32)


t0 = time.time()
try:
    model = VoxCPM.from_pretrained("/content/voxcpm2_checkpoints", load_denoiser=False, optimize=False)
except TypeError:
    model = VoxCPM.from_pretrained("/content/voxcpm2_checkpoints", load_denoiser=False)
orig_sr = int(getattr(model.tts_model, "sample_rate", 48000))
load_seconds = round(time.time() - t0, 2)

with open("/content/tts_jobs.json", "r", encoding="utf-8") as f:
    raw = json.load(f)

jobs = raw if isinstance(raw, list) else raw.get("items", [])
results = {"engine": "voxcpm2", "load_seconds": load_seconds, "items": []}

for job in jobs:
    ts = time.time()
    out_path = job["output_path"]
    spk_wav = job["spk_audio_prompt"]
    prompt_text = job.get("prompt_text") or DEFAULT_PROMPT_TEXT
    chunks = split_sentences(job["text"])
    if not chunks:
        raise RuntimeError(f"Empty text for job {job.get('id')}")

    chunk_wavs = []
    pause = np.zeros(int(0.10 * TARGET_SR), dtype=np.float32)
    for c_idx, chunk in enumerate(chunks):
        last_err = None
        for attempt in range(3):
            try:
                wav = model.generate(
                    text=chunk,
                    prompt_wav_path=spk_wav,
                    prompt_text=prompt_text,
                    reference_wav_path=spk_wav,
                    cfg_value=float(job.get("cfg_value", 2.0)),
                    inference_timesteps=int(job.get("inference_timesteps", 10)),
                )
                proc = postprocess_chunk(wav, orig_sr, TARGET_SR)
                dur = proc.size / float(TARGET_SR)
                rms = float(np.sqrt(np.mean(np.square(proc)))) if proc.size else 0.0
                if dur < 0.25 or rms < 2e-4:
                    raise RuntimeError(f"chunk quality gate failed (dur={dur:.2f}s, rms={rms:.5f})")
                chunk_wavs.append(proc)
                if c_idx + 1 < len(chunks):
                    chunk_wavs.append(pause)
                last_err = None
                break
            except Exception as e:
                last_err = e
        if last_err is not None:
            raise RuntimeError(f"VoxCPM2 failed on chunk {c_idx + 1}/{len(chunks)}: {last_err}")

    full_audio = normalize_segment_loudness(np.concatenate(chunk_wavs))
    sf.write(out_path, full_audio, TARGET_SR, subtype="PCM_16")
    elapsed = time.time() - ts
    audio_secs = full_audio.size / float(TARGET_SR)
    results["items"].append({
        "id": job.get("id", out_path),
        "output_path": out_path,
        "chunks": len(chunks),
        "generation_seconds": round(elapsed, 2),
        "audio_seconds": round(audio_secs, 2),
        "rtf": round(elapsed / max(audio_secs, 1e-6), 3),
        "sample_rate": TARGET_SR,
    })

with open("/content/tts_results.json", "w", encoding="utf-8") as f:
    json.dump(results, f, ensure_ascii=False, indent=2)
print("TTS_RESULTS_JSON:" + json.dumps(results, ensure_ascii=False), flush=True)
'''
    with open("/content/run_voxcpm2_jobs.py", "w", encoding="utf-8") as f:
        f.write(voxcpm_worker)
    run_cmd_heartbeat("python3 /content/run_voxcpm2_jobs.py")

else:
    # IndexTTS 2.5 path
    if not os.path.exists("/content/index-tts/.git"):
        run_cmd_heartbeat("git clone --depth 1 https://github.com/index-tts/index-tts.git /content/index-tts")

    if not os.path.exists("/content/index-tts/.venv/bin/python"):
        run_cmd_heartbeat("/usr/local/bin/uv sync --python 3.11", cwd="/content/index-tts")

    if not os.path.exists("/content/index-tts/checkpoints/config.yaml"):
        if os.path.exists(INDEXTTS_TAR):
            print("[CACHE] Restoring IndexTTS-2.5 checkpoints from Google Drive...", flush=True)
            os.makedirs("/content/index-tts/checkpoints", exist_ok=True)
            run_cmd_heartbeat(f"tar -xf '{INDEXTTS_TAR}' -C /content/index-tts/checkpoints")
        else:
            dl_cmd = (
                '/content/index-tts/.venv/bin/python -c "'
                "from huggingface_hub import snapshot_download; "
                "snapshot_download(repo_id='IndexTeam/IndexTTS-2.5', local_dir='/content/index-tts/checkpoints')"
                '"'
            )
            run_cmd_heartbeat(dl_cmd, cwd="/content/index-tts")
            if os.path.isdir("/content/drive/MyDrive"):
                os.makedirs(DRIVE_CACHE_DIR, exist_ok=True)
                print("[CACHE] Archiving IndexTTS-2.5 checkpoints to Google Drive...", flush=True)
                run_cmd_heartbeat(f"tar -cf '{INDEXTTS_TAR}' -C /content/index-tts/checkpoints .")

    indextts_worker = r'''
import json
import math
import os
import re
import sys
import time
import wave
import numpy as np
import soundfile as sf
from scipy.signal import resample_poly

sys.path.insert(0, "/content/index-tts")
from indextts.infer_v2_5 import IndexTTS2

TARGET_SR = 24000
MAX_CHUNK_CHARS = 160

DIGIT_MAP = {
    "0": "零",
    "1": "一",
    "2": "二",
    "3": "三",
    "4": "四",
    "5": "五",
    "6": "六",
    "7": "七",
    "8": "八",
    "9": "九",
}

ACRONYM_MAP = {
    "NVLink": "N V Link",
    "macOS": "Mac O S",
    "iOS": "i O S",
    "iPadOS": "iPad O S",
    "SQLite": "S Q Lite",
    "PCIe": "P C I E",
    "FP8": "F P 8",
    "FP16": "F P 16",
    "BF16": "B F 16",
    "API": "A P I",
    "GPU": "G P U",
    "CPU": "C P U",
    "NPU": "N P U",
    "TPU": "T P U",
    "CLI": "C L I",
    "SDK": "S D K",
    "LLM": "L L M",
    "TTS": "T T S",
    "ASR": "A S R",
    "RAG": "R A G",
}


def normalize_for_indextts25(text: str) -> str:
    # 1. Convert 4-digit years like 2026年 -> 二零二六年 so IndexTTS 2.5 never reads 两千零二十六年
    def repl_year(match: re.Match) -> str:
        yr = match.group(1)
        return "".join(DIGIT_MAP[d] for d in yr) + "年"

    text = re.sub(r"\b(19\d{2}|20\d{2})\s*年", repl_year, text)

    # 2. Space out technical acronyms for cleaner letter-by-letter English articulation
    for token, spoken in ACRONYM_MAP.items():
        text = re.sub(rf"(?<![A-Za-z0-9]){re.escape(token)}(?![A-Za-z0-9])", spoken, text)
    return text


def split_sentences(text: str, max_chars: int = MAX_CHUNK_CHARS) -> list[str]:
    text = normalize_for_indextts25(re.sub(r"\s+", " ", text).strip())
    if not text:
        return []
    parts = re.split(r"(?<=[。！？!?；;\n])", text)
    chunks = []
    current = ""
    for part in parts:
        part = part.strip()
        if not part:
            continue
        if len(current) + len(part) <= max_chars:
            current = f"{current} {part}".strip() if current else part
        else:
            if current:
                chunks.append(current)
            if len(part) <= max_chars:
                current = part
            else:
                subparts = re.split(r"(?<=[，,、：:])", part)
                subcur = ""
                for sp in subparts:
                    sp = sp.strip()
                    if not sp:
                        continue
                    if len(subcur) + len(sp) <= max_chars:
                        subcur = f"{subcur}{sp}"
                    else:
                        if subcur:
                            chunks.append(subcur)
                        subcur = sp
                current = subcur
    if current:
        chunks.append(current)
    return chunks


def postprocess_chunk(audio: np.ndarray, orig_sr: int, target_sr: int = TARGET_SR) -> np.ndarray:
    audio = np.asarray(audio, dtype=np.float32).reshape(-1)
    audio = np.nan_to_num(audio, nan=0.0, posinf=0.0, neginf=0.0)
    if audio.size == 0:
        return audio
    audio = audio - float(np.mean(audio))
    if orig_sr != target_sr:
        g = math.gcd(int(orig_sr), int(target_sr))
        audio = resample_poly(audio, int(target_sr) // g, int(orig_sr) // g).astype(np.float32)
    fade_in = min(int(0.015 * target_sr), audio.size // 4)
    fade_out = min(int(0.025 * target_sr), audio.size // 4)
    if fade_in > 0:
        curve = 0.5 * (1.0 - np.cos(np.linspace(0, np.pi, fade_in, dtype=np.float32)))
        audio[:fade_in] *= curve
    if fade_out > 0:
        curve = 0.5 * (1.0 + np.cos(np.linspace(0, np.pi, fade_out, dtype=np.float32)))
        audio[-fade_out:] *= curve
    return audio


def normalize_segment_loudness(audio: np.ndarray, target_rms: float = 0.085, max_peak: float = 0.891) -> np.ndarray:
    if audio.size == 0:
        return audio
    rms = float(np.sqrt(np.mean(np.square(audio))))
    if rms > 1e-4:
        gain = min(target_rms / rms, 4.0)
        audio = audio * gain
    peak = float(np.max(np.abs(audio)))
    if peak > max_peak:
        audio = audio * (max_peak / peak)
    return np.clip(audio, -1.0, 1.0).astype(np.float32)


t0 = time.time()
tts = IndexTTS2(
    cfg_path="/content/index-tts/checkpoints/config.yaml",
    model_dir="/content/index-tts/checkpoints",
    use_fp16=True,
)
load_seconds = round(time.time() - t0, 2)

with open("/content/tts_jobs.json", "r", encoding="utf-8") as f:
    raw = json.load(f)

jobs = raw if isinstance(raw, list) else raw.get("items", [])
results = {"engine": "indextts25", "load_seconds": load_seconds, "items": []}

for job in jobs:
    ts = time.time()
    out_path = job["output_path"]
    spk_wav = job["spk_audio_prompt"]
    chunks = split_sentences(job["text"])
    if not chunks:
        raise RuntimeError(f"Empty text for job {job.get('id')}")

    chunk_wavs = []
    pause = np.zeros(int(0.08 * TARGET_SR), dtype=np.float32)
    tmp_chunk_path = "/content/_tmp_indextts_chunk.wav"
    for c_idx, chunk in enumerate(chunks):
        tts.infer(
            spk_audio_prompt=spk_wav,
            text=chunk,
            output_path=tmp_chunk_path,
            verbose=False,
        )
        wav_data, sr = sf.read(tmp_chunk_path, dtype="float32")
        proc = postprocess_chunk(wav_data, sr, TARGET_SR)
        chunk_wavs.append(proc)
        if c_idx + 1 < len(chunks):
            chunk_wavs.append(pause)

    full_audio = normalize_segment_loudness(np.concatenate(chunk_wavs))
    sf.write(out_path, full_audio, TARGET_SR, subtype="PCM_16")
    elapsed = time.time() - ts
    audio_secs = full_audio.size / float(TARGET_SR)
    results["items"].append({
        "id": job.get("id", out_path),
        "output_path": out_path,
        "chunks": len(chunks),
        "generation_seconds": round(elapsed, 2),
        "audio_seconds": round(audio_secs, 2),
        "rtf": round(elapsed / max(audio_secs, 1e-6), 3),
        "sample_rate": TARGET_SR,
    })

with open("/content/tts_results.json", "w", encoding="utf-8") as f:
    json.dump(results, f, ensure_ascii=False, indent=2)
print("TTS_RESULTS_JSON:" + json.dumps(results, ensure_ascii=False), flush=True)
'''
    with open("/content/index-tts/run_indextts25_jobs.py", "w", encoding="utf-8") as f:
        f.write(indextts_worker)
    run_cmd_heartbeat("/content/index-tts/.venv/bin/python /content/index-tts/run_indextts25_jobs.py", cwd="/content/index-tts")
