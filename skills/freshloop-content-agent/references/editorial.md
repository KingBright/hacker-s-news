# Editorial acceptance

## Source and dedup decisions

Verify complete source content, title, author/publisher and publication date. Preserve statements as attributed claims when independent verification is unavailable. An opinion, promotional benchmark or anonymous account is not an established fact. Numbers require units, currency, denominator and time window. Never translate a prediction into an outcome.

Match events by entities + action + date, not URL or title alone. Merge corroborating sources into one event, retain conflicting evidence, and state what is new relative to previously published coverage. These are two separate operations: merge multiple reports of ONE event into one story, then compile MULTIPLE independent stories in the SAME Radio category into a digest. Radio must cover ALL verified, relevant new events in its category, not a top-N selection. Importance determines narration depth: `major` explains facts, context and limits; `brief` is one clear factual sentence naming the actor and development, retaining essential attribution/uncertainty. No fixed 3–6 target, minimum-three rule or preferred total length. If only 1–2 events qualify, include them; skip only an empty category. Never split one event or add stale news to fill a quota. Introduce events clearly without invented causality. Preserve category/subscription routing.

Before selection, paginate all new candidate metadata in the assigned category/window. Keep a local coverage ledger with source IDs/URLs, event keys and outcomes: included-major, included-brief, merged-duplicate, already-covered-without-update, out-of-scope, unverifiable, or pending (fetch/budget failure). Exclusions need concrete reasons; low importance alone is not one. Compare every eligible event with the final `stories`; `editorial.radio_coverage.eligible_event_keys` must match them exactly. Set `reviewed=true` only after reconciliation. Report candidate/unique/major/brief/excluded/pending counts and pending IDs in run-report.md. Never claim complete coverage while feeds, pagination, source verification or pending items remain incomplete. Focus affects depth, never removes qualifying news.

The 6500-character transport limit remains. First shorten framing and context and use brief narration for lower-priority events; never silently delete events to fit. If full coverage still cannot fit, preserve remaining verified events in the ledger and explicitly report incomplete coverage for follow-up; do not invent extra slot IDs or claim completion.

For important claims record `claim`, `source_url`, `evidence` (short exact passage or media timestamp) in `editorial.claims`. This private job audit is separate from the listening script. A source list alone does not establish that every claim is grounded.

## Chinese listening manuscript

Write natural, concise Chinese for the audience. Preserve proper nouns and technical terms where useful, explaining unfamiliar terms on first mention. Lead with what happened and why it matters, then concrete evidence and limitations. Avoid greetings repeated per story, filler conclusions, unsupported causal transitions, rhetorical hype and full raw URLs. Attribute perspectives explicitly: “作者认为…” or “该公司公布的测试显示…”. Dates must match the run and the event; do not say “今天” for stale news or infer workdays/holidays.

The script must make sense without seeing a screen. Convert an inspected graph into the comparison, scale, units and trend; avoid “如图所示”. Explain a demonstrated sound or video action, preserving uncertainty about causes. When media is essential and inaccessible, skip rather than make up the missing observation.

A compressed Reading version is a Chinese synthesis of argument/evidence/limits, not the original text beneath a template heading. “原版” preserves the author's structure and meaning; mark a translation as such, and never label your summary as the original article. No placeholder sections, raw HTML entities, orphan formula references, split-word bullet lists or copied navigation.

## Media provenance and presentation

For each meaningful media item record `url`, `kind`, `status` and `observation`:
- `inspected`: actually viewed/listened; describe what you observed, with timestamps for clips.
- `transcript_only`: only text available; do not claim to hear tone/sounds or observe video.
- `unavailable`: inaccessible; no content claim beyond verified surrounding text.

Prefer source-owned original assets. Do not replace them with generated illustrations or rehost unauthorized recordings. Keep source attribution and working absolute HTTPS links. In `reader_markdown` / `compressed_markdown`, place images on their own line `![meaningful caption](https://...)`. Follow with a caption and source link. Put original audio/video links on their own line `[收听原始音频](https://...)` / `[观看原始视频](https://...)`; describe timestamps and why they matter. Existing clients can open these links; web additionally supports inline audio for direct audio links. These are distinct from Cortex's narrated `audio_script`.

## Final editorial gate

Score actual output on five dimensions (0–2 each): source fidelity, selection/new information, Chinese listening quality, structure/compression, and media treatment (including honest absence). Require at least 8/10 and no zero for fidelity. Do not inflate the grade to publish. Mechanically validated JSON is not proof of editorial quality. Reread the output and a representative underlying source before setting `ready=true`.

Reject or rework: unsupported facts, stale items presented as new, raw source copy as summary, missing core visual/audio evidence, inaccurate numbers, pointless merging, or duplicate content. A shorter verified program is preferable to a filled quota.
