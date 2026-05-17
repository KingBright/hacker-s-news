# FreshLoop Agent Memory

The project should not depend on chat history alone. Use these layers to recover
context before changing code:

1. `AGENTS.md`: repo-level rules, canonical commands, risk notes, and product
   boundaries.
2. `task.md`: the active implementation checklist and verification state.
3. `docs/build-and-deploy.md`: authoritative deploy and packaging behavior.
4. `docs/freshloop-product-style.md`: UI/product language for Web and Android.
5. `~/.happy_coding/knowledge/`: long-term lessons from mistakes and debugging.

## Recovery Command

Run:

```bash
./scripts/context_snapshot.sh
```

It prints the current git state, the must-read docs, pending checklist items,
recent local lessons, and the canonical build/deploy commands. Treat its output
as the first context packet for a resumed session.

## When To Update This Memory

Update these files whenever a mistake would otherwise be easy to repeat:

- A build/deploy command was misunderstood.
- A platform-specific runtime requirement was hidden in a script.
- A UI surface drifted away from the product language.
- A risk boundary was patched with broad error handling instead of being modeled
  directly.

If the lesson is specific to this repo, update `AGENTS.md` or `docs/`. If it is
generalizable across projects, append it to `~/.happy_coding/knowledge/`.
