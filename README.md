# Aperture

> Universal LLM context visualization, management, and control proxy.

**Status:** Beta — Phase 4 (Token Economics). Core proxy, context engine, and MCP tools working. 696 tests passing.

## The Problem

AI coding tools like Claude Code and Codex send your entire conversation history with every API request — and you have zero visibility into what's in that context window, how full it is, or what's eating your tokens. When context fills up, the tool compacts or drops information silently. You're flying blind.

## What Aperture Does

Aperture sits as a transparent local proxy between your AI coding tool and the provider API. It intercepts every request, parses the conversation into semantic blocks, and gives you (and the AI) tools to see and manage what's in context.

**What's working today:**
- **Transparent proxy** — Zero-config interception for Anthropic and OpenAI APIs (Claude Code, Codex CLI)
- **Context engine** — Real-time block parsing, zone assignment (primacy/middle/recency), token counting, staleness tracking
- **5 MCP tools** — The AI can inspect and manage its own context: `preview`, `read`, `search`, `plan`, `status`
- **Staged planning** — The AI proposes context changes (archive, compress, recall, pin, shift), you review, then commit
- **Persistent archival** — Archived blocks are stripped from every subsequent request, freeing tokens durably
- **Desktop UI** — Tauri app with block visualization, token budget bar, session management, settings panel
- **Cache-safe design** — All mutations are applied without breaking Anthropic/OpenAI prompt caching

**What it enables:**
- See exactly what's in your context window and how full it is
- The AI can clean up its own context — archiving stale tool results, compressing old code reads
- Persistent archival means freed tokens stay freed across the entire conversation
- Full API transparency — every request/response captured and inspectable

## Quick Start

### Platform Support

| Platform | Status | Notes |
|----------|--------|-------|
| Linux | Tested | Primary development platform |
| macOS | Should work | Tauri supports macOS; not actively tested |
| Windows | Not supported | WebView2 backend untested; no Windows CI |

### Prerequisites

- **Node.js 20+**
- **Rust 1.75+** (via [rustup](https://rustup.rs/))
- **npm** (included with Node.js)

**Linux (Ubuntu/Debian):**
```bash
sudo apt install build-essential libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf
```

**macOS:**
```bash
xcode-select --install
```

### Build & Run

```bash
# Install dependencies
make install

# Start development server (launches Tauri app + proxy on port 5400)
make dev
```

> First build compiles the Rust backend from scratch — expect 3-5 minutes. Subsequent builds are incremental (~5s).

### Point Your AI Tool at the Proxy

**Claude Code:**
```bash
ANTHROPIC_BASE_URL=http://localhost:5400 claude
```

**Codex CLI:**
```bash
OPENAI_BASE_URL=http://localhost:5400 codex
```

The proxy forwards all requests transparently. Your AI tool works exactly as before, but now Aperture is watching and can manage context.

### Optional: Install the CLI Helper

A convenience wrapper (`aperture claude`, `aperture start`, `aperture status`) is available for bash, zsh, and fish:

```bash
./scripts/install.sh
```

After installing, restart your shell and use `aperture claude` instead of setting env vars manually.

### MCP Tools (Optional)

Aperture includes a standalone MCP server that lets the AI inspect and manage its own context. To use it with Claude Code, build first then copy the example config:

```bash
# After make install, the MCP binary is at src-tauri/target/debug/aperture-mcp
cp .mcp.json.example .mcp.json
# Then launch Claude Code — it will auto-discover the MCP server
ANTHROPIC_BASE_URL=http://localhost:5400 claude
```

### Run Quality Gates

```bash
# Full quality gate (lint + typecheck + tests)
make check

# Individual checks
cargo test --manifest-path src-tauri/Cargo.toml    # 643 Rust tests
npx vitest run                                       # 53 frontend tests
cargo clippy --manifest-path src-tauri/Cargo.toml   # Zero warnings
```

## Architecture

```
┌─────────────┐     ┌─────────────────────────────────────┐     ┌──────────┐
│  AI Coding  │────▶│              Aperture                │────▶│ Provider │
│    Tool     │◀────│                                      │◀────│   API    │
│ (Claude,    │     │  ┌───────┐  ┌────────┐  ┌────────┐  │     │(Anthropic│
│  Codex)     │     │  │ Proxy │─▶│ Engine │─▶│Planner │  │     │ OpenAI)  │
│             │     │  └───────┘  └────────┘  └────────┘  │     └──────────┘
│             │     │  ┌───────┐  ┌────────┐  ┌────────┐  │
│             │     │  │Parser │  │Rewriter│  │  MCP   │  │
│             │     │  └───────┘  └────────┘  └────────┘  │
└─────────────┘     └─────────────────────────────────────┘
                              │           ▲
                              ▼           │
                         ┌─────────────────┐
                         │   Tauri Desktop  │
                         │   App (Svelte 5) │
                         └─────────────────┘
```

- **Proxy** — axum HTTP proxy with SSE tee, transparent byte-passthrough
- **Parser** — Extracts semantic blocks from Anthropic Messages API and OpenAI Responses/Chat APIs
- **Engine** — Block store, session management, zone assignment, budget tracking
- **Planner** — Mutation planning, staged plans, heuristic suggestions, persistent archive intent
- **Rewriter** — JSON-level payload mutation, cleanup, trailing context injection
- **MCP Server** — Standalone binary exposing context tools via Model Context Protocol

See `docs/ARCHITECTURE.md` for the full design and `docs/REPO_STRUCTURE.md` for code layout.

## Documentation

| Doc | What It Covers |
|-----|----------------|
| [`docs/OVERVIEW.md`](docs/OVERVIEW.md) | Project motivation, how it works, workflows, roadmap |
| [`docs/HACKATHON_SUBMISSION.md`](docs/HACKATHON_SUBMISSION.md) | Submission snapshot — what works, known issues, demo guide |
| [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) | System architecture and design decisions |
| [`docs/REPO_STRUCTURE.md`](docs/REPO_STRUCTURE.md) | Code layout and module ownership |
| [`docs/INTEGRATION.md`](docs/INTEGRATION.md) | Frontend/backend IPC contracts |
| [`docs/DOCS_INDEX.md`](docs/DOCS_INDEX.md) | Full documentation navigation map |
| [`dev/`](dev/README.md) | Development working docs: phase plans, diagnostics, research |

## Tech Stack

- **App Shell:** Tauri v2
- **Frontend:** Svelte 5 + Tailwind CSS
- **Backend/Proxy:** Rust (axum + tokio + reqwest + serde_json)
- **Context Protocol:** MCP (Model Context Protocol) via standalone binary
- **Testing:** cargo test (643) + Vitest (53) + cargo clippy + svelte-check + ESLint

## License

[MIT](LICENSE)
