# Aperture Phase Index

> Quick implementation index for fresh sessions.
> Canonical startup flow: read `.context/RESUME.md`, then this file, then `phase-{N}.md`.

---

## Execution Order

### Completed
1. `phase-1.md` — Proxy Core
2. `phase-1.5.md` — Stability & Modularity Hardening
3. `phase-2.md` — Context Engine

### Active
4. `phase-3.md` — **Metacognition + Dynamic Context Shifting** *(Implementation complete; remediation active)*

### Queued
5. `phase-4.md` — Dynamic Compression *(was Phase 3)*
6. `phase-5.md` — Heat, Clustering, Rebalancing *(was Phase 4)*
7. `phase-6.md` — Memory Lifecycle, Checkpoints, Forking *(was Phase 5)*
8. `phase-7.md` — Staging, Presets, Templates *(was Phase 6)*
9. `phase-8.md` — Cleaner Sidecar *(was Phase 7)*
10. `phase-9.md` — Search and NL Commands *(was Phase 8)*
11. `phase-10.md` — Analytics and Warnings *(was Phase 9)*
12. `phase-11.md` — Task Integration and Transactional Pause/Swap *(was Phase 10)*
13. `phase-12.md` — System Prompts, A/B, Git, Adaptive Learning *(was Phase 11)*
14. `phase-13.md` — Plugins and Ecosystem *(was Phase 12)*

> **Note**: `phase-3.md` has been replaced with the new metacog content.
> The old compression content (logically Phase 4) is preserved in `dev/phase-4/`.
> Phase files `phase-4.md` through `phase-12.md` have not been physically renamed yet — each is logically shifted +1.
> Physical rename will happen at next major checkpoint.
> Current remediation artifacts: `dev/metacog-design/staff-review-2026-02-13.md`, `dev/phase-4/plan.md`, `dev/phase-4/tasks.md`.

---

## Phase 3/4 Relationship

Phase 3 (metacognition + shifting) and Phase 4 (compression) are tightly coupled:
- Phase 3 builds the context planner, MCP tools, autonomous heuristics, and cleanup system
- Phase 3 has two token-reduction mechanisms of its own: **archival** (remove block from payload, keep in storage — 100% savings) and **model-authored compression** (model writes summaries via `context_plan()`)
- Phase 4 adds the **sidekick LLM** path for bulk/autonomous compression, compression queue, preserve-keys, and quality scoring
- Phase 3 can ship and demo independently — archival + model-authored summaries give the planner real budget impact without Phase 4

---

## Ownership Boundaries

- Phase 2 owns deterministic dependency tracking and baseline block versioning.
- **Phase 3 owns context tools + client adapters (MCP/proxy-injected/passive), context planner, manifest injection, ephemeral cleanup, autonomous heuristics, file mutation tracking, budget ceiling, model-authored compression, and basic relevance scoring.**
- **Phase 4 owns sidekick LLM compression, compression queue, preserve-keys, quality scoring, and sidekick backend config.** (Model-authored compression is Phase 3 scope.)
- Phase 5 owns dynamic rebalancing behavior, heat tracking, and topic clustering.
- Phase 6 owns non-destructive memory lifecycle (`hot/warm/cold/archived`), checkpoints, and forking.
- Phase 8 extends dependency tracking with semantic edges and upgrades compression-only backend into full sidecar orchestration.
- Phase 11 extends pause/swap into task-aware transactions.
- Phase 12 extends versioning with richer UX/insight workflows.

---

## Design Documents

| Document | Purpose |
|----------|---------|
| `dev/metacog-design/design.md` | Phase 3 architecture: vision, philosophy, workflows, tool surface, cleanup system |
| `dev/research/context-awareness/design.md` | Original metacognition brainstorm (superseded by above, kept for reference) |
| `dev/phase-4/` | Compression planning, token economics, diagnostic investigation |
| `dev/research/codex-proxy/` | Codex proxy research and provider models |

---

## Fresh Session Checklist

1. Confirm branch and working tree (`git status --short`).
2. Read `.context/RESUME.md` current state + next step.
3. Read current phase doc and copy its success criteria into an execution checklist.
4. Read `dev/metacog-design/design.md` for Phase 3 architecture context.
5. Run `make check` before and after implementation.
6. Update `RESUME.md` and the phase doc at checkpoint/phase completion.
