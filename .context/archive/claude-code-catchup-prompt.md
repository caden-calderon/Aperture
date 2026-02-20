# Aperture Catch-Up Prompt for Claude Code (Independent Forensic Deep Dive)

Read these first, in order:
1. `.context/RESUME.md`
2. `docs/DOCS_INDEX.md`
3. `dev/active/phase-4-compression-readiness/context.md`
4. `dev/active/phase-4-compression-readiness/tasks.md`
5. `dev/active/phase-4-compression-readiness/plan.md`
6. `dev/active/phase-4-compression-readiness/deep-dive-diagnostics-round-1-2026-02-19.md`
7. `dev/active/phase-4-compression-readiness/deep-dive-diagnostics-round-2-2026-02-19.md`
8. `dev/active/phase-4-compression-readiness/deep-dive-diagnostics-round-3-2026-02-19.md`

Mission:
Run an independent staff-level forensic deep dive on Aperture context bugs, challenge current conclusions with evidence, and drive this to permanent resolution quality.

Severity and urgency:
- These are serious, user-visible correctness bugs.
- We have already run multiple fix attempts; patch-loop guessing is no longer acceptable.
- This pass must prioritize certainty over speed: code-level causality, runtime evidence, and reproducible tests only.
- The goal is to leave no ambiguity about why each failure happens and what would fully resolve it.

Critical context:
- Latest forensic repro anchor:
  - `~/.claude/projects/-home-caden-projects-Aperture/66dd683a-8f48-423a-a2e2-73338203f61e.jsonl`
  - `~/.aperture/aperture.db`
- Current proven mechanisms from round 2:
  1. Projection can report large savings while payload rewrite removes zero turns under partial-turn archive coverage.
  2. Context cleanup matcher misses namespaced MCP tool names.
  3. Auxiliary session ingests can flip active session and current UI mutation toast logic can report false archival.

Hard constraints:
- Diagnostics-first: no speculative fixes.
- If proposing a fix, include direct runtime evidence and a failing-test-first proof plan.
- Explicitly separate proven facts from hypotheses.
- Prefer small reproducible replay tests/harnesses over broad rewrites.
- No "probably", "seems", or "likely" conclusions without backing evidence.
- Treat "fixed" as a high bar: root cause proven, regression tests in place, and no identified contradiction in logs/runtime behavior.

What to evaluate independently:
1. Confirm or refute each round-2 finding with your own evidence chain.
2. Identify any higher-severity root cause we might have missed.
3. Validate assumptions against official docs for Claude Code/MCP, Anthropic caching, and OpenAI/Codex tool-history semantics.
4. Call out mismatches between Aperture metrics and Claude `/context` that are expected-by-design vs potentially buggy.
5. Surface any additional latent defects discovered during deep tracing, even if not in the current target list.

Output format:
- Findings (severity-ordered; include file refs and evidence source)
- Root-cause proof per finding (runtime evidence + test evidence + code-path evidence)
- Fix acceptance criteria per finding (what must be true before calling it resolved)
- What you disagree with (if any), and why
- What was ruled out
- Open questions
- Proof status (proven vs suspected)
- Minimal next experiments before any production fix

Note:
This is a forensic review request, not an implementation request.
