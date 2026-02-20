# Repository Structure

Last updated: 2026-02-19

## Top-Level Layout
- `src-tauri/`: Rust backend (proxy, engine, metacog runtime, MCP runtime, terminal bridge, tests).
- `src/`: Svelte/Tauri frontend.
- `docs/`: stable project documentation.
- `docs/archive/`: historical/superseded durable docs.
- `dev/active/`: initiative/phase execution docs (plans, tasks, context logs).
- `.context/`: session working-memory artifacts and carry-over notes.
- `.context/archive/`: historical working-memory notes/prompts.
- `tests/`: shell/manual/integration support outside Rust crate tests.

## Backend Layout (`src-tauri/src/`)
- `proxy/`: HTTP proxy runtime.
  - `handler/`: request flow helpers (routing/headers/exchange finalization).
  - `interceptor/`: context-tool interception and response-shape helpers.
  - `capture/`: capture store + SSE reconstruction helpers.
  - `parser/`: provider wire parsing and stable block identity derivation.
  - `rewriter/`: JSON mutation application and sanitation/injection helpers.
  - `context_api.rs`: internal `/_aperture/*` API for context tools.
- `engine/`: authoritative session/block state and planner pipeline.
- `metacog/`: runtime/tool dispatch and provider-specific tool wiring.
- `mcp/`: standalone MCP JSON-RPC server runtime used by `aperture_mcp` bin.
- `terminal/`: PTY and Codex bridge integration.
- `events/`: event types + dispatcher.

## Docs Layout and Ownership
- `docs/`: canonical reference docs; keep accurate and concise.
- `dev/active/`: mutable execution state for active initiatives.
- `.context/`: volatile context memory; not canonical architecture/API source.
- `docs/DOC_LIFECYCLE.md`: authoritative doc lifecycle and archival policy.

## Current Cleanliness Notes
- Proxy and MCP orchestration hotspots have been split by concern.
- Remaining larger backend hotspots for future cleanup are mostly planner/engine/terminal internals.
- Doc sprawl is managed via `docs/DOCS_INDEX.md`, `dev/active/README.md`, and `.context/README.md`.
