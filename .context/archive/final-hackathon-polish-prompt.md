# Aperture Final Hackathon Polish Prompt (paste after context clear)

Read these first, in order:
1. `README.md`
2. `docs/DOCS_INDEX.md`
3. `docs/DOC_LIFECYCLE.md`
4. `docs/HACKATHON_SUBMISSION.md`
5. `.context/RESUME.md`
6. `dev/active/phase-4-compression-readiness/context.md`
7. `dev/active/phase-4-compression-readiness/tasks.md`
8. `dev/active/phase-4-compression-readiness/plan.md`
9. `.context/binary-zooming-alpaca.md`

Mission:
Execute the final polish pass for hackathon submission readiness.
Prioritize behavior-stable bug fixes and repo/docs presentability.

Primary targets:
- Backend bug-dive finish:
  - persistent archival mutation matching across block ID churn
  - `context_plan(stage)` acceptance of `#`-prefixed IDs
  - temporary block disappear/reappear during tool-use subrequests
- Docs/repo final polish:
  - remove or archive stale docs safely
  - enforce docs lifecycle rules from `docs/DOC_LIFECYCLE.md`
  - keep one clear fresh-context path for new LLM sessions

Quality bar:
- Staff-level code quality, no speculative rewrites.
- Behavior-preserving unless a bug fix is intentional and tested.
- Keep changes reviewable and scoped.

Execution requirements:
1. Start with findings (severity-ordered, file refs).
2. Provide a concrete fix map before editing.
3. Implement in small increments with tests.
4. Run validation:
   - `cargo test --manifest-path src-tauri/Cargo.toml`
   - `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings`
   - frontend checks only if frontend touched
5. Update continuity docs:
   - `.context/RESUME.md`
   - `dev/active/phase-4-compression-readiness/context.md`
   - `dev/active/phase-4-compression-readiness/tasks.md`
   - `dev/active/phase-4-compression-readiness/plan.md`
   - `docs/HACKATHON_SUBMISSION.md` if status changes

Output format:
- Findings
- Fix map
- Implemented changes
- Validation results
- Residual risks + next steps

