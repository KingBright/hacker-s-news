# Radio category digest regression — 2026-09-21

> Superseded count policy: the follow-up user requirement below replaces the
> initial minimum-three/3–6 guidance. It is retained here only as an audit trail.

The listener report was correct. The live audit found 22 Radio episodes dated
September 21, each with one source and durations of 48–143 seconds. Inspection
of saved manuscripts confirmed single-event narration. Source count alone is
not proof of independent event count. Fifteen older comparison episodes had
3–30 sources; the old Cortex aggregator explicitly required at least three
unique topics per category.

## Cause

External-agent migration preserved one episode per category, but lost the
multi-event editorial contract. Instructions focused on merging duplicate
reports of one event and did not require compiling independent events into a
category briefing. The helper and Nexus accepted nonempty sources and a short
Chinese script. Earlier acceptance verified publication/audio delivery but
missed this editorial regression. Client edition grouping and TTS did not
cause the issue.

## Fix deployed

- Skill/editorial/artifact instructions now distinguish event deduplication
  from category compilation: minimum three independent events, normally 3–6.
  Insufficient verified new events means skip/defer with a reason, not filler.
- External Radio artifacts require `stories` with distinct event keys, titles,
  substantive bodies and cited source URLs. Each body must occur in the full
  displayed script and the actual spoken script (including pronunciation
  variants). Sources must be declared and backed by editorial claim evidence;
  at least three distinct URLs must actually be referenced.
- Helper and Nexus enforce the structural contract. Semantic independence,
  relevance, factual accuracy and evidence fidelity remain editorial duties;
  three URLs or labels alone cannot prove three independent events.
- Morning/evening native Antigravity prompts were updated, saved, reopened and
  compared exactly with docs/antigravity-schedules.json. Schedules remain enabled
  at 07:30 and 18:00 Asia/Shanghai. Weekly remains enabled Monday08:00 and its
  prompt was read back unchanged. No new scheduler was added.
- Canonical `./scripts/deploy.sh --backend` completed. Live capabilities now
  advertise minimum_independent_stories=3. No client or Cortex deployment.

## Verification and limits

Python helper: 9 tests passed. Nexus: 42 tests passed. Tests reject single-story,
repeated-event, unbacked-source and incomplete spoken coverage drafts. A rejected
submission creates no episode/voice job and preserves its lease; a corrected
three-story submission creates one episode and one voice job containing the
complete script. Skill validation and live Feed smoke passed; production test
writes were skipped. Private evidence: .task-work/radio-digest-audit/.

The existing 22 episodes were not deleted, replaced or regenerated. No synthetic
program was published. The next native scheduled production has not yet run
under these rules; its semantic quality and actual listening experience remain
to be accepted against the restored contract.


## Follow-up: comprehensive coverage with depth tiers

The user clarified: cover all news, distinguishing developed major stories from
one-sentence briefs. Fixed counts would cause omissions. Current policy:

- Scan all new candidates within each assigned category/window and deduplicate
  events; include every verified relevant event with new information.
- `tier=major` develops evidence/context; `tier=brief` gives a factual sentence.
  Low importance affects depth, never eligibility. No 3–6 target or minimum3.
  Genuine 1–2-event days can publish; empty categories skip without filler.
- Keep a source/event ledger with explicit included/excluded/pending outcomes.
  Required reviewed `editorial.radio_coverage.eligible_event_keys` must match
  actual story keys exactly. Helper/Nexus reject missing eligible stories,
  duplicate keys, missing tier/evidence, or absent spoken coverage.
- Briefs allow >=10 characters; major blocks retain >=60. A legitimately lone
  brief can publish without padding. Reading/Weekly requirements are unchanged.
- Ledger accuracy and semantic eligibility are still agent editorial duties;
  the server cannot discover unreported candidates. Incomplete feeds, retrieval,
  budget or overflow must be reported, never labeled complete. The existing
  6500-character limit remains: compress depth before reporting unresolved
  overflow; do not silently drop events or create unauthorized additional slots.
- Native morning/evening prompts saved/reopened with exact match. Helper10 and
  Nexus43 tests passed, including brief and sparse-day coverage regressions.
  No client or TTS change required. Next scheduled listening acceptance pending.

The follow-up backend release completed through the canonical entrypoint after
one transient SSH timeout/retry. Live capabilities confirm `coverage_required`,
`major`/`brief` tiers and minimum1; live Feed smoke passed. Historical programs
were preserved. Evidence files use the `coverage-` prefix in the private audit
directory.
