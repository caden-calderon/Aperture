# dev/ — Development Working Memory

This directory contains the working artifacts from building Aperture: design documents,
diagnostic investigations, research notes, and development history. It's part of the
development record, not the canonical project documentation — see [`docs/`](../docs/DOCS_INDEX.md)
for the authoritative reference.

## Structure

| Directory | What's Here |
|-----------|-------------|
| [`phase-4/`](phase-4/) | Current phase — token economics, refactor, bug-dive work |
| [`diagnostics/`](diagnostics/) | 10 rounds of deep-dive debugging sessions (plan layering failures) |
| [`metacog-design/`](metacog-design/) | Phase 3 design: metacognition + dynamic context shifting |
| [`research/`](research/) | Provider research (Codex proxy, provider modularity, context awareness) |
| [`audits/`](audits/) | Quality/security audits and code reviews |

## How to Read This

- **For project architecture**: start with [`docs/ARCHITECTURE.md`](../docs/ARCHITECTURE.md)
- **For the current development state**: start with [`phase-4/context.md`](phase-4/context.md)
- **For the diagnostic story**: [`diagnostics/README.md`](diagnostics/README.md) explains what happened
- **For Phase 3 design decisions**: [`metacog-design/design.md`](metacog-design/design.md) (35K — comprehensive)
