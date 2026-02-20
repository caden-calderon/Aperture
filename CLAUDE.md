# Aperture

Universal LLM context visualization, management, and control proxy.

## Stack

- **Backend:** Rust (Tauri v2 + axum proxy + context engine)
- **Frontend:** Svelte 5 + Tailwind + Canvas API
- **Aesthetic:** Halftone/dithering — Obra Dinn meets IDE. Density = meaning.

## Key Files

| File | Purpose |
|------|---------|
| `.claude/skills/aperture-ui.md` | UI design system (colors, typography, animations, dithering spec) |
| `.context/RESUME.md` | **Session entry point** — current state, what to read, next steps (local-only, gitignored) |
| `docs/phases/phase-N.md` | Phase-specific design and implementation details |
| `dev/phase-4/` | Current active phase working docs |

> **Note:** `.context/` is gitignored — it lives only on your local machine as session working memory.
> `docs/` and `dev/` are in git. See `docs/DOCS_INDEX.md` for the full navigation map.

## Build Strategy

**UI-first.** Build the complete visual experience with mock data before wiring backend.
Phase 0 = full Svelte 5 UI with all interactions, animations, and dithering effects.

## Conventions

### Rust
- `thiserror` for domain errors, `anyhow` for application errors
- Async everywhere in proxy (tokio)
- Proxy critical path: zero blocking, microsecond latency budget
- `cargo clippy` and `cargo fmt` before committing

### Svelte 5
- Runes: `$state`, `$derived`, `$effect` — no legacy `$:` reactive statements
- Components in `src/lib/components/`
- Stores in `src/lib/stores/`
- Canvas for visual effects (halftone, heat, dissolution)
- DOM for blocks (accessibility, text selection)
- Tailwind for layout only, custom CSS for effects

### Design
- Reference `.claude/skills/aperture-ui.md` for ALL visual decisions
- Every pixel encodes data — no decorative elements
- JetBrains Mono for code/data, IBM Plex Mono for headers
- Test in dark mode (default) and consider light/newspaper mode

## Dev Commands

```bash
# Development
make dev              # pnpm tauri dev
aperture claude       # Launch Claude Code through proxy

# Quality (run before phase completion)
make check            # lint + typecheck + test

# Individual checks
cargo clippy --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
npm run check         # svelte-check
```

## Session Workflow

1. Read `.context/RESUME.md` first (local-only, not in git)
2. Read `dev/phase-4/context.md` for current state
3. Continue from checkpoint
4. Update `.context/RESUME.md` before compaction (~70% context)
