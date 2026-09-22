#!/usr/bin/env python3
"""Offline VoxCPM throughput comparison; never leases or publishes product jobs.

Use a release Cortex binary. Python 3.11+, macOS/Linux, no extra packages.
Results include model startup and WAV generation, not upload or MP3 encoding.
"""
import argparse
import concurrent.futures
import json
import pathlib
import subprocess
import threading
import time
import tomllib
import wave


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=pathlib.Path, required=True)
    parser.add_argument('--config', type=pathlib.Path, default=pathlib.Path('config.toml'))
    parser.add_argument('--output', type=pathlib.Path, required=True)
    parser.add_argument('--concurrency', type=int, nargs='+', default=[1, 2, 3])
    parser.add_argument('--jobs', type=int, default=6)
    parser.add_argument('--timeout', type=int, default=360)
    parser.add_argument('--text-file', type=pathlib.Path)
    args = parser.parse_args()
    if args.jobs < 1 or args.timeout < 1 or any(n not in range(1, 5) for n in args.concurrency):
        parser.error('jobs/timeout must be positive; concurrency must be 1..4')
    binary = args.binary.resolve(strict=True)
    args.output.mkdir(parents=True, exist_ok=False)
    output = args.output.resolve()
    with args.config.open('rb') as stream:
        tts = tomllib.load(stream)['tts']
    if tts['engine'] != 'voxcpm_metal':
        parser.error('this benchmark currently matches the production voxcpm_metal policy')
    voxcpm = dict(tts['voxcpm'])
    voxcpm.update(max_len=1024, inference_timesteps=10, cfg_value=2.0,
                  retry_badcase_ratio_threshold=6.0, control_instruction=None)
    text = args.text_file.read_text() if args.text_file else (
        '今天我们关注人工智能如何改变内容生产。可靠的数据来源和清晰的事实核查，'
        '仍然是高质量节目的基础。技术可以提升效率，但最终的内容需要帮助听众理解变化。')
    request = output / 'request.json'
    request.write_text(json.dumps({'config': {'engine': tts['engine'], 'device': tts.get('device', 'metal'),
                                              'voxcpm': voxcpm}, 'mode': 'wav', 'text': text}, ensure_ascii=False))
    lock = threading.Lock()
    active = set()
    results = []

    def job(width, index):
        prefix = output / f'{width}-{index}'
        with prefix.with_suffix('.log').open('w') as log:
            with subprocess.Popen([str(binary), 'tts-worker', str(request), str(prefix.with_suffix('.wav')),
                                   str(prefix.with_suffix('.json')), str(prefix.with_suffix('.progress'))],
                                  stdout=log, stderr=log) as process:
                with lock:
                    active.add(process.pid)
                try:
                    try:
                        code = process.wait(timeout=args.timeout)
                    except BaseException:
                        process.kill()
                        process.wait()
                        raise
                    if code:
                        raise RuntimeError(f'TTS exited {code}; see {prefix}.log')
                finally:
                    with lock:
                        active.discard(process.pid)
        with wave.open(str(prefix.with_suffix('.wav'))) as audio:
            duration = audio.getnframes() / audio.getframerate()
        if duration <= 0:
            raise RuntimeError(f'Empty audio: {prefix}')
        return duration

    for width in args.concurrency:
        start = time.monotonic()
        peak_rss = 0
        with concurrent.futures.ThreadPoolExecutor(max_workers=width) as pool:
            futures = [pool.submit(job, width, i) for i in range(args.jobs)]
            while not all(f.done() for f in futures):
                with lock:
                    pids = list(active)
                if pids:
                    sample = subprocess.run(['ps', '-o', 'rss=', '-p', ','.join(map(str, pids))],
                                            capture_output=True, text=True)
                    peak_rss = max(peak_rss, sum(int(v) for v in sample.stdout.split()))
                time.sleep(0.5)
            durations = [f.result() for f in futures]
        elapsed = time.monotonic() - start
        result = dict(concurrency=width, jobs=args.jobs, wall_seconds=round(elapsed, 2),
                      audio_seconds=round(sum(durations), 2),
                      audio_seconds_per_minute=round(sum(durations) * 60 / elapsed, 2),
                      peak_worker_rss_mib=round(peak_rss / 1024),
                      durations_seconds=[round(v, 2) for v in durations])
        results.append(result)
        (output / 'results.json').write_text(json.dumps(results, indent=2))
        print(json.dumps(result), flush=True)


if __name__ == '__main__':
    main()
