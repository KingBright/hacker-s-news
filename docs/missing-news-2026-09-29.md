# Missing morning news — 2026-09-29

The morning Antigravity schedule dispatched at 07:36:40 Asia/Shanghai, but its
editorial run stalled in published-history retrieval around 07:38. It never
reached manuscript publication or TTS. Cortex and Nexus remained available;
the evening schedule dispatched independently at 18:10:40 and continued work.

## Cause

The agent generated an unbounded Python loop calling
`/api/items?limit=100&offset=N`, then the equivalent Reading endpoint. Both
Nexus query structs accepted `page` but silently ignored `offset`. Every
request returned the first 100 records, so neither the empty-page nor short-page
exit condition could be reached. The background process remained active for
over ten hours while the conversation waited for it.

Cortex source listing uses offset pagination, making the contract mismatch
easy to miss. The agent did not compare returned IDs or impose a scan deadline.
The scheduler's successful command status only described conversation creation.

## Repair and verification

- Radio and Reading list endpoints now honor explicit row offsets; they take
  precedence over page numbers. Negative offsets clamp to zero, while page
  arithmetic saturates instead of overflowing.
- Edition-card pagination rejects offset explicitly, rather than silently
  ignoring it.
- The shared editorial skill documents the separate pagination contracts and
  requires bounded history scans, repeated-ID detection and terminal command
  status. These are agent workflow requirements, not a new runtime watchdog.
- Nexus regression checks cover disjoint pages, end-of-list termination,
  query deserialization, precedence and numeric boundaries. All 47 Nexus tests
  plus the database test pass.
- Deployed through the canonical backend release. Live Radio and Reading
  offsets 0 and 5 return disjoint records; offset 5 matches page 2 at limit 5;
  offset 100000000 returns an empty list. Feed API smoke passes.
- The existing morning process exited and its conversation resumed source
  retrieval. Late morning catch-up versus evening-only production is a separate
  editorial choice; task dispatch or resumption is not proof of audible news.
- The latest weekly digest covers September 21–27 and already has 344 seconds
  of audio. Its scheduled window was not missed.

The September 29 evening program was published at 18:27:15, item
`32432e8d-6e16-4ce6-8b14-18ef093e03da`, voice job
`05f8a5e5-5f15-4a4f-a7fa-f6b4568a4c48`. Cortex leased it on its first attempt
and advanced through the configured presenter sections. The evening agent then
encountered a model-connection `proto: unexpected EOF` while preparing its run
report, after publication. This did not roll back the manuscript or stop Cortex;
do not resubmit the program to repair a missing report. Audio readiness must
still be verified from the voice job itself.

Private evidence is in `.task-work/missing-news-2026-09-29/`. Existing unrelated
client edits were preserved, and no client release was needed for this repair.
