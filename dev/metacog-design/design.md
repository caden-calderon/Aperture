# Metacognition + Dynamic Context Shifting — Design Document

> **Status**: Phase 3 implementation complete (Checkpoints A-G). Staff review complete; remediation in progress.
> **Priority**: CRITICAL — Aperture's two most differentiated features
> **Created**: 2026-02-11
> **Context**: Brainstorm session capturing vision, architecture decisions, and implementation philosophy
> **Review Artifacts**:
> - `dev/active/metacog-dynamic-shifting/staff-review-2026-02-13.md`
> - `dev/active/metacog-dynamic-shifting/plan.md`
> - `dev/active/metacog-dynamic-shifting/tasks.md`

---

## Vision

Every AI tool today treats context as a dumb append-only text buffer. Context fills linearly, hits a wall, panic-compresses everything, loses information, the model "forgets," the user re-explains, repeat. This is the status quo.

Aperture changes this fundamentally with two interlocking features:

**Metacognition**: The model has explicit, structured awareness of its own context window. It can see what it has, what's compressed, what's archived, and can actively explore and manage its own memory through real tools.

**Dynamic Context Shifting**: The system autonomously manages the context window as a living pool. Blocks continuously flow between states — full detail when relevant, compressed when backgrounded, archived when dormant, recalled when needed. The context window is never "full" because it's always being optimized. The model never "forgets" because relevant information surfaces before it's needed.

Together, these create a **collaborative context management system** where both the system (heuristics) and the model (intelligence) work to keep context optimal. Neither alone is as good as both together.

### Why This Matters Now

1. The codebase is lean — getting these in now avoids retrofitting into a more complex system later
2. These are the "this changes how I work with AI" features — everything else (heat maps, presets, analytics) is incremental
3. The proxy-sees-everything architecture makes this uniquely possible — since every major LLM tool sends the full conversation every request, the proxy can rewrite reality on every turn

### The Killer Demo

Two agents side by side on a long-running task. One running through Aperture, one vanilla CLI. Watch the vanilla agent hit context walls, lose information, hallucinate from stale context. Watch the Aperture agent smoothly cycle context, maintain coherence, and complete the task with better output. That sells itself.

---

## Core Philosophy

### Automation Over Manual Control

The primary experience should be: "Aperture automatically manages your context window. You can intervene if you want, but you probably won't need to." Manual controls (compression sliders, per-block actions) exist as escape hatches, not as the primary workflow.

When context gets huge, nobody wants to sift through blocks manually compressing things. The most a user should need to do is select a bunch of blocks and hit compress. Both manual and automatic should exist, but automation is prioritized.

### The Model as Collaborator

The model isn't just a consumer of context — it's a collaborator in managing it. The system handles the mechanical (budget pressure, staleness decay), the model handles the intelligent ("I'm about to refactor auth, I need that file read back"). Model intent overrides system heuristics (except hard budget limits).

### Continuous Over Catastrophic

Instead of "oh we have 20% context left, compress everything," the system does continuous micro-management:
- Micro-compressions of stale blocks
- Progressive archival of irrelevant blocks
- Preemptive recall of relevant blocks
- Position shifts to exploit primacy/recency attention

This entirely skips the "pause and do a full compress" paradigm that loses valuable info.

### Exploiting LLM Attention Patterns

LLMs have known attention biases:
- **Primacy**: Strong attention to early context (system prompt, first messages)
- **Recency**: Strong attention to recent context (last few turns)
- **Middle**: Weakest attention — things get "lost in the middle"

The system actively positions blocks to exploit this:
```
[System prompt + manifest]           ← primacy (model pays most attention)
[Pinned / high-relevance blocks]     ← primacy
[Compressed middle-zone blocks]      ← middle (model attention weakest here)
[Current task context]               ← recency (model pays strong attention)
[Latest user message]                ← recency
```

Important context lives where the model pays attention. Low-priority context sits in the middle, compressed. The system reorders, compresses, archives, injects — anything to make the automatic autonomous system better.

---

## Architecture

### Two Agents, One Context

```
┌─────────────────────────────────────────────┐
│              APERTURE ENGINE                 │
│                                             │
│  System Heuristics         Model Intent     │
│  (always running)          (per-turn)       │
│  ┌──────────────┐     ┌──────────────────┐  │
│  │ Budget pressure│    │ context_plan()   │  │
│  │ Staleness decay│    │ context_search() │  │
│  │ Task detection │    │ context_read()   │  │
│  │ Dep overlap   │    │ Self-directed    │  │
│  │ File mutation │    │ + triggered      │  │
│  └──────┬───────┘     └──────┬───────────┘  │
│         │                    │              │
│         └────────┬───────────┘              │
│                  ▼                           │
│          Context Planner                     │
│     (resolves conflicts, applies)            │
│                  │                           │
│                  ▼                           │
│    ┌─────────────────────────┐               │
│    │  Updated Context State  │──► Manifest   │
│    └─────────────────────────┘    (next turn) │
└─────────────────────────────────────────────┘
```

**Conflict resolution**: Model intent wins over system heuristics. If the system wants to archive block X but the model just pinned it, the pin holds. Only exception: hard budget limits — if at capacity ceiling, even pinned blocks may need compression (but never archival without model consent).

### Shared Core + Client Adapters

**Critical architecture**: One metacognition system with multiple front doors. The core planner, manifest generation, heuristics, block store, and mutation engine are shared. Thin client adapters handle transport differences.

```
┌──────────────────────────────────────────────────┐
│                  SHARED CORE                      │
│  Planner · Manifest · Cleanup · Heuristics        │
│  Block Store · Mutation Engine · File Tracker      │
└──────────────────┬───────────────────────────────┘
                   │
        ┌──────────┼──────────┐
        ▼          ▼          ▼
  ┌───────────┐ ┌──────────┐ ┌──────────────┐
  │  Claude   │ │  Codex   │ │   Passive    │
  │  Runtime  │ │  Runtime │ │   Runtime    │
  │  (MCP)    │ │  (Proxy) │ │ (manifest    │
  │           │ │          │ │  only)       │
  └───────────┘ └──────────┘ └──────────────┘
```

**`ContextToolRuntime` trait** (Rust):
```rust
trait ContextToolRuntime {
    /// Make context tools available to the model
    fn register_tools(&self) -> Vec<ContextToolDef>;

    /// Extract context tool calls from model response
    fn extract_context_calls(&self, response: &ResponseData) -> Vec<ContextToolCall>;

    /// Inject tool results into conversation history
    fn inject_results(&self, results: &[ContextToolResult], messages: &mut Messages);

    /// Strip context tool calls from history (cleanup)
    fn cleanup_history(&self, messages: &mut Messages) -> CleanupResult;

    /// Inject manifest into payload
    fn inject_manifest(&self, manifest: &Manifest, messages: &mut Messages);
}
```

### Client Runtimes

**ClaudeMcpRuntime** (MCP native):
- Tools exposed via Aperture's MCP server
- Claude Code discovers and calls them natively
- Tool results handled by Claude Code's MCP plumbing
- Cleanup strips MCP tool_use/tool_result from proxy-visible history
- Best experience: full interactive exploration with real-time feedback

**CodexProxyRuntime** (proxy-injected tools):
- Tools injected into the API request's `tools[]`/`functions[]` array by the proxy
- Model calls them as regular function calls
- Proxy intercepts context tool calls in the response, handles internally
- Injects results alongside real tool results on next request
- If only context tools called: proxy re-invokes API with results (inner loop)
- Slightly more latency than MCP, but same capabilities

**PassiveRuntime** (manifest only, no tools):
- For clients that support neither MCP nor proxy tool injection
- Model receives manifest (awareness) but cannot call context management tools
- All context management handled by autonomous heuristics
- Still useful: model sees budget pressure, staleness warnings, can mention needs in natural language
- System does its best with heuristics alone

### Client Compatibility Matrix

| Client | Runtime | Tools Available | Exploration | Notes |
|--------|---------|----------------|-------------|-------|
| Claude Code | MCP native | Full | Interactive | Primary path |
| Codex CLI (API key) | Proxy-injected | Full | Interactive (with re-invoke) | OpenAI function calling |
| Codex CLI (ChatGPT sub) | Proxy-injected | Full | Interactive (with re-invoke) | Same, different auth route |
| Aider / Continue.dev | Proxy-injected or Passive | Varies | Depends on client | Need testing |
| Unknown clients | Passive | None | None | Manifest + heuristics only |

### Why This Layering Matters

MCP is a **client** capability, not a provider capability. Claude Code supports MCP; Codex CLI does not. But both go through the proxy and both use APIs that support tool/function calling. The adapter layer means:
- The shared core doesn't care which client is connected
- Adding a new client is writing one adapter, not touching the planner
- Degradation is graceful: MCP → proxy-injected → passive → heuristics-only

**The proxy's role across all runtimes**:
1. Apply planned context mutations between turns (reorder payload, inject compressed blocks)
2. Inject manifest/status into primacy
3. Clean up ephemeral tool calls from conversation history
4. Run autonomous heuristics (budget pressure, staleness, task detection)
5. Track file mutations across tool calls
6. Forward traffic (its normal job)
7. For proxy-injected runtimes: additionally inject tool definitions and handle tool call interception

### Plan → Preview → Apply Pattern

The model doesn't modify context in real-time. It **plans** changes and **previews** the result. Actual mutations are deferred to between turns.

This is the key architectural insight that simplifies everything:
- Tools are read-only exploration + planning (no side effects during the turn)
- The proxy applies mutations between turns (the only thing modifying the actual payload)
- The model sees a preview of what context will look like after changes
- The model can confirm or adjust before anything happens

---

## The Model's Experience

### Context Management Workflow (Example)

```
"Okay, task complete. Next task is refactoring auth. Let me check my context."

→ calls context_preview()
← Returns: block inventory with smart-extracted previews for every block

"Block #8 preview shows AuthMiddleware, validate_token, refresh_session —
 that's exactly what I need. Let me see the full thing."

→ calls context_read(8)
← Returns: full content of block #8

"Definitely need this for the refactor. #12 is test output — don't need it.
 #15 is frontend CSS — irrelevant now but I'll need it in a few tasks.
 Let me plan my changes and write a compression for #15."

→ calls context_plan({
    expand: [8],
    shift_to: { 8: "primacy" },
    compress: {
      15: "CSS styles for context blocks (.context-block) and zone headers
          (.zone-header). Uses Tailwind @apply for layout. Theme variables
          for colors. ~80 lines."
    },
    archive: [12]
  })
← Returns: PREVIEW of resulting context state
   "After changes: 48% budget, 25 blocks active, #8 in primacy (847 tok),
    #15 model-compressed (412→~25 tok), #12 archived. Net savings: 2,800 tok."

"Looks good. Ready for the next task."

TURN ENDS. Cleanup strips exploration. Breadcrumb left. Context shifts applied.
Model wakes up next turn with auth.rs in primacy, CSS compressed, tests archived.
```

### The Search Capability

The model can grep its own brain:

```
→ calls context_search("auth handler")
← Returns:
   Matches:
     #8  [active, middle, compressed] auth.rs file read (turn 3) — 847 tok
     #21 [archived] handler.rs refactor discussion (turn 12) — 1,203 tok
     #34 [active, recency] auth middleware error (turn 28) — 156 tok
   Use context_read(id) for full content.
```

Search spans active blocks AND archives. The model can find anything from its entire history, drill down with `context_read()` for full content, then plan changes to bring it into active context.

### Two-Tier Block Viewing

Blocks have two viewing levels:

1. **Preview**: Smart extraction of key elements — function names, file paths, code snippets, important text. Not a summary, more like a table of contents. Rule-based extraction that pulls the most identifying/useful fragments:
   ```
   #8 auth.rs read (turn 3, 847 tok, middle)
   Preview: fn AuthMiddleware::validate_token() ... fn refresh_session() ...
            JWT validation, session management ... imports: jsonwebtoken, axum
   ```
   Enough to decide "do I need this?" without seeing the full content.

2. **Full** (via `context_read(id)`): Complete original content of the block.

The model drills down: preview → "looks relevant, let me see it" → full read → plan to keep/shift/archive.

### Compression Strategy (Three Sources)

Compression is distinct from previews. Three compression sources:

1. **Model-authored compression**: The model writes its own summaries during context management. It knows what matters because it's been working with the content. It knows what details the next task needs. The compression gets included in `context_plan()` and applied on turn end. More dirty-period content, but higher quality results — who better to summarize something than the thing that was using it.

   ```
   → calls context_plan({
       compress: {
         8: "auth.rs contains AuthMiddleware with validate_token() for JWT
             validation and refresh_session() for session renewal. Uses
             jsonwebtoken crate. Key: tokens expire after 1hr, refresh
             window is 5min before expiry."
       }
     })
   ```

2. **Sidekick LLM compression**: For bulk/automated compression. When the system autonomously compresses 15 stale blocks, it sends them to a sidekick model (Haiku, Codex mini, OpenRouter). No need to bother the primary model with writing each compression.

3. **Archival** (remove from payload entirely): For blocks that are truly irrelevant. Kept in storage, recallable, but zero token cost in active context.

No rule-based truncation or "trimmed" level for actual compression. Either a real model (primary or sidekick) writes the summary, or the block gets archived entirely.

---

## Ephemeral Tool Calls + Cleanup

### The Core Idea

When the model explores its context (searching, reading, inspecting), that exploration process is **ephemeral**. It exists during the turn for the model to reason with, but gets stripped from history between turns. Only the results persist.

**Analogy**: When you look something up on your phone, you don't keep a log of "opened Google, typed query, clicked result" in your working memory. You just have the information now.

### The Cleanup Flow

```
TURN N:
  Model explores context (5-10 tool calls: preview, search, read, plan)
  Model reasons about what it found
  Model produces substantive response + context_plan with changes

BETWEEN TURNS (cleanup crew):
  1. Read planned changes from context_plan
  2. Apply mutations: expand, compress, archive, recall, reorder
  3. Strip ALL context_* tool_use entries from history
  4. Strip ALL context_* tool_result entries from history
  5. Replace with breadcrumb: "Context updated: expanded #8 auth.rs → primacy,
     compressed #15, archived #12. Budget: 52%."
  6. Update manifest

TURN N+1:
  Model sees:
  - Clean history (no context tool calls, just breadcrumb)
  - Blocks in their new positions (expanded, compressed, shifted)
  - Updated manifest reflecting current state
  - Zero memory of the exploration process
  - Starts next task immediately
```

### The "Dirty Period"

During the turn, the model's context includes its exploration tool calls and results. This is the "dirty period" — context temporarily inflated by management overhead. This is fine because:
- The model NEEDS this info during the current turn to make decisions
- It's cleaned up before the next turn
- Only the final plan + breadcrumb survive

Things the model explored but decided it didn't need (expanded a block, checked it, rejected it) are completely erased. Zero residue.

### Breadcrumb Format

```
[Context update: searched "auth handler" → expanded #8 (auth.rs, 847 tok → primacy),
 compressed #15 (CSS, 412→89 tok), archived #12 (test output). Net: -1,960 tok. Budget: 52%]
```

One line. The model knows what happened and why without remembering the process. On future turns, it trusts this breadcrumb like reading its own notes.

---

## Layered Awareness (Manifest Design)

### The Problem with Full Manifests

A full context manifest (every block, zone, staleness, tokens) costs 200-300 tokens per turn. Over a 50-turn conversation, that's 10,000-15,000 tokens of pure overhead. Most turns, the model doesn't need the full inventory.

### Layered Solution

**Always present** (~30 tokens, in primacy or recency):
```
[Aperture: 62% budget | 28 active, 5 compressed, 3 archived | 2 pending actions]
```

Status line. Model always knows the big picture. Cheap.

**On change** (~50-100 tokens, only when something shifted):
```
[Context Δ: compressed #5,#12 (-1,400 tok). Recalled #2 auth.rs (+847 tok). Budget now 58%.]
```

Only appears when the system or model's previous commands caused changes. Most turns, absent.

**On demand** (full inventory, model requests via `context_status()`):
Complete manifest with every block, zone, staleness, previews. Expensive but rare — model only asks when making big context management decisions.

**On budget pressure** (system-injected at thresholds):
```
[Aperture Warning: 85% budget. Recommend archiving stale blocks: #5 (stale:0.95),
 #8 (stale:0.88), #12 (stale:0.91). Context tools available.]
```

Gives the model a chance to manage before the system acts autonomously.

### Where to Place It

Open question — needs experimentation. Options:
- **Primacy** (first system message): Always grounding the model's awareness
- **Recency** (just before latest user message): Freshest signal
- **Both**: Brief in primacy, detailed in recency
- **Dynamic**: Primacy when small, recency when detailed

Start with primacy for the status line. Experiment from there.

---

## Dynamic Code Context Updates

### The Problem

```
Turn 5:  Model reads auth.rs → block stored with file contents
Turn 10: Model edits auth.rs lines 20-30
Turn 15: Model's context still has Turn 5's READ with OLD code
Turn 25: Model recalls auth.rs block → outdated version → wrong assumptions
```

This happens in every LLM tool today. Old file reads sit in conversation history even after the model itself changed the file.

### The Solution

The proxy sees all traffic:
- Sees `read_file("auth.rs")` → captures block with contents
- Sees `edit_file("auth.rs", ...)` → knows auth.rs changed, can see diff/new content in tool result
- On next payload construction: engine updates block #5's content to reflect edits

The model never sees stale file content. Same for archived blocks — if a file gets edited after its read was archived, the archived version updates too. When recalled, the model gets the current version.

No filesystem access needed — just tracking tool call results through the proxy.

---

## Context Tool Surface (Shared Across Runtimes)

### Tools Exposed

```
aperture_context_preview()              → Block inventory with smart-extracted previews
aperture_context_read(block_id)         → Full content of a block
aperture_context_search(query, scope?)  → Search active + optionally archived blocks
aperture_context_plan(actions)          → Plan changes (including model-authored compressions), return preview
aperture_context_status()               → Full detailed manifest on demand
```

**Note**: No separate `inspect` tool. `preview()` shows smart extractions for all blocks. `read()` shows full content. Two tiers, not three.

### What They Return

**context_preview()**:
```
Budget: 62% (124,000 / 200,000 tokens)
Active blocks (28):
  PRIMACY:
    #1  system-prompt (1,800 tok, pinned)
    #3  project-config (450 tok, pinned)
  RECENCY:
    #45 user-msg-current (89 tok)
    #44 assistant-response (1,450 tok)
    #43 tool-result: auth.rs edit (312 tok)
  MIDDLE:
    #8  auth.rs read (compressed, 45 tok, original: 847)
        Preview: fn AuthMiddleware::validate_token() ... fn refresh_session() ...
    #12 test-output (stale: 0.9, 1,203 tok)
        Preview: cargo test ... 14 passed, 0 failed ... auth_test, session_test ...
    #15 styles.css read (412 tok)
        Preview: .context-block { ... } .zone-header { ... } tailwind @apply ...
    ... (19 more)
Compressed: 5 blocks (saved 3,200 tok)
Archived: 3 blocks (recallable via search)
```

**context_plan(actions)** — supports model-authored compressions, splits, shifts:
```
→ context_plan({
    expand: [8],
    shift_to: { 8: "primacy" },
    compress: {
      15: "CSS styles for context blocks and zone headers. Uses Tailwind."
    },
    archive: [12],
    split: { thread_23: { at: 5, archive_before: true } }
  })

← Planned changes:
  ✓ Expand #8 auth.rs (45 → 847 tok)
  ✓ Shift #8 to primacy
  ✓ Compress #15 styles.css (412 → ~20 tok) [model-authored]
  ✓ Archive #12 test-output (-1,203 tok)
  ✓ Split thread #23 at position 5 (archive first half)

Preview after apply:
  Budget: 48% (96,000 / 200,000 tok)  [was 62%]
  Active: 25 blocks  [was 28]
  Primacy: system-prompt, project-config, auth.rs
  Net savings: 2,800 tokens
```

**Multiple `context_plan()` calls**: Last plan wins. Each call replaces the previous plan entirely. Model can iterate on its plan before the turn ends.

---

## Trigger Mechanisms

### When Does Context Management Happen?

Combo approach — multiple trigger sources:

**Self-directed**: Model decides on its own to manage context. Works naturally when the model is between tasks, planning ahead, or notices something off.

**Task completion**: When the system detects a task boundary (significant shift in file references, tool patterns, or explicit task markers), it can nudge: "Task appears complete. Context tools available for cleanup."

**Budget warnings**: At configurable thresholds, the system injects warnings into the manifest. The model can respond proactively, or the system acts autonomously after N turns of inaction.

**Configurable budget ceiling**: User sets their comfort threshold (e.g., "never exceed 70%"). Three internal thresholds derived from it:
- **Soft** (~50% of ceiling): Start archiving stalest blocks
- **Medium** (~80% of ceiling): Archive middle-zone aggressively
- **Hard** (ceiling): Aggressive archival, only primacy + current task remain full

The engine owns the policy. The user owns the ceiling.

### Autonomous Heuristics (System-Driven)

Always running, independent of model actions:
- **Budget pressure**: Progressive compression as capacity grows
- **Staleness decay**: Blocks not referenced in N turns get progressively compressed
- **Task detection**: File reference shifts signal task boundaries, trigger relevance re-scoring
- **Dependency overlap**: If current task touches file X and block #14 is a read of file X, boost #14's relevance
- **File mutation tracking**: Edit operations update corresponding read blocks

### Model-Driven (Via Tools)

The model uses MCP tools when it chooses to. Trigger signals help but don't force. The model is the expert on what it needs; the system is the janitor keeping things tidy.

---

## Structural Constraints for Reordering

### Thread Lines as Atomic Units

The existing thread-line logic already knows "these blocks are a structural group" — tool call → tool result, prompt → response chains, multi-step tool sequences.

During reordering, thread groups are the default **atomic units**. The thread grouping utility from Phase 2 (turn-continuity + role-transition checks) becomes the reordering constraint solver.

### Thread Splitting

Large threads (e.g., 15 messages: user → assistant → 5 tool calls → assistant) can be expensive to move as a unit. Thread splitting is handled by:

- **Hard rules**: Tool call + tool result pairs MUST stay adjacent. Never split those.
- **Model-directed splitting**: The model can include split instructions in `context_plan()`. It has the intelligence to know "the first half of this thread was exploration, the second half was the actual fix — split here, archive the exploration, keep the fix."
- **System heuristics**: For autonomous compression, default to treating threads as atomic. Only the model can authorize splitting.

### Position Constraints

- Tool call + tool result pairs MUST stay adjacent
- System messages stay in primacy
- Latest user message always in recency (last position)
- Pinned blocks stay in their assigned zone
- Model-directed splits respected as specified

---

## Design Decision Log

### Why Standalone MCP Tools Over Proxy Intercept

**Evaluated**:
1. Inline @commands in response text (regex parsing, strip from output)
2. Structured `<context>` XML blocks in response text
3. Proxy-injected tool definitions with proxy re-invocation
4. **Selected: Standalone MCP tools**

**Reasoning**:
- @commands require teaching the model special syntax and regex parsing is fragile
- `<context>` blocks are better but still text-based, fire-and-forget with N+1 turn delay
- Proxy re-invocation works but adds complexity, latency, and cost
- MCP tools are native to how models work, give real-time feedback, and the client handles execution normally

### Why Plan-Preview-Apply Over Immediate Mutation

**Evaluated**:
1. Commands execute immediately, context changes mid-turn
2. **Selected: Plan changes, preview result, apply between turns**

**Reasoning**:
- Immediate mutation is complex (proxy inner loops, state consistency during turn)
- Preview lets the model confirm "yes, this is what I want" before committing
- Deferred application is simpler — proxy reads the plan, applies between turns
- The dirty period (exploration) cleans up naturally

### Why Ephemeral Tool Calls Over Persistent History

**Evaluated**:
1. Keep all context tool calls in history (model remembers exploration)
2. **Selected: Strip exploration, leave breadcrumb**

**Reasoning**:
- 5-10 tool calls per management session = 1,000-2,000 tokens of overhead per session
- Over a long conversation, this adds up fast
- The model doesn't need to remember HOW it managed context, just WHAT changed
- Breadcrumb gives sufficient context without the overhead
- "When you remember something, you don't remember the act of remembering"

### Why Model-Authored Compression Over Rule-Based Truncation

**Evaluated**:
1. Rule-based truncation (cut to N tokens, add `...`)
2. Rule-based trimming (strip whitespace/boilerplate)
3. **Selected: Model writes its own compressions + sidekick LLM for bulk**

**Reasoning**:
- Rule-based truncation loses important content unpredictably
- Rule-based trimming saves minimal tokens (5-10%) on code-heavy context
- The model knows what matters because it's been working with the content
- The model knows what the next task needs — it can tailor compression to preserve relevant details
- For bulk/automated compression, sidekick LLM (Haiku, mini, etc.) handles at lower cost
- Model-authored compression happens naturally during context management — zero extra infrastructure

### Why Two-Tier Viewing Over Three

**Evaluated**:
1. Three tiers: one-line preview → compressed summary → full content
2. **Selected: Two tiers: smart-extracted preview → full content**

**Reasoning**:
- The middle "compressed summary" tier is redundant if previews are well-designed
- Smart extraction (function names, file paths, code snippets, key text) gives enough signal to decide relevance
- Reduces tool surface (no `context_inspect`, just `context_preview` and `context_read`)
- The model either needs a quick signal (preview) or the full content (read) — rarely the middle ground

### Why Last-Plan-Wins for context_plan()

Multiple `context_plan()` calls in one turn: each replaces the previous entirely. The model can iterate — plan, see preview, adjust, plan again. Only the final plan gets applied on turn end. Simple, predictable, no accumulation confusion.

### Why Layered Awareness Over Full Manifest Every Turn

**Evaluated**:
1. Full manifest every turn (~200-300 tokens)
2. Delta-only (what changed since last turn)
3. **Selected: Layered — status line always, delta on change, full on demand**

**Reasoning**:
- Full manifest wastes tokens on most turns (nothing changed, model doesn't need it)
- Delta-only breaks after context compaction events
- Layered scales: ~30 tok/turn normally, ~80 tok/turn when things change, ~300 tok rarely
- Budget pressure warnings provide timely detailed info exactly when needed

---

## Open Questions (Need Experimentation)

1. **Manifest placement**: Primacy vs recency vs both? Start with primacy, experiment.
2. **First few turns**: When context is small, manifest/tools are overhead. Only inject after context reaches some threshold where shifting becomes relevant? Needs testing to find the sweet spot — early on the model remembers everything fine.
3. **Cleanup granularity**: Strip only tool calls? Or also strip the model's reasoning text about context management? (Leaning: strip tool calls only, leave reasoning text as-is)
4. **Client detection**: How does the proxy detect which client is connected to select the right runtime? Auth header format (`sk-` vs non-`sk-` Bearer), request path patterns, or explicit config?
5. **How aggressive should autonomous heuristics be?** Configurable, but what's the right default? Need real-world data.
6. **Should the model be able to create new synthetic blocks?** Like `context_note("auth uses middleware pattern")` — creating a memory that doesn't correspond to any actual conversation turn.
7. **MCP-to-engine IPC**: MCP servers run as subprocesses (stdio). Engine lives in Tauri app. Communication path needs to be determined — local HTTP to Tauri backend is the likely answer but needs validation.
8. **Model-authored compression quality**: The model writes great task-relevant summaries, but might also write lazy ones. Need guardrails? Or trust the model?
9. **Preview extraction quality**: Rule-based extraction of function names/paths/snippets — how smart does this need to be? Simple regex vs AST parsing vs heuristics?

## Resolved Questions

- **Confirmation flow**: Auto-apply on turn end. Last `context_plan()` wins. The preview IS the confirmation.
- **Compression approach**: No rule-based truncation for actual compression. Model-authored for important blocks, sidekick LLM for bulk, archival for irrelevant.
- **Block viewing tiers**: Two (preview + full), not three. Smart extraction for previews.
- **Thread splitting**: Hard rules (never split tool_call/tool_result) + model can direct splits via `context_plan()`. System defaults to atomic threads.
- **History sync**: Proxy strips context tool calls on every request pass. Claude Code's local state is irrelevant — proxy is authority on what API sees.
- **Client compatibility**: Shared core + client adapter pattern. `ContextToolRuntime` trait with ClaudeMcpRuntime (MCP native), CodexProxyRuntime (proxy-injected tools), PassiveRuntime (manifest only). MCP is a client capability, not a provider one. Graceful degradation: MCP → proxy-injected → passive.

---

## Relationship to Other Phases

### Phase 3 (This Phase) Owns
- Context tool surface with client adapters (MCP, proxy-injected, passive)
- Context planner module (signal collection, mutation planning, payload construction)
- Manifest generation (layered: status + delta + full)
- Ephemeral tool call cleanup + breadcrumbs
- Autonomous heuristics (budget pressure, staleness, task detection)
- File mutation tracking (dynamic code context updates)
- Configurable budget ceiling
- Basic relevance scoring (recency + dependency overlap + explicit signals)

### Phase 4 (Compression) Provides
- Sidekick LLM integration for automated bulk compression (Haiku, Codex mini, OpenRouter)
- Compression queue for async batch processing
- Preserve-keys system (keep error messages, stack traces verbatim)
- Quality scoring for sidekick-generated compressions
- Note: Model-authored compression is a Phase 3 feature (part of metacognition). Phase 4 adds the sidekick path for autonomous/bulk compression.

### Future Phases Build On This
- Phase 5 (Memory Lifecycle): Formal hot/warm/cold/archived states with richer transitions
- Phase 7 (Sidecar): Smarter relevance scoring, semantic overlap detection, quality verification
- Phase 8 (Search): Semantic search for @recall, embedding-based relevance

---

## Supersedes

This document supersedes `dev/active/context-awareness/design.md` (2026-02-06) which was the original brainstorm. Key evolutions:
- Proxy-intercepted @commands → Standalone MCP tools
- Immediate mutation → Plan-preview-apply pattern
- Full manifest every turn → Layered awareness
- Three-tier viewing → Two-tier (preview + full)
- Rule-based compression → Model-authored + sidekick LLM
- Threads always atomic → Model-directed splitting via context_plan
- Added: ephemeral tool calls + cleanup
- Added: dynamic code context updates
- Added: structural constraints via thread lines
- Added: configurable budget ceiling with derived thresholds
- Added: model-authored compression (model writes its own summaries)
- Added: last-plan-wins semantics for context_plan()
- Added: smart-extracted previews (function names, paths, snippets)
