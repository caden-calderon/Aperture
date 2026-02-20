# Aperture Deep-Dive Diagnostics Continuation Prompt (Round 4, Post-Clear)

Read these first, in order:
1. `.context/RESUME.md`
2. `dev/active/phase-4-compression-readiness/context.md`
3. `dev/active/phase-4-compression-readiness/tasks.md`
4. `dev/active/phase-4-compression-readiness/plan.md`
5. `dev/active/phase-4-compression-readiness/deep-dive-diagnostics-round-1-2026-02-19.md`
6. `dev/active/phase-4-compression-readiness/deep-dive-diagnostics-round-2-2026-02-19.md`
7. `dev/active/phase-4-compression-readiness/deep-dive-diagnostics-round-3-2026-02-19.md`

Mission:
Continue a staff-level forensic diagnostics deep dive on Aperture context bugs with zero speculative fixes.

Hard constraints:
- Do not implement production fixes unless root cause is proven with direct runtime evidence and tests.
- Prefer analysis, instrumentation, replay, and failing-test-first proof.
- If uncertain, classify uncertainty explicitly and gather more evidence.
- Keep edits limited to diagnostics docs/tests/harnesses unless explicitly approved for production fixes.

Current high-priority proof tracks:
- Projection says large archival savings but rewrite applies little/no payload reduction in partial-turn archive sets.
- Cleanup of context tools with namespaced MCP names (`mcp__aperture__aperture_context_*`).
- Active-session churn and false archival toasts during auxiliary/session-interleaved traffic.
- Token-domain mismatch framing between Aperture block metrics and Claude `/context`.

Required workflow:
1. Gather fresh logs for the active repro run (avoid stale assumptions).
2. Correlate JSONL timeline, Aperture DB state, and UI event flow.
3. Extend minimal replay tests for any claimed bug before proposing code changes.
4. Run targeted Rust/frontend tests for each hypothesis.
5. Update diagnostics docs with severity, confidence, and proof status.

Research round requirement (must do in this round):
- Validate assumptions using official docs/web for:
  - Claude Code context accounting and MCP behavior,
  - Anthropic prompt caching + `cache_control` invalidation boundaries,
  - Codex/OpenAI tool-history semantics and token accounting differences.
- Capture exact doc-backed constraints and compare against Aperture behavior.

Output format:
- Findings (severity-ordered, with file references and evidence source)
- What was ruled out
- Open questions
- Proof status (proven vs suspected)
- Next diagnostic experiments

Reminder:
No guessing. Treat this as forensic debugging until failure modes are unambiguous.
