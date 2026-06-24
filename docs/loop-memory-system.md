# FreshLoop Memory System

FreshLoop memory is the substrate behind `Loop`: user expression, preference
learning, profile assembly, and future personalized feed ranking. It is adapted
from Telos' memory OS, but extracted into a product-neutral backend crate so the
core memory model is not coupled to any single agent runtime.

## Product Model

FreshLoop should treat feedback as personal publishing:

- A user writes a thought, excerpt, quoted comment, or reflection into `My Loop`.
- That user expression is stored as durable memory evidence.
- A model later extracts preference signals from the expression.
- Reading, Daily Brief, Weekly Digest, and future Loop ranking consume those
  signals to personalize the next round of content.

The original user expression must remain the source of truth. Model-derived
preferences are evidence-backed derived memories, not silent overwrites.

## Memory Types

The shared memory crate supports these product-relevant types:

- `UserExpression`: raw user-authored Loop posts, comments, excerpts, and notes.
- `PreferenceSignal`: model-extracted preferences and aversions.
- `UserProfileStatic`: stable user facts and long-term preferences.
- `UserProfileDynamic`: current projects, short-lived context, and temporary goals.
- `InteractionEvent`: recent actions or conversations.
- `Semantic`: durable facts not specifically about the user.
- `Procedural`: repeatable workflows or methods.
- `Episodic`: raw experiences that may decay.

Each memory has provenance, confidence, strength, version chain fields,
relations, namespace isolation, temporal validity, optional source references,
and metadata.

## Namespace Rule

All user memory is scoped as:

```text
user:{user_id}
```

Public or anonymous memory is intentionally not supported at this layer.
Unscoped memory would make privacy and personalization errors too easy.

## Current Nexus API

Nexus owns the HTTP boundary and uses `x-user-id` for user scoping, matching the
existing reading progress model.

Loop publishing:

- `POST /api/loop/posts`: create a personal Loop post.
- `GET /api/loop/posts`: list the user's Loop feed.
- `GET /api/loop/posts/{id}`: fetch one post and its references.
- `PATCH /api/loop/posts/{id}`: update title, body, visibility, or status.
- `DELETE /api/loop/posts/{id}`: soft-delete a post and remove its active memory.

Loop posts support these post types:

- `thought`
- `quote_comment`
- `excerpt`
- `reflection`
- `observation`

References can point to articles, daily briefs, weekly digests, radio items,
audio offsets, external URLs, or other Loop posts. Creating or updating a Loop
post writes a `UserExpression` memory entry with `source_ref=loop_post:{id}`.

Memory management:

- `POST /api/memory/entries`: create a user memory entry.
- `GET /api/memory/entries`: list recent user memory entries.
- `GET /api/memory/search?q=...`: search user memory.
- `GET /api/memory/profile`: return structured profile plus prompt context.
- `DELETE /api/memory/entries/{id}`: delete one user-owned memory.

Internal preference extraction:

- `GET /api/internal/loop/posts/pending-preferences`: Cortex reads Loop posts
  whose preference extraction has not completed.
- `POST /api/internal/memory/entries`: Cortex writes derived
  `PreferenceSignal` memories.
- `GET /api/internal/memory/profile/{user_id}`: Cortex reads a budgeted memory
  profile for personalization.
- `POST /api/internal/loop/posts/{id}/preference-result`: Cortex marks the
  Loop post as `processed`, `skipped`, or `failed`.
- `POST /api/trigger/loop/preferences`: manually trigger extraction from the
  Cortex trigger API.

The default scheduled extraction times are `09:00` and `21:00` local time,
unless overridden by:

```toml
[loop_preferences]
enabled = true
schedule_times = ["09:00", "21:00"]
max_posts_per_cycle = 20
# personalization_user_id = "USER_ID"
profile_context_max_chars = 3200
```

The default store path is:

```text
~/.freshloop/data/loop_memory.redb
```

It can be overridden with `MEMORY_DB_PATH`.

## Extraction From Telos

Reused ideas from Telos:

- redb-backed local persistence
- graph-like memory relations
- version chains for conflicting facts
- namespace partitioning
- confidence and provenance
- decay for temporary memories
- profile prompt assembly
- async write queue

Intentional differences:

- No dependency on Telos DAG, mission scheduler, or model gateway.
- Embeddings are injected through `EmbeddingProvider`.
- Conflict arbitration is injected through `ConflictResolver`.
- FreshLoop-specific content starts with `UserExpression` and `PreferenceSignal`.

## Current Personalization Flow

When `[loop_preferences].personalization_user_id` is configured, Cortex fetches
that user's budgeted memory profile once per curated feed run and once per
weekly digest run.

The profile is injected into LLM prompts as selection and framing context:

- Daily curated article analysis uses it to adjust `should_publish`,
  `quality_score`, and topic tagging.
- Weekly digest generation uses it to choose the main thread, emphasis order,
  "what to watch" direction, and audio framing.

The factual boundary is explicit: personalization may change priority and
framing, but compressed article text, audio scripts, key points, and weekly
claims must remain grounded in the source article or weekly candidate material.
Personalized weekly draft caches are keyed by a hash of the user id so one
user's digest draft cannot overwrite another user's draft.

## Next Integration Steps

1. Add Loop post tables/API that write `UserExpression` memory entries with
   `source_ref` pointing to articles, briefs, weekly digests, or audio offsets.
   Done: the first backend API and storage layer now exist.
2. Add an async preference extraction worker that turns `UserExpression` entries
   into `PreferenceSignal` entries.
   Done: Cortex now has a scheduled and manually triggerable worker.
3. Inject `build_user_profile()` output into Cortex curated feed selection,
   daily brief generation, and weekly digest synthesis.
   Done: curated feed selection and weekly digest synthesis now consume the
   configured user's memory profile. Radio/Daily Brief prompt integration is
   still the next backend personalization surface.
4. Add a `Memory` / `Why this was recommended` surface so users can inspect and
   correct how FreshLoop understands them.
5. Add export/delete controls before any broader social or public sharing layer.
