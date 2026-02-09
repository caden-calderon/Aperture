# Aperture Phase Index

> Quick implementation index for fresh sessions.
> Canonical startup flow: read `.context/RESUME.md`, then this file, then `phase-{N}.md`.

---

## Execution Order

1. `phase-1.md` — Proxy Core
2. `phase-1.5.md` — Stability & Modularity Hardening
3. `phase-2.md` — Context Engine
4. `phase-3.md` — Dynamic Compression
5. `phase-4.md` — Heat, Clustering, Rebalancing
6. `phase-5.md` — Memory Lifecycle, Checkpoints, Forking
7. `phase-6.md` — Staging, Presets, Templates
8. `phase-7.md` — Cleaner Sidecar
9. `phase-8.md` — Search and NL Commands
10. `phase-9.md` — Analytics and Warnings
11. `phase-10.md` — Task Integration and Transactional Pause/Swap
12. `phase-11.md` — System Prompts, A/B, Git, Adaptive Learning
13. `phase-12.md` — Plugins and Ecosystem

---

## Ownership Boundaries

- Phase 2 owns deterministic dependency tracking and baseline block versioning.
- Phase 3 owns compression data model, queue contract, and preserve-keys.
- Phase 4 owns dynamic rebalancing behavior (using Phase 1 hold primitives).
- Phase 5 owns non-destructive memory lifecycle (`hot/warm/cold/archived`), manifest, and recall.
- Phase 7 extends dependency tracking with semantic edges and upgrades queue/model execution to sidecar orchestration.
- Phase 10 extends pause/swap into task-aware transactions (does not reintroduce core hold primitives).
- Phase 11 extends versioning with richer UX/insight workflows.

---

## Fresh Session Checklist

1. Confirm branch and working tree (`git status --short`).
2. Read `.context/RESUME.md` current state + next step.
3. Read current phase doc and copy its success criteria into an execution checklist.
4. Run `make check` before and after implementation.
5. Update `RESUME.md` and the phase doc at checkpoint/phase completion.
