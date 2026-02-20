# Hackathon Submission Snapshot

Last updated: 2026-02-20

## Project Summary

Aperture is a local proxy and control plane for AI coding tools (Claude Code, Codex CLI, etc.). It sits transparently between the tool and the provider API, parsing every request into semantic blocks and giving both the user and the AI real-time visibility and control over what's in the context window.

The core insight: AI coding tools send your entire conversation history with every request, but you have zero visibility into that context — what's taking up space, what's stale, what could be archived. Aperture makes this visible and actionable.

## What's Working (Verified in Manual Testing)

### Proxy Layer
- Transparent HTTP proxy for Anthropic Messages API and OpenAI Responses/Chat APIs
- SSE streaming passthrough with tee for capture
- Zero-config: point your tool's base URL at `localhost:5400`
- Hop-by-hop header stripping, zstd decompression, proper byte-passthrough

### Context Engine
- Real-time block parsing from all 3 API wire formats
- Content-fingerprint-based block IDs (stable across positions)
- Zone assignment: Primacy (system), Middle (old turns), Recency (recent turns)
- Token counting, staleness tracking, usage heat
- Session management with model-aware flip guard

### MCP Tools (All 5 Verified Working)
The AI can inspect and manage its own context via Model Context Protocol:

| Tool | Purpose | Status |
|------|---------|--------|
| `aperture_context_preview` | Zone-grouped block inventory with archival suggestions | Verified |
| `aperture_context_status` | Full manifest with per-block token counts | Verified |
| `aperture_context_search` | Relevance-ranked search across blocks with snippets | Verified |
| `aperture_context_read` | Full content of a single block (with size guardrails) | Verified |
| `aperture_context_plan` | Stage, append, preview, commit, or discard context mutations | Verified |

### Plan Operations (All 6 Verified Working)

| Operation | What It Does |
|-----------|-------------|
| `archive` | Remove blocks from active context (persistent — re-applied every turn) |
| `compress` | Replace block content with AI-authored summary |
| `expand` | Restore full content from compressed block |
| `recall` | Bring archived block back into active context |
| `pin` | Protect block from archival |
| `shift_to` | Move block between zones (primacy/middle/recency) |

### Desktop UI (Tauri + Svelte 5)
- Block visualization with zone coloring
- Token budget bar with soft/medium/hard threshold markers and ceiling slider
- Session dropdown with provider usage readout
- Settings panel for budget ceiling configuration
- Real-time updates via Tauri IPC events

### Verified Behaviors (Manual Test Round 10)
- **Persistent archival stacking**: 3 successive archive rounds accumulated correctly (8+8+5 = 21 blocks stripped per turn)
- **Cache-safe mutations**: All mutations applied to last user message position, preserving prompt cache prefix
- **Fail-open design**: Proxy errors, tool failures, and guardrail triggers never break the AI tool
- **Guardrails**: Rate limiting, circuit breaker, output size caps, kill switch

## Test Suite

```
643 Rust tests — engine, planner, parser, rewriter, proxy, interceptor, MCP, integration
 53 Frontend tests — stores, budget, policy, adapters, threading
  0 Clippy warnings
```

## Known Limitations (Honest Status)

This is beta software with active development. Known issues:

1. **Breadcrumb delta display**: Shows "Net: +0" for persistent re-archival (budget % is correct, only the delta display is wrong)
2. **Budget % gap vs Claude Code's `/context`**: Aperture tracks message payload tokens only; Claude Code includes system prompt + tool schema overhead (~17.5% gap)
3. **Tool overhead cost**: Each plan cycle (preview + stage + commit) adds ~2-3k tokens in tool use/result blocks. Archival targets should be >3k tokens to be ROI-positive.
4. **File edit crash through proxy**: Claude Code occasionally crashes after editing files while proxied through Aperture. Edits persist but the session dies. Under investigation.
5. **Cache-aware archival**: Archival can cause one-time cache misses when removing blocks mid-prefix. Mitigated by batch-gating, but a fully cache-aware strategy is planned.

## Architecture

See [`ARCHITECTURE.md`](ARCHITECTURE.md) for the full design. Key modules:

| Module | Responsibility |
|--------|---------------|
| `proxy/parser/` | Wire format parsing into canonical Block records |
| `proxy/rewriter/` | JSON-level payload mutation, cleanup, sanitization |
| `proxy/handler/` | Upstream routing, transport, SSE tee |
| `engine/` | Block store, sessions, zones, budget tracking |
| `engine/planner/` | Mutation planning, heuristics, persistent archive intent |
| `metacog/` | Runtime detection, tool dispatch, MCP integration |
| `mcp/` | Standalone MCP server binary (stdio JSON-RPC) |

## Demo Guide

**Best demo flow:**
1. Launch Aperture (`make dev`) and Claude Code through the proxy
2. Have a multi-turn conversation (code generation, file reads, tool use)
3. Show the UI — blocks accumulating, zones, token budget filling
4. Have the AI call `aperture_context_preview` — show it seeing its own context
5. Stage an archival plan (`archive` stale tool results) → commit → show tokens freed
6. Continue coding — show the archived blocks stay gone across turns
7. Show `compress`, `recall`, `pin` operations
8. Show the settings panel — drag the budget ceiling slider

**Demo prompts**: `dev/phase-4/manual-test-prompts.md`

## Build & Run

```bash
# Prerequisites: Node.js 20+, Rust 1.75+
# Linux: sudo apt install build-essential libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf

make install    # Install all dependencies
make dev        # Launch Tauri app + proxy (first build: ~3-5 min)
make check      # Full quality gate
```

## Development Methodology

This project was built with systematic engineering practices:
- **Phase discipline**: UI-first (Phase 0) → Proxy (Phase 1) → Engine (Phase 2) → Metacognition (Phase 3) → Token Economics (Phase 4)
- **10 rounds of diagnostic investigation** with JSONL log analysis, hypothesis tracking, and fix verification (see `dev/diagnostics/`)
- **3 refactor tranches** splitting oversized modules into focused boundaries before fixing bugs
- **Test-driven fixes**: Every bug fix includes regression tests

## Where To Read Next

1. [`docs/ARCHITECTURE.md`](ARCHITECTURE.md) — System design
2. [`docs/REPO_STRUCTURE.md`](REPO_STRUCTURE.md) — Code layout
3. [`docs/INTEGRATION.md`](INTEGRATION.md) — Frontend/backend contracts
4. [`dev/`](../dev/README.md) — Diagnostic investigations, design history, and research
