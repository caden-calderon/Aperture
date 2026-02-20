# Aperture — Project Overview

> Universal LLM context visualization, management, and control proxy.

## Motivation

AI coding tools like Claude Code and Codex are powerful but operate as black boxes.
Every request sends your entire conversation history to the provider API — and you have
zero visibility into:

- What's actually in the context window right now
- How full it is, or how fast it's filling
- What's eating the most tokens (old tool results? stale code reads?)
- When the tool will start silently dropping or compacting information

When context fills up, tools compact or drop information without telling you. You're
flying blind. The AI is too — it can't see what it previously knew, but it also can't
tell you what it's forgotten.

**Aperture is a solution to this.** It sits as a transparent local proxy between your
AI tool and the provider API, giving both you and the AI full visibility into — and
control over — the context window.

## How It Works

```
Your AI Tool ──── HTTP ────▶ Aperture Proxy ──── HTTP ────▶ Provider API
               ◀────────────                  ◀────────────
                     │               │
                     ▼               ▼
              Context Engine    MCP Server
              (parse + track)   (AI self-control)
                     │               │
                     └───────┬───────┘
                             ▼
                    Tauri Desktop App
                   (visualize + manage)
```

**On every request:**
1. Aperture intercepts the outgoing payload
2. Parses all messages into semantic blocks (system, user, assistant, tool calls, tool results)
3. Assigns zones: primacy (stable top), middle (historical), recency (active)
4. Tracks token usage, heat (access frequency), staleness, and file references
5. Forwards the request transparently to the provider

**On every response:**
1. Aperture intercepts the response stream
2. Checks if the AI used any context management tools
3. If so, processes those tool calls internally and re-invokes the AI
4. Forwards the final response to your tool

## The Workflows

### 1. Passive Observation (zero config)
Start Aperture, point your tool at the proxy, and watch. The desktop app shows every
block in your context window — what it is, how big it is, which zone it's in, and
how the budget is tracking. No AI involvement required.

### 2. AI Self-Management (MCP tools)
Aperture exposes 5 MCP tools the AI can call to inspect and manage its own context:

| Tool | Purpose |
|------|---------|
| `aperture_context_preview` | Get a summary of all blocks with token counts and zones |
| `aperture_context_read` | Read the full content of a specific block |
| `aperture_context_search` | Search across blocks by keyword |
| `aperture_context_plan` | Stage a context mutation plan (archive, compress, pin, shift) |
| `aperture_context_status` | Get budget status and alert level |

The AI can use these tools to understand what's in its context, propose changes, and
commit them — without breaking the conversation flow.

### 3. Staged Planning
The most powerful workflow. The AI:
1. Calls `aperture_context_preview` to see what's in context
2. Calls `aperture_context_plan` with `op: "stage"` to propose mutations
3. Reviews its own proposal (optional)
4. Calls `aperture_context_plan` with `op: "commit"` to apply

Once committed, **mutations are persistent**: archived blocks are stripped from every
subsequent request, not just the current one. The AI doesn't need to re-plan every turn.

### 4. User-Driven Management (Desktop UI)
The Tauri desktop app provides direct block manipulation: pin important blocks, manually
archive stale ones, adjust the budget ceiling, and watch the token budget bar in real-time.

## Architecture Decisions

**Why a proxy?** A proxy is the only way to observe and modify API traffic without
modifying the AI tool itself. Aperture works with any tool that talks to Anthropic or
OpenAI APIs — no plugins, no forks, no integration work.

**Why Rust?** The proxy is on the hot path for every AI request. Rust gives us
microsecond-level latency overhead, safe concurrency, and zero-copy stream passthrough.

**Why Tauri?** Native desktop performance with a web frontend. The context visualization
involves real-time canvas rendering (block heat maps, dithering effects) that benefits
from native GPU access.

**Why MCP?** Model Context Protocol is the emerging standard for AI tool integration.
Building Aperture as an MCP server means it works with any MCP-capable client, not just
Claude Code.

**Why cache-safe mutations?** Both Anthropic and OpenAI use prefix-based prompt caching.
Naive context removal (inserting/deleting blocks mid-conversation) would break the cache
and double API costs. Aperture applies all mutations at the trailing edge of the conversation,
preserving the cache prefix.

## Current State (Phase 4 — Token Economics)

**What's working:**
- Transparent proxy for Anthropic Messages API and OpenAI Responses/Chat APIs
- Block parsing, zone assignment, token counting, staleness tracking
- All 5 MCP tools: preview, read, search, plan, status
- Staged planning with persistent archival
- Plan layering: multiple commit rounds stack correctly
- Desktop UI: block list, token budget bar, session selector, settings panel
- Cache-safe mutation rewriting
- 696 tests (643 Rust + 53 frontend)

**Known limitations:**
- Budget display shows Aperture's estimate (message tokens only) — Claude Code's `/context`
  includes overhead (system prompt, tools, memory) which adds ~35k tokens
- Breadcrumb delta shows "Net: +0" on persistent re-archival (cosmetic only)
- Sessions using the proxy may occasionally crash Claude Code after file edits
  (edits persist — this is a proxy latency interaction with Claude Code's session management)

## Roadmap

The phase plan (see `docs/phases/`) outlines the full vision:

| Phase | Focus | Status |
|-------|-------|--------|
| 0 | UI prototype | Complete |
| 1 | Proxy core | Complete |
| 2 | Context engine | Complete |
| 3 | Metacognition + MCP tools | Complete |
| 4 | Token economics + stability | In Progress |
| 5 | Memory lifecycle, checkpoints, forking | Planned |
| 6 | Staging, presets, templates | Planned |
| 7 | NL search and commands | Planned |
| 8+ | Compression, multi-session, sharing | Planned |

The long-term vision: Aperture becomes the control plane for AI context — not just
observing it, but actively shaping it for cost, coherence, and continuity.
