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

## 2026-10-12: factual depth and complete candidate audit

For each major story, state the named company/person/institution and concrete development first. Then explain the source-supported specifics: what changed, dates, numbers and units, who is affected, comparison/baseline, and any material limitation. Use only details actually available. Preserve at least two distinct factual claims in editorial.claims for its cited URLs: the development and a supporting detail. This is an evidence requirement, not a character or duration target. A shorter precise explanation can pass; vague claims cannot. Do not downgrade important news merely to evade the guard. If the source cannot support it, record pending/unverifiable and the reason.

Read the actual article body through to its conclusion. Never synthesize from text[:500], the first 1000 characters, title, sidebar or navigation. Evidence excerpts must support the associated assertion, including the alleged improvement and its measured baseline. A title or “原网页确认事实” does not establish those facts. Mechanical checks catch structural regressions; fact and listening review still decide whether the explanation is clear and true.

Use `sources --product-line radio --all --since FIRST_SEEN_UNIX --out RUN/radio-sources.json` (and separately reading) for a bounded complete metadata inventory. Use the last verified edition boundary; widen it when recovering an incomplete run. `--since` filters cache first-seen time, NOT source publication date. Verify the publisher's original date in the body; a recent aggregation/feed date does not make a September release new in October. With no verified boundary, omit --since and reconcile the retained cache. Abort on repeated pages, inventory changes or budgets; preserve the incomplete ledger. Complete metadata pagination does not prove feeds fetched successfully or all sources were reviewed.

Maintain the existing coverage ledger from that inventory: every source ID/URL, category, actual publication date, event key, outcome and concrete reason. Keep failed fetches pending; do not remove them from the ledger or call an empty page a factual brief. Reconcile all configured categories, including empty/unresolved categories, against final stories. eligible_event_keys must come from this ledger, never be reset to the shortened selected list by a “fix validation” script. Report candidate/unique/major/brief/excluded/pending counts and category gaps.

Reading has three different outputs. reader_markdown preserves the author's actual structure and substantive content, or is explicitly labeled a faithful translation/excerpt; it is never two invented sentences labeled the original. compressed_markdown retains thesis, reasoning, key facts/examples and conclusion/limits in concise Chinese. audio_script explains those same facts naturally for listening, without screen references or unsupported “industry impact” commentary. Never clone reader_markdown into compressed_markdown. If a source is itself a short notice, label it accurately and do not present it as a deep article.

Weekly must explain specific findings from the actual included article contents and connect supported similarities/differences. A list of three titles and generic topic descriptions is not a digest. Insufficient substantive material means report the gap, not invent or pad. Source dates and attribution remain visible in the evidence ledger.

Reading must retain at least two distinct substantive claims with different supporting passages (e.g. thesis and example/limitation); Weekly at least three. These facts must actually be explained in the displayed and spoken versions. Title-only evidence cannot establish the article's argument. This checks evidence structure, not correctness or a manuscript length quota.
