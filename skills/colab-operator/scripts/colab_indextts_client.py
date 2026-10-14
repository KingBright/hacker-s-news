#!/usr/bin/env python3
"""
Local Mac orchestrator for Google Colab L4 IndexTTS-2.5 synthesis.
Guarantees automatic `colab stop` in a `finally` block so zero Compute Units are wasted.
"""
from __future__ import annotations
import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

COLAB_BIN = os.environ.get("COLAB_BIN") or shutil.which("colab") or str(Path.home() / ".local/bin/colab")
EXAMPLE_SCRIPT = str(Path(__file__).resolve().parents[1] / "examples/indextts_example.py")
DEFAULT_PROXY = os.environ.get("COLAB_PROXY") or os.environ.get("HTTPS_PROXY") or os.environ.get("HTTP_PROXY") or "http://127.0.0.1:8228"


def colab_env(proxy: str) -> dict:
    env = os.environ.copy()
    if proxy:
        env["http_proxy"] = proxy
        env["https_proxy"] = proxy
    return env


def run_colab(args_list: list[str], proxy: str, check: bool = True) -> subprocess.CompletedProcess:
    cmd = [COLAB_BIN] + args_list
    print(f"[colab-client] $ {' '.join(cmd)}", flush=True)
    res = subprocess.run(cmd, env=colab_env(proxy), text=True, capture_output=True)
    if res.stdout:
        print(res.stdout.strip(), flush=True)
    if res.stderr:
        print(res.stderr.strip(), file=sys.stderr, flush=True)
    if check and res.returncode != 0:
        raise RuntimeError(f"colab command failed (exit={res.returncode}): {' '.join(cmd)}")
    return res


def main() -> int:
    parser = argparse.ArgumentParser(description="Synthesize speech on Google Colab L4 with IndexTTS-2.5")
    parser.add_argument("--session", default="indextts-l4", help="Colab session name")
    parser.add_argument("--gpu", default="L4", help="Colab GPU type (L4, A100, T4)")
    parser.add_argument("--proxy", default=DEFAULT_PROXY, help="Local HTTP/HTTPS proxy URL")
    parser.add_argument("--spk-wav", help="Local reference speaker WAV file for single-item mode")
    parser.add_argument("--text", help="Text to synthesize for single-item mode")
    parser.add_argument("--lang", default="ZH", help="Language code (ZH, EN, JA, etc.)")
    parser.add_argument("--output", help="Local output WAV path for single-item mode")
    parser.add_argument("--manifest", help="Path to JSON manifest list of {id, spk_wav, text, lang, output_wav}")
    parser.add_argument("--keep-session", action="store_true", help="Do not stop session after completion")
    args = parser.parse_args()

    if args.manifest:
        with open(args.manifest, "r", encoding="utf-8") as f:
            local_jobs = json.load(f)
    elif args.spk_wav and args.text and args.output:
        local_jobs = [
            {
                "id": "single",
                "spk_wav": args.spk_wav,
                "text": args.text,
                "lang": args.lang,
                "output_wav": args.output,
            }
        ]
    else:
        parser.error("Provide either --manifest or (--spk-wav, --text, --output)")

    session_created = False
    try:
        run_colab(["new", "-s", args.session, "--gpu", args.gpu], args.proxy)
        session_created = True

        # Upload unique reference WAVs
        uploaded_spks = {}
        remote_jobs = []
        for idx, job in enumerate(local_jobs):
            local_spk = str(Path(job["spk_wav"]).resolve())
            if local_spk not in uploaded_spks:
                remote_spk_name = f"/content/spk_{len(uploaded_spks)}_{Path(local_spk).name}"
                run_colab(["upload", "-s", args.session, local_spk, remote_spk_name], args.proxy)
                uploaded_spks[local_spk] = remote_spk_name

            remote_out_name = f"/content/out_{idx}_{job.get('id', idx)}.wav"
            remote_jobs.append(
                {
                    "id": job.get("id", f"item_{idx}"),
                    "spk_audio_prompt": uploaded_spks[local_spk],
                    "text": job["text"],
                    "lang": job.get("lang", "ZH"),
                    "output_path": remote_out_name,
                    "_remote_name": remote_out_name,
                    "_local_output": str(Path(job["output_wav"]).resolve()),
                }
            )

        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False, encoding="utf-8") as tf:
            json.dump(
                [
                    {
                        "id": r["id"],
                        "spk_audio_prompt": r["spk_audio_prompt"],
                        "text": r["text"],
                        "lang": r["lang"],
                        "output_path": r["output_path"],
                    }
                    for r in remote_jobs
                ],
                tf,
                ensure_ascii=False,
                indent=2,
            )
            temp_jobs_path = tf.name

        try:
            run_colab(["upload", "-s", args.session, temp_jobs_path, "/content/tts_jobs.json"], args.proxy)
        finally:
            os.unlink(temp_jobs_path)

        run_colab(["exec", "-s", args.session, "-f", EXAMPLE_SCRIPT, "--timeout", "900"], args.proxy)

        for r in remote_jobs:
            Path(r["_local_output"]).parent.mkdir(parents=True, exist_ok=True)
            run_colab(["download", "-s", args.session, r["_remote_name"], r["_local_output"]], args.proxy)
            print(f"[colab-client] Saved -> {r['_local_output']}", flush=True)

        return 0
    finally:
        if session_created and not args.keep_session:
            print(f"[colab-client] Stopping session '{args.session}' to protect Compute Units...", flush=True)
            run_colab(["stop", "-s", args.session], args.proxy, check=False)


if __name__ == "__main__":
    raise SystemExit(main())
