# Aperture Tranche #3 Kickoff Prompt (paste after context clear)

> Historical prompt for the completed tranche #3 refactor pass.
> Current prompt for new sessions: `.context/final-hackathon-polish-prompt.md`.

Read these first, in order:
1) `.context/RESUME.md`
2) `dev/active/phase-4-compression-readiness/context.md`
3) `dev/active/phase-4-compression-readiness/tasks.md`
4) `dev/active/phase-4-compression-readiness/plan.md`
5) `.context/binary-zooming-alpaca.md`

Mission:
Continue the refactor-first track with cleanup tranche #3.
Do not start with bug-specific fixes unless a tiny safe fix naturally falls out of refactor work.

Priority tranche #3 focus:
- Complete staff-level cleanup of remaining backend orchestration hotspots:
  - `src-tauri/src/bin/aperture_mcp.rs`
  - `src-tauri/src/proxy/handler.rs`
  - `src-tauri/src/proxy/interceptor.rs`
  - `src-tauri/src/proxy/capture.rs`
  - any closely related support modules that still mix concerns or keep stale/dead pathways
- Raise architecture quality:
  - tighten module boundaries and ownership contracts,
  - standardize naming/error/context propagation/logging style,
  - reduce mixed-concern runtime file size and improve test targeting.
- Raise repo and docs quality:
  - refresh architecture map and backend ownership docs,
  - keep phase/task docs accurate and actionable,
  - leave a clean bug-dive handoff.

Standards:
- FAANG-level quality without over-engineering.
- Behavior-preserving refactors by default.
- Keep changes small and reviewable; no big-bang rewrites.
- Reduce LLM token waste from oversized mixed-concern files.

Execution requirements:
1) First produce a concrete tranche #3 refactor map (what moves where and why).
2) Implement tranche #3 end-to-end in small reviewable increments.
3) Move inline tests out of hot production files where practical.
4) Remove dead/stale pathways where safe and covered.
5) Run validation:
   - `cargo test --manifest-path src-tauri/Cargo.toml`
   - `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings`
   - frontend checks only if touched
6) Update docs when done:
   - `.context/RESUME.md`
   - `dev/active/phase-4-compression-readiness/context.md`
   - `dev/active/phase-4-compression-readiness/tasks.md`
   - `dev/active/phase-4-compression-readiness/plan.md`

Output format:
- Findings first (severity-ordered, with file references).
- Then refactor plan.
- Then implemented changes.
- Then test/validation results.
- Then residual risks and next steps.
