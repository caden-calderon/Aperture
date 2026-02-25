# Documentation Index

Last updated: 2026-02-24

This is the canonical navigation map for Aperture documentation.
When in doubt, prefer `docs/` over `dev/` over `.context/`.

## Start Here

| Goal | Where to Go |
|------|-------------|
| Understand the project | [`docs/OVERVIEW.md`](OVERVIEW.md) |
| Quick setup | [`README.md`](../README.md) |
| Hackathon submission snapshot | [`docs/HACKATHON_SUBMISSION.md`](HACKATHON_SUBMISSION.md) |
| System architecture | [`docs/ARCHITECTURE.md`](ARCHITECTURE.md) |
| Current dev state | [`dev/phase-4/context.md`](../dev/phase-4/context.md) |

## Canonical Reference Docs (`docs/`)

| Doc | What It Covers |
|-----|----------------|
| `OVERVIEW.md` | Project motivation, how it works, workflows, roadmap |
| `ARCHITECTURE.md` | Three-layer architecture, module ownership, design decisions |
| `INTEGRATION.md` | Frontend/backend IPC contracts, Tauri events, localStorage |
| `REPO_STRUCTURE.md` | Code layout and module ownership |
| `PROVIDER_CAPABILITY_MATRIX.md` | Provider/runtime capability baseline |
| `SECURITY_BASELINE.md` | Security constraints and hardening requirements |
| `DOC_LIFECYCLE.md` | Doc location, naming, archival, and update rules |
| `HACKATHON_SUBMISSION.md` | Submission snapshot — what works, known issues, demo guide |

## Phase Design Docs (`docs/phases/`)

Architectural design documents for each development phase. These describe the *what* and *why*
of each phase's design before implementation.

See [`docs/phases/README.md`](phases/README.md) for the full phase index and status.

## Active Development (`dev/`)

Working artifacts from building Aperture. See [`dev/README.md`](../dev/README.md).

| Dir | What's There |
|-----|--------------|
| `dev/phase-4/` | Current phase: token economics, refactor, bug-dive |
| `dev/diagnostics/` | 10 deep-dive debugging rounds for plan layering failure |
| `dev/metacog-design/` | Phase 3 design: metacognition + dynamic context shifting |
| `dev/research/` | Provider research (Codex, OpenAI Responses API, modularity) |
| `dev/audits/` | Quality/security audits from earlier phases |

## Archive

- `docs/archive/` — historical product/reference docs (never source of truth)
- `docs/archive/APERTURE-brainstorm.md` — original 74K design brainstorm (legacy)

## Authority Order

`docs/` > `dev/` > `.context/` (local-only, gitignored)
