### Uncertain Findings (Needs Cross-Reference)

Things that look wrong but couldn't be fully confirmed from the file being read alone.
Both agents add here. At reconciliation, each entry gets investigated and either
promoted to a confirmed bug (in the relevant audit table) or cleared with explanation.

| File | Location | Observation | What Would Confirm/Deny It | Agent | Status |
|------|----------|-------------|---------------------------|-------|--------|
| `engine/pipeline.rs` | `classify()` line 88 | Pruning candidate generation excludes only pinned blocks, so unpinned `Role::System` blocks are eligible. Not yet clear whether downstream consumers auto-apply this list or treat it as advisory UI data. | Read `engine/mod.rs` and planner/applicator call sites for `ClassificationResult.pruning_candidates`; if any automatic mutation path consumes candidates without a role filter/policy check, treat as confirmed bug. | Codex | Partially cleared: `engine/mod.rs::classify()` is a read-only query that returns `ClassificationResult` to callers without acting on `pruning_candidates`. Whether `planner/applicator.rs` auto-acts on candidates still TBD — investigate during planner reads. |
| _To be populated_ | | | | | |

**Status values**: `Unconfirmed` → `Confirmed` (add to audit table) / `Cleared` (explain why it's fine)

---

