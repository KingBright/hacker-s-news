# Source review and delivery

Mechanical checks bind evidence and validate structure. They never prove factual truth or actual listening quality.

Production review-check and submit require both actual independent reviewer receipts. self_review is available only for private offline previews; unavailable or unverifiable independent reviewers cannot be replaced with editor-generated pass comparisons. Read-only reviewers can return their actual JSON response to the editor for verbatim saving. They must read the complete source snapshots, using bounded file ranges when needed; inability to write a receipt file is not permission to invent a review.

`refresh --out FILE` writes FILE.checkpoint.json before calling the existing API once and retains its exact IngestReport (feeds_ok, entries_cached, errors). Repeating that invocation reads a completed bound response; started/unknown/rejected outcomes are never replayed. Partial feed errors remain explicit and block a full-coverage claim. A timed-out refresh remains unresolved until its actual result is verified. The synchronous full-feed refresh has a 300-second client bound and no automatic replay; normal requests retain 45 seconds. Cache-first-seen timestamps, full pagination, source article fetches and a published result_ref do not establish that refresh completed or that every feed succeeded. The existing helper has no refresh-resolution transition: do not create one in a scratch script, change the disposition, or rehash an event chain. Keep the original issue and report the unverified/partial freshness. Publishing and TTS submission stop while refresh is unresolved; status reads remain available.

## Preserve the candidate/depth plan before drafting

Radio uses the existing candidate ledger, now supplied with `validate --ledger RUN/candidate-ledger.json`. Bind the complete `sources --all` output, not a shortened list of selected titles. Every returned source ID needs a category, outcome and concrete reason; every configured category needs covered/empty/pending/filtered with a reason. Use a verified prior cache-first-seen boundary to avoid repeatedly treating retained historical inventory as new. Original publication dates must still be checked separately. If no verified boundary exists, retain the complete inventory and reconcile old/out-of-scope sources honestly.

Ledger shape (values must describe the actual inputs):
```json
{"inventory_file":"RUN/radio-sources.json","inventory_sha256":"JSON digest","categories":[{"name":"技术产业","outcome":"covered","reason":"实际处理结论"}],"candidates":[{"source_id":"actual ID","category":"技术产业","outcome":"included-major","event_key":"actual event","reason":"why new/relevant","importance_reason":"why it needs explanation","required_details":{"actor":"who","event":"what happened","limits":"essential boundaries, or why none apply"}}]}
```
Outcomes: included-major, included-brief, merged-duplicate (points to included event_key), already-covered-without-update, out-of-scope, unverifiable, pending. All included/merged sources of an event must be declared by its story. Major/brief is decided from substance before drafting. The first valid ledger is pinned in the existing state; changing the plan or changing a major into a brief after a validation failure is rejected. Repair the facts/evidence instead. Sparse all-brief days remain valid when the real source-based plan supports that choice. Never choose a count or word quota to make the plan pass.

For Reading, reserve editorial time at the start and choose at most three worthwhile articles; a selected article can submit before Radio while its TTS runs. Retain `reading-ledger.json` binding the complete Reading inventory and each candidate's decision. Its outcomes are published (with actual voice_job_id), already-published, not-selected (specific content reason), pending, unverifiable. Budget failure is pending, not not-selected or no-new-content. Never claim the edition complete because Radio audio alone succeeded.

## Prepare complete originals and the final units

Run mechanical validation first, then:
```sh
python3 scripts/freshloop_agent_runtime.py review-inputs --artifact RUN/radio_program.json --sources RUN/sources_content/ACTUAL_ID.json --ledger RUN/candidate-ledger.json --out RUN/review-inputs.json
```
List every actual source file. Reading/weekly do not use the Radio ledger flag. Read the generated `review-inputs.request.json`: it contains the actual input_file/input_sha256/artifact_sha256. The input digest is NOT the artifact digest. Do not guess or recalculate metadata after a review to rebind an old result.

The request also names review_batches/index.json. Give the same two original native reviewers one batch file at a time. Metadata is bounded to700 lines, up to four units per batch; each source/unit has a complete readable view and read-plan files listing ranges of at most80 lines/24KB. Read every listed range; an800-line/truncated packet read is not a full-source read. Opening/closing context is included. Reading preserves the whole original plus compressed/audio units; weekly preserves body/audio. Unavailable or unread originals mean unverifiable. Read-only reviewers return actual partial JSON; they do not write files or launch task scripts.

Each reviewer returns one JSON receipt using this exact existing schema. Save the actual response unchanged as facts-receipt.json or listening-receipt.json:
```json
{"role":"facts","artifact_sha256":"from request","input_file":"from request","input_sha256":"from request","blocking":[],"warnings":[],"checked_artifacts":["actual final file"],"summary":"specific actual review","comparisons":[{"unit":"/sections/0/stories/0/script","source_ids":["actual ID"],"checked_dimensions":["actor","event","time","numbers","limits","core_meaning","important_omissions","unsupported_additions","compression"],"conclusion":"pass","differences":[],"assessment":"concretely explain retained actor/event/limits and what may be omitted","evidence":[{"source_id":"actual ID","source_excerpt":"complete source sentence or paragraph","draft_excerpt":"complete final sentence or paragraph","observation":"specific correspondence or difference"}]}]}
```
Return one comparison for EVERY unit of the current batch and quote every contributing source. Finish all batches for both roles before acceptance. The listening receipt uses role=listening and independently assesses spoken meaning, handoffs, numbers and listener comprehension. Do not copy facts comparisons into listening. For revise/reject/unverifiable, differences lists kind/reason and concrete source/final evidence; never set pass while material omissions remain. Preserve the actual source's experimental, tentative, publication-date and comparison-scope limits; do not search unrelated history for prompt examples.

Complete sentence/paragraph anchors are required. Arbitrary `[10:20]` slices, ellipses, generic “通过审核”/“pass”, or names without their event do not establish a comparison. A valid brief may be short; brevity alone is not a failure. Do not reproduce whole papers in every receipt: quote the complete relevant sentences and explain the comparison after actually reading the full originals.

Import each real native reply (normal response or send_message to its actual parent) before assembly:
```sh
python3 scripts/freshloop_agent_runtime.py review-import --conversation ACTUAL_REVIEWER_UUID --role facts --batch RUN/.editorial/review-batches/HASH/batch-001.json --out RUN/facts-001.json
```
The importer reads the original Antigravity conversation logs in read-only mode. It verifies reviewer role, distinct author, exact input/batch binding and complete actual source/unit reads before the reply. It saves the returned judgment unchanged plus origin evidence. Production checks reverify that evidence. No editor-authored JSON can substitute for native output; this is filesystem provenance, not proof of semantic truth.

Assemble all imported parts using `--facts facts-001.json facts-002.json ... --listening listening-001.json listening-002.json ...`:
```sh
python3 scripts/freshloop_agent_runtime.py review-assemble --artifact RUN/radio_program.json --inputs RUN/review-inputs.json --facts RUN/facts-receipt.json --listening RUN/listening-receipt.json --out RUN/review.json
python3 scripts/freshloop_agent_runtime.py review-check --state RUN/radio-state.json --artifact RUN/radio_program.json --review-file RUN/review.json
```
The assembler copies records exactly, preserves their hashes and verifies them; it never fills missing comparisons, dimensions, quotations or verdicts. Incomplete receipts require the original reviewer to finish its review. Editing a manuscript invalidates both original receipts: prepare new inputs and re-review. When driven directly by Gemini task mode, assemble verified `facts` and `listening` receipts with `--mode gemini_direct` (which enforces all full-source `verify_comparison` and non-identical role checks without requiring Antigravity's local SQLite database). For private offline previews only, unavailable native tools permit honestly disclosed self-review with `--mode self_review --unavailable-reason ACTUAL_LIMITATION`. The program returns semantic_truth_verified_by_program=false.

Only then use the original `submit --state ... --artifact ... --review-file ...`. Preserve existing slots, state, leases and unknown outcomes. No extra model endpoint, Teamwork project or new scheduler.

## Traceable exceptions

Helper error events and verified transitions append to .editorial/issues.events.jsonl with a chain of digests. editorial-issues.json and run-report.md are projections. Retain original failed drafts, source snapshots and raw review receipts; approved changes carry the current input/review/artifact binding. The helper detects a manually edited projection or inconsistent event chain. Legacy summaries without original events remain unverified.

Never edit, truncate or delete either issue file to clear a failure. Never change all dispositions to corrected_and_reviewed in a generator script. Only successful current review or reconciliation of the original submission can update a disposition through the helper. Same-OS filesystem integrity checks are not a security boundary against an actor able to rewrite all files; they prevent unsupported summary changes from being treated as acceptance.

Three distinct rejected drafts (initial plus two revisions) exhaust the same job. Stop/fail the original owned job after reconciling uncertain submit outcomes, rather than creating another ID or manufacturing pass records. Keep a specific reason and pending candidates.

## Delivery of the original voice jobs

Collect all real voice_job_ids from submit, including selected Reading and any required weekly. Manifest:
```json
{"date":"YYYY-MM-DD","edition":"晚间","radio_ledger":"RUN/candidate-ledger.json","radio_state":"RUN/radio-state.json","reading_ledger":"RUN/reading-ledger.json","outputs":[{"title":"actual output","voice_job_id":"actual ID"}],"warnings":["actual pending coverage or failure"]}
```
Empty outputs require a genuine no_new_content_reason. Daily production requires the original pinned Radio ledger and reconciled Reading decisions; pending categories/candidates, unverified selection or missing selected audio block a completion report.

```sh
python3 scripts/freshloop_agent_runtime.py finalize-run --manifest RUN/delivery-manifest.json --out RUN/delivery.json --report RUN/run-report.md --wait-seconds 5400
```
This checks original voice state, product linkage, positive duration, public MP3 bytes and the latest complete natural-week weekly/Reading links. It never creates or repeats TTS. Keep the editing budget45minutes and a separate90minute audio waiting window. Running is unfinished; only exit0 plus delivered=true permits the deterministic run-report.md success report. Exit2 is failure/incomplete; preserve the same tasks.

Report date/edition, result, listening links/durations, weekly check and unresolved items. Internal IDs stay private. Audio accessibility does not prove factual quality, full ASR or client playback.
