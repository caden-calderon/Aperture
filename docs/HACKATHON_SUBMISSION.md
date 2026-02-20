# Hackathon Submission Snapshot

Last updated: 2026-02-19

## Project Summary
Aperture is a local proxy + control plane for AI coding tools. It provides visibility and control over context blocks, budget pressure, and archival planning before requests reach provider APIs.

## What Is Working
- Provider proxy path for Claude/Codex-style workflows.
- Session-aware context ingest/state tracking.
- Context planning APIs (`status`, `preview`, `plan`, `search`, `read`) through MCP tooling.
- Behavior-preserving backend refactor tranches that split major orchestration hotspots:
  - parser/rewriter/engine/planner/metacog splits
  - MCP runtime extraction
  - handler/interceptor/capture/context_api boundary cleanup
- Rust validation gates pass after the refactor tracks (`cargo test`, `cargo clippy -D warnings`).

## Known Open Issues (Honest Status)
- Manual verification is still required on a fresh Claude run to confirm persistent archival remains stable across long tool-heavy sessions.
- Manual verification is still required to confirm temporary block disappear/reappear behavior is fully resolved under tool-use subrequests.
- `/context` and Aperture budget numbers are expected to diverge by design; operator instrumentation confidence can still be improved.
- `cache_control` marker awareness in archival rewriting is still pending.

## Demo Notes
- Best demo story:
  - show context visibility and block-level suggestions,
  - show staged planning + commit flow,
  - show fail-open behavior and stable runtime even with guardrails.
- If demonstrating manually, use current prompts in:
  - `dev/active/phase-4-compression-readiness/manual-test-prompts.md`

## Run / Validation
```bash
make install
make dev
make check
```

Rust-only validation:
```bash
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
```

## Where To Read Next
1. `docs/DOCS_INDEX.md`
2. `docs/ARCHITECTURE.md`
3. `docs/INTEGRATION.md`
4. `.context/RESUME.md`
5. `dev/active/phase-4-compression-readiness/{context,tasks,plan}.md`
