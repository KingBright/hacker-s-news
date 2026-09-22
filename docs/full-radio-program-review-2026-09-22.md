# Complete Radio programs and production review — 2026-09-22

Radio now publishes one complete morning/evening program, with one opening,
ordered category sections, natural presenter handoffs and one closing. The user
requested remastering the most recent three editions; older audio is preserved.
Coverage remains comprehensive: major news plus brief news, with candidate
reconciliation and source evidence. Importance controls depth, not inclusion.

## Editorial and audio pipeline

A `radio_program` artifact contains opening/closing and complete category drafts.
Nexus validates each category against the existing evidence/coverage contract,
then atomically creates one item and one multi-host voice job. A rejected draft
creates neither. Identical submission reuses the same result; another completed
program for the same logical date/slot is rejected.

Cortex uses the existing host configuration:

| Host | Categories |
| --- | --- |
| Vivian | AI前沿、技术产业、生命健康; program opening/closing |
| 灿灿 | 硬件数码、游戏电竞 |
| 小萌 | 影音文娱 |
| 小凡 | 商业财经、国际时政 |
| 小何 | 科学探索、生活杂谈、其他 |

Outgoing presenters speak handoff lines; incoming presenters pick up the topic.
Within a presenter's sections, the script changes topic naturally without
repeated welcomes/dates/sign-offs. Transitions must not invent causality.

Audio generation keeps the two-process resource limit. Each finished section is
cached by text, TTS settings and voice-reference content; retries reuse it.
Sections decode one at a time into a single MP3 encoder, with short pauses,
without accumulating the entire program's PCM or joining incompatible MP3
headers. Only a successfully completed program replaces the old tracks in its
edition card. Playing an existing track is not interrupted by a quiet refresh.

## Review scope and concrete fixes

Reviewed first-party production paths across Cortex ingestion/cache/editorial
mode/TTS workers, Nexus agent/feed/item/upload/auth handlers, Web and Android
feed/playback state, and deployment scripts. This is a production-code review,
not a claim to audit every third-party dependency or experimental model fork.

| Finding | Consequence | Fix/evidence |
| --- | --- | --- |
| External worker ignored configured category hosts | Every external episode used the default voice | Program renderer resolves each category from configured hosts; unknown mappings/failed voice loading fail visibly |
| Independent episode introductions | A card sounds like separate shows | One opening/closing and edited content-specific handoffs |
| Multipart stream error discarded | A truncated long audio file could be reported successful | Explicit stream errors, temporary file, flush and atomic rename; interrupted-upload regression preserves existing audio |
| Every poll downloaded all loaded historical pages | Increasing network and UI work while browsing | Quiet refresh requests only the newest six editions; older pages remain on demand |
| Web grouping copied growing arrays repeatedly | Quadratic grouping work | One append per item; complete-program preference avoids duplicate durations/queues |
| bcrypt ran on async request threads | Login/account creation could stall unrelated requests | CPU work moved to blocking workers |
| File uploads in deployment had no retry/atomic staging | SSH interruption could leave partial binaries/configuration | Bounded retries reopen source input and atomically rename complete files; deterministic retry regression |
| Deployment generated credentials in repository root | A failed upload could leave a sensitive staging file | Private temporary directory with exit cleanup; ignore legacy generated filenames |
| Chinese date glyph 〇 was unstable in speech recognition | Opening year could be misread | Spoken-only normalization to 零; dedicated regression |
| Startup deduplication deleted original category items when a program reused their first source URL | Three original items disappeared after a restart | Removed destructive startup deduplication; programs store citations separately with no canonical article URL; transactional recovery restored all three original IDs/audio/source links, with uniqueness and replay regression coverage |
| Skill's generic example still capped Radio at three categories | A future manual run could omit categories despite the scheduled full-coverage prompt | Unified the default/example with all configured Radio categories; Reading retains its separate article budget |

## Verification

Backend tests cover program atomicity/replay, category coverage, configured-host
routing, WAV validation and interrupted uploads. Client tests cover replacement
of legacy tracks and accurate full-program duration. Flutter analysis, Web
lint/build, skill/helper/runtime tests, brand and Android release contracts run
for the release. Private logs and acceptance evidence live in
`.task-work/program-2026-09-22/`.

Web, Nexus and Android 1.3.35+44 are published. Public APK SHA-256:
`863469dd1beebc7842887d7a4d1794210b1d77130d525a7cee151d3993135462`.
Cortex uses the canonical local installer with its mandatory ASR gate.
Native Antigravity morning/evening configurations were updated without changing
IDs/times/enabled state; watched-config restart logs and prompt readback confirm
activation. Weekly remains unchanged and was verified.

The three remasters reuse previously published news bodies and source evidence;
they do not claim a new retrospective source-completeness audit. Final audio
completion and playback acceptance are recorded below.

The final Cortex installation passed seven-chunk ASR: mean 99.46%, minimum 98.01%,
no later-chunk degradation. The new program opening independently transcribed
the complete date as 2026-09-22. Ten already-rendered segments retained their
size and modification time across the upgrade, confirming cache reuse. The
restored three historical items retain 91s/108s/143s audio and 4/1/1 source links;
the three full programs retain 42/11/11 citations. Nexus tests pass 46 cases plus
the database-recovery regression. Post-deployment Feed smoke passes.

The weekly check confirms 18 unique week windows. The latest weekly digest has
184 seconds of audio, with the linked Reading item's URL and duration matching.
Native weekly scheduling remains Monday at 08:00 Asia/Shanghai.

## Final acceptance

| Edition (Asia/Shanghai) | Card duration | Audio item |
| --- | --- | --- |
| 2026-09-22 morning | 16:13 | `3c4fb5f2-f883-44de-af9e-2876c4e9409a` |
| 2026-09-21 morning | 20:23 | `754a9b8a-03c0-44d0-823a-d290a72d3f25` |
| 2026-09-21 evening | 14:06 | `b5efbca8-f467-4fa3-832c-da593bd3f529` |

All three voice jobs completed. Downloaded public MP3 files pass full FFmpeg
decoding, duration comparison and HTTP 206 range-read checks. Each edition now
returns one ready program; ten older items in the comparison page retain their
titles, audio URLs, durations and categories unchanged. Original category
records/audio are preserved, while the recent edition cards select the remaster.

Web acceptance covers 390×844 and 1365×900 layouts, automatic card updates and
new-audio hints, single-program totals, transparent expanded rows, playback of
all three programs, pause and a seek to ten minutes. Playback retained its source
through a quiet refresh. Temporary viewport overrides were reset after testing.
Android passed analysis, edition tests and the public APK release contract;
this run does not claim physical-device background-playback acceptance.

Opening/closing ASR samples confirm the dates and unified program structure.
Presenter handoff samples include Vivian→灿灿→小萌→小凡→小何 and the return to
Vivian. Sampled segment mean volumes range from −26.4 to −22.4 dB; the highest
sample peak is −0.2 dB. This is targeted audio validation, not a claim of perfect
proper-name pronunciation or a fresh fact-check of every historical story.

At completion no isolated TTS workers remained. The temporary GPU load came from
the two concurrent Metal TTS processes; the service retains its two-process cap.
