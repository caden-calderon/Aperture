# Aperture Deep-Dive Diagnostics Continuation Prompt (Post-Clear)

Read these first, in order:
1. `.context/RESUME.md`
2. `.context/final-hackathon-polish-prompt.md`
3. `dev/active/phase-4-compression-readiness/context.md`
4. `dev/active/phase-4-compression-readiness/tasks.md`
5. `dev/active/phase-4-compression-readiness/plan.md`
6. `dev/active/phase-4-compression-readiness/deep-dive-diagnostics-round-1-2026-02-19.md`
7. `dev/active/phase-4-compression-readiness/deep-dive-diagnostics-round-2-2026-02-19.md`
8. `dev/active/phase-4-compression-readiness/deep-dive-diagnostics-round-3-2026-02-19.md`

Mission:
Continue a staff-level diagnostics deep dive on Aperture context bugs with zero speculative fixes.

Hard constraints:
- Do not implement fixes unless root cause is proven with direct runtime evidence and tests.
- Prefer analysis, instrumentation, replay, and failing-test-first proof.
- If uncertain, classify uncertainty explicitly and gather more evidence.
- Keep edits limited to diagnostic docs/tests/harnesses unless explicitly approved for production fixes.

Current high-priority investigation targets:
- Commit says large archival savings but Claude `/context` does not go down.
- Temporary block disappear/reappear behavior during tool-heavy subrequests.
- Archive notifications firing excessively or falsely.
- Session churn / active-session flips causing UI/context instability.
- Mismatch between token bar, Aperture tool outputs, and Claude `/context`.
- Verify whether context-tool cleanup correctly handles namespaced MCP tool names.

Required workflow:
1. Gather fresh logs for the active repro run (avoid stale `/tmp` assumptions).
2. Correlate JSONL timeline, Aperture DB state, and UI event flow.
3. Build/extend minimal replay tests for any claimed bug before proposing code changes.
4. Run targeted Rust tests for each hypothesis.
5. Update diagnostics doc with severity, confidence, and proof status.

Research round requirement (in one of the next rounds):
- Use web/docs to validate assumptions for:
  - Claude Code context accounting and MCP behavior,
  - Anthropic prompt caching/invalidation boundaries,
  - Codex/OpenAI tool-history semantics and token accounting differences.
- Capture exact doc-backed constraints and compare against Aperture behavior.

Output format for each round:
- Findings (severity-ordered, with file references and evidence source)
- What was ruled out
- Open questions
- Proof status (what is proven vs suspected)
- Next diagnostic experiments

Reminder:
No more guessing. Treat this as forensic debugging until the failure modes are unambiguous.
