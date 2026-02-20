# Aperture

> Universal LLM context visualization, management, and control proxy.

**Status:** Active development (Phase 4 refactor-first cleanup complete; targeted bug-dive in progress)

## What It Is

Aperture runs as a local proxy between AI coding tools (Claude Code, Codex, etc.) and provider APIs. It provides real-time context visibility, block-level manipulation, snapshot branching, and a path toward policy-driven context management.

## Tech Stack

- **App Shell:** Tauri v2
- **Frontend:** Svelte 5 + SvelteKit
- **Backend/Proxy:** Rust (axum)
- **Testing/Checks:** Vitest + ESLint + svelte-check + cargo clippy/test

## Development Commands

```bash
# Install dependencies
make install

# Start development server
make dev

# Build for production
make build

# Run full quality gate (lint + typecheck + tests)
make check

# Run tests only
make test
```

## Documentation

- `docs/DOCS_INDEX.md` — canonical docs navigation map (start here)
- `docs/HACKATHON_SUBMISSION.md` — concise submission/demo snapshot and known issues
- `docs/ARCHITECTURE.md` — system architecture
- `docs/INTEGRATION.md` — frontend/backend integration and IPC contracts
- `docs/SECURITY_BASELINE.md` — security constraints and hardening baseline
- `docs/REPO_STRUCTURE.md` — repo layout and ownership map
- `docs/DOC_LIFECYCLE.md` — documentation lifecycle, naming, and archive rules
- `dev/active/README.md` — active workstream index
- `.context/RESUME.md` — session resume entrypoint for active implementation context
- `.context/README.md` — working-memory notes classification

## License

MIT
