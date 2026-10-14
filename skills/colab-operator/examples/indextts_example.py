"""
Colab L4 IndexTTS 2.5 云端语音克隆标准执行脚本（含 Google Drive 缓存恢复与心跳保活）
运行方式：
  https_proxy=http://127.0.0.1:8228 http_proxy=http://127.0.0.1:8228 \
    /Users/jinliang/.local/bin/colab exec -s indextts-l4 -f examples/indextts_example.py --timeout 600
"""
import os
import subprocess
import time
import json
import threading

DRIVE_CACHE_DIR = "/content/drive/MyDrive/FreshLoop_Colab_Cache"
CHECKPOINT_TAR = f"{DRIVE_CACHE_DIR}/indextts25_checkpoints.tar"

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
        print(out[-3000:], flush=True)
        raise RuntimeError(f"Command failed: {cmd}")
    return out

if not os.path.exists("/content/index-tts/.git"):
    run_cmd_heartbeat("git clone --depth 1 https://github.com/index-tts/index-tts.git /content/index-tts")

if not os.path.exists("/content/index-tts/.venv/bin/python"):
    run_cmd_heartbeat("/usr/local/bin/uv sync --python 3.11", cwd="/content/index-tts")

if not os.path.exists("/content/index-tts/checkpoints/config.yaml"):
    if os.path.exists(CHECKPOINT_TAR):
        print("[CACHE] Restoring IndexTTS-2.5 checkpoints from Google Drive...", flush=True)
        os.makedirs("/content/index-tts/checkpoints", exist_ok=True)
        run_cmd_heartbeat(f"tar -xf '{CHECKPOINT_TAR}' -C /content/index-tts/checkpoints")
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
            run_cmd_heartbeat(f"tar -cf '{CHECKPOINT_TAR}' -C /content/index-tts/checkpoints .")

# 若存在 /content/tts_jobs.json 则按任务清单批量合成，否则运行默认示例
infer_runner = """
import json
import os
import sys
import time
import wave

sys.path.insert(0, "/content/index-tts")
from indextts.infer_v2_5 import IndexTTS2

t0 = time.time()
tts = IndexTTS2(
    cfg_path="/content/index-tts/checkpoints/config.yaml",
    model_dir="/content/index-tts/checkpoints",
    use_bf16=True,
)
load_seconds = round(time.time() - t0, 2)

jobs_path = "/content/tts_jobs.json"
if os.path.exists(jobs_path):
    with open(jobs_path, "r", encoding="utf-8") as f:
        jobs = json.load(f)
else:
    jobs = [
        {
            "id": "default_vivian",
            "spk_audio_prompt": "/content/vivian.wav",
            "text": "欢迎收听 FreshLoop。今天关注 OpenAI、Claude 和 Gemini 的最新进展。",
            "lang": "ZH",
            "output_path": "/content/indextts25_output.wav",
        }
    ]

results = {"load_seconds": load_seconds, "items": []}
for job in jobs:
    ts = time.time()
    out_path = job["output_path"]
    tts.infer(
        spk_audio_prompt=job["spk_audio_prompt"],
        text=job["text"],
        lang=job.get("lang", "ZH"),
        output_path=out_path,
        verbose=False,
    )
    elapsed = time.time() - ts
    with wave.open(out_path, "rb") as wf:
        audio_secs = wf.getnframes() / float(wf.getframerate())
        sr = wf.getframerate()
    results["items"].append({
        "id": job.get("id", out_path),
        "output_path": out_path,
        "generation_seconds": round(elapsed, 2),
        "audio_seconds": round(audio_secs, 2),
        "rtf": round(elapsed / max(audio_secs, 1e-6), 3),
        "sample_rate": sr,
    })

with open("/content/tts_results.json", "w", encoding="utf-8") as f:
    json.dump(results, f, ensure_ascii=False, indent=2)
print("TTS_RESULTS_JSON:" + json.dumps(results, ensure_ascii=False), flush=True)
"""

with open("/content/index-tts/run_jobs.py", "w", encoding="utf-8") as f:
    f.write(infer_runner)

run_cmd_heartbeat("/content/index-tts/.venv/bin/python /content/index-tts/run_jobs.py", cwd="/content/index-tts")
