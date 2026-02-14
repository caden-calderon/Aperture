# Restart Prompt (Post-Clear)

Use this prompt after context clear:

```text
Resume Aperture Phase 3 remediation from the latest staff review.

Read in this order:
1) .context/RESUME.md
2) .context/CODE_STANDARDS.md
3) dev/active/metacog-dynamic-shifting/context.md
4) dev/active/metacog-dynamic-shifting/plan.md
5) dev/active/metacog-dynamic-shifting/tasks.md
6) dev/active/metacog-dynamic-shifting/staff-review-2026-02-13.md
7) dev/active/metacog-dynamic-shifting/design.md
8) .context/phases/phase-3.md

Then immediately start implementation of Wave 1 tasks from dev/active/metacog-dynamic-shifting/tasks.md:
- Fix planner budget ceiling plumbing.
- Fix interceptor reinvoke lifecycle ordering.
- Fix captured response body source for intercepted responses.
- Add/strengthen tests for these paths.

Constraints:
- Keep fail-open proxy behavior.
- Preserve existing architecture boundaries.
- Add tests with each fix.
- Run and report: cargo fmt --check, cargo clippy -D warnings, cargo test, npx vitest run, npm run check.

When done, update:
- dev/active/metacog-dynamic-shifting/context.md
- dev/active/metacog-dynamic-shifting/tasks.md
- .context/RESUME.md
with exact completed items and remaining blockers.
```
