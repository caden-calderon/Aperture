# diagnostics/ — Plan Layering Deep Dives

Twelve rounds (and counting) of systematic debugging sessions investigating a critical bug
in Aperture's metacognition system: **plan layering failure** (R9-1 / MT-1).

## The Problem

Aperture's AI can propose context mutations (archiving stale blocks, compressing old reads).
These mutations are staged in a "pending plan" and committed by the AI when ready. The bug:
second and third committed plans never fired — only the first plan persisted across turns.

**Root cause**: `commit_staged_plan_for_session()` set the pending plan but never updated
`persistent_archived_ids`. IDs were only persisted when `plan_for_session()` consumed the
pending plan. If that consumption failed (session mismatch or streaming race), the IDs were
silently lost.

**Fix**: `add_persistent_archives_for_session()` — eagerly populate `persistent_archived_ids`
at commit time, making archive intent durable regardless of whether the rewriter consumes the
pending plan. See `src-tauri/src/engine/planner/mod.rs` and `src-tauri/src/metacog/tools/plan.rs`.

## The Investigation

| Round | Focus |
|-------|-------|
| [01](round-01.md) | Initial symptom identification — second clean never fires |
| [02](round-02.md) | Session ID tracking, first hypothesis |
| [03](round-03.md) | Cold-start path investigation |
| [04](round-04.md) | Streaming race condition analysis |
| [04-consolidated](round-04-consolidated.md) | Full findings summary at round 4 |
| [05](round-05.md) | Ingest timing and block availability |
| [06](round-06.md) | Persistent archival flow trace |
| [07](round-07.md) | Mutex and concurrency analysis |
| [08](round-08.md) | Fix candidates evaluated |
| [08-investigation](round-08-investigation.md) | Deeper dive into candidate fixes |
| [09](round-09.md) | Fix implementation and diagnostic tracing |
| [10](round-10.md) | **Manual test — PASSED** (3 archive rounds, 2 successful cleans) |
| [11](round-11.md) | **REGRESSION** — 3rd round fails (2 cleans work, 3rd never fires) |
| [12](round-12.md) | **Confirmed regression** — same pattern (2 cleans, 3rd fails) |

## Current Status

**Regression confirmed in R11 + R12.** Round 10 showed 3 rounds working (8+8+5 = 21 blocks).
Rounds 11 and 12 both show the same failure: first 2 cleanup rounds fire correctly, 3rd
committed plan never fires. The 3rd plan commits successfully (MCP returns success) but the
archived block IDs never appear in persistent state.

Diagnostic `warn!()` traces (R9-DIAG) are in place. Next step: analyze R11/R12 JSONL logs
to identify why the 3rd commit fails to persist.
