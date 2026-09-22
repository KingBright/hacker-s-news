# TTS concurrency and pipeline efficiency — 2026-09-21

## Runtime boundary

The inspected local service is still the older self-driven Cortex binary. The
external-agent implementation is in the working tree, not deployed by this
review. Current local configuration has two voice workers and two isolated TTS
processes. Moving editorial work to Antigravity only releases local LLM resources
when Cortex actually runs in `external_agent` mode and any separately resident
local model is unloaded.

## Implemented improvements

- Nexus selects and leases a voice job in one atomic SQLite `UPDATE ... RETURNING`.
  Previously concurrent workers could select the same row; the losers returned
  an empty queue even while other work remained. The returned attempt count now
  also reflects the database update. A four-worker regression covers distinct
  leases, kind filtering and empty queue behavior.
- Cortex reuses its process sampler and refreshes memory for only the worker PID.
  Previously every worker rebuilt and refreshed a full-system snapshot every
  two seconds.
- Isolated TTS children terminate when their owning async operation is dropped.
  Cancellation must not leave an orphan process competing for the same GPU.

## Repeatable benchmark

Run an optimized Cortex binary; do not use debug timings for capacity planning:

```bash
python3 scripts/benchmark_tts_concurrency.py \
  --binary "$HOME/.freshloop/bin/cortex" \
  --config config.toml \
  --output .task-work/tts-benchmark-new \
  --jobs 6 --concurrency 1 2 3
```

The output directory must be new. This uses the worker protocol directly, never
leases or publishes jobs, and stores WAVs, logs and JSON measurements locally.
It holds the batch size and text constant, includes model startup, and validates
nonempty WAVs. It excludes upload and MP3 encoding. Each job has a 360-second
limit. Optional `--text-file` allows a representative full manuscript.

Compare both batch wall time and generated audio seconds per minute. VoxCPM
outputs vary between runs; audio duration alone can reward unwanted pauses or
repetition. WAV validity is not a transcript or listening quality assessment.
RSS sampling does not measure all Metal unified memory, so it is not a safe
aggregate memory cap. Test while other GPU jobs are idle and inspect system
memory pressure. Avoid simultaneous editorial schedulers with differing job IDs.

## Measurement and recommendation

Machine: Mac13,2, 64 GiB RAM, 20 logical CPUs. Existing optimized production
VoxCPM2 Metal worker; production sampling policy (10 steps, CFG 2, max length
1024). Six identical short Chinese scripts per configuration; all batches
produced 83.36 seconds of audio without worker failures.

| Concurrent workers | Batch wall seconds | Audio seconds per wall minute | Sampled peak worker RSS |
| --- | ---: | ---: | ---: |
| 1 (clean baseline recheck) | 125.33 | 39.91 | 4.09 GiB |
| 2 | 86.32 | 57.94 | 8.40 GiB |
| 3 | 92.45 | 54.10 | 12.47 GiB |

The uncontended single-worker recheck took 125.33 seconds. Two workers
reduced batch time by 31.1% versus that baseline.
Two and three-worker measurements ran after backend compilation completed.
Three workers took 7.1% longer and used about 48% more sampled RSS than two.
Keep the current two workers/two process slots; do not raise to three or four
based on available RAM alone. These are short-job measurements from one machine,
not a long-manuscript stress test or an audio quality gate. The benchmark runs
the installed worker, so these numbers do not quantify the new queue/sampler
code's gains. Full logs and WAVs are under `.task-work/tts-throughput/`.

## Remaining opportunities, in priority order

1. Activate the external-mode rollout described in `scheduled-agent-runbook.md`;
   confirm production status and audio publication before turning off the old
   editorial schedule. Do not infer activation from installed skills alone.
2. Keep source selection bounded, reuse cached pages/media and submit each
   reviewed manuscript as soon as ready; TTS can overlap the remaining agent
   editorial work. Preserve stable job IDs to avoid duplicate audio work.
3. Match `voice_worker.concurrency` and `tts.worker_max_processes`. Increasing
   only the former can lease work that waits locally for a process slot.
4. Retain process isolation until representative long-job measurements justify
   a persistent model pool. A warm pool needs request framing, cancellation,
   crash recovery and memory recycling; it is not a free concurrency upgrade.
5. If queues grow despite sustained GPU utilization, additional independent GPU
   machines can consume the same Nexus voice queue with unique worker IDs.
   Extra processes on the same GPU cannot add hardware capacity.

No production config, service or client was changed by this optimization pass.
