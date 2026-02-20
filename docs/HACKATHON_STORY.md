## Inspiration

I was mid-session with Claude Code — deep into a refactor, multiple files open, good momentum — when it compacted. Just like that. I got a little summary note and a model that had quietly forgotten half of what we'd built together that session.

The frustrating part wasn't losing the context. It was not knowing *what* was lost. There's no way to see inside the context window. No way to know what got summarized into mush, what got dropped entirely, what the model still had a grip on. You just have to keep going and hope.

I started preferring full `/clear` sessions over letting it compact. Better to start clean and re-read the files explicitly than work with a context I couldn't trust. That's a bad place to be.

And it kept nagging at me — this is fundamental infrastructure for anyone doing serious work with AI coding tools, and it's completely invisible. Not just to you. The AI itself has no way to see what it knows, how full its memory is, or what's about to get dropped. It's flying blind too.

That's what Aperture is about.

## What it does

Aperture is a transparent local proxy. You point Claude Code (or Codex, or any OpenAI-compatible tool) at `localhost:5400` instead of the real API, and Aperture sits in the middle — intercepting every request, parsing the conversation into semantic blocks, and giving you a live view of what's actually in context.

At the most basic level: you can finally *see* it. Every block. Every tool call result. Every file read. How many tokens each one costs. How stale it is.

But there's a layer of design underneath the visualization that matters: **zones**.

There's well-studied research on how LLMs actually attend to their context — it's not uniform. Models tend to recall things at the very beginning (primacy) and very end (recency) of their context window much better than things in the middle. Content in the middle gets lost. This isn't a bug; it's how attention mechanisms distribute across long sequences. It's just usually invisible.

Aperture makes zones explicit. Every block is assigned to one of three zones — **primacy** (stable top: system prompts, key instructions), **recency** (active bottom: recent turns), or **middle** (everything in between, where things quietly get forgotten). The UI renders these zones clearly. The archival heuristics target them intelligently — the middle is where stale blocks pile up and where cleanup gives the most value.

The AI can also act on zones directly. The `shift_to` mutation lets it move a block from the middle to primacy if it's actually important, or push something to the trailing edge so it stays fresh in recency. The whole zone model becomes a first-class tool rather than an invisible implementation detail.

Beyond visualization: Aperture exposes 5 MCP tools the AI can call to inspect and manage its own context. The workflow: the AI calls `aperture_context_preview`, sees everything in its memory with token counts and zone assignments, identifies the dead weight — old tool results, file reads from three tasks ago, all sitting in the middle — stages an archival plan, and commits it. Those blocks get permanently stripped from every subsequent request for the rest of the session. The freed tokens stay freed. It doesn't need to clean up again next turn.

It's not just a debugging tool. It's a memory manager the AI can actually use.

## How we built it

Solo project, about two weeks, built with AI coding assistants doing most of the heavy lifting — which made for an interesting development loop given what we were building.

The architecture is three layers:

**A Rust proxy** (axum + tokio) handles the actual traffic. It's on the hot path for every API call, so it's zero-copy stream passthrough with async-first design throughout. This layer also handles the mutation rewriting — when the AI commits a plan, Aperture modifies the outgoing payload to strip archived blocks before they ever reach the provider.

**A context engine** that parses every payload into semantic blocks, tracks zones, token counts, access heat, and staleness — then fires real-time events to the frontend via Tauri IPC.

**A Tauri desktop app** (Svelte 5) for the visualization layer. Real-time block list with zone coloring, token budget bar with configurable thresholds, session selector, settings panel.

The development was phase-disciplined: UI mockup first to validate the design, then proxy core, then engine, then the metacognition layer (MCP tools + staged planning), then stability hardening. Design doc before implementation for each phase. 696 tests across the stack.

Late in development, the environment itself was proxied through Aperture — watching the AI edit Aperture's own source code while Aperture tracked which context blocks it was using. Mostly for testing and validation, but it confirmed a core principle: when the proxy is working right, you forget it's there.

## Challenges we ran into

**The plan layering failure** — the hardest technical bug.

The feature: the AI commits an archival plan on turn 10, cleans again on turn 20, stacks another round on turn 30. Multiple cleanup passes across a long session. The bug: only the first plan ever fired. Every subsequent commit was silently a no-op.

Ten rounds of debugging. JSONL log analysis. Diagnostic tracing wired into the session state machine. Hypothesis tracking across sessions. Narrowing it down to a race between two code paths with different ideas about which session was "current." The actual fix was a handful of lines — `commit_staged_plan_for_session()` was setting the pending plan but never updating `persistent_archived_ids`, so IDs only persisted if the rewriter consumed the plan in the same turn, which it didn't always do.

Ten rounds to find it. The debugging history is in `dev/diagnostics/`.

**The time we torched all our tokens.**

The zone system gave us an obvious idea: if primacy is where LLMs pay the most attention, and we want the AI to always be aware of its context status, why not inject a live context manifest into the top of every request? System-level information, always visible, always fresh.

We built it. It worked great — the AI always knew its context state. And then we looked at the API costs.

Anthropic's prompt caching is prefix-based. The cache key for any given block is a hash of everything that came before it. Change anything in the sequence and every block after it is a cache miss. A dynamic manifest injected at the top of every request meant the *entire* message cache was invalidated on every single turn. We were paying full price on hundreds of thousands of tokens per request, every turn, when a cached session should have cost a fraction of that. The manifest was burning money every time the AI sent a message.

The fix required pulling the manifest out entirely and redesigning how Aperture communicates context state. Now instead of injecting information that breaks the cache, Aperture applies mutations at the trailing edge of the conversation — the only place you can safely modify without invalidating everything before it. The AI gets context awareness through the MCP tools it explicitly calls, not through ambient injection. The cache stays intact. The costs stay sane.

It was a painful lesson, but it shaped the whole mutation rewriting architecture into something cleaner.

## Accomplishments that we're proud of

Getting the end-to-end loop working: the AI sees its own context, proposes a plan, commits it, and the next request is lighter. No manual intervention. No interrupting the conversation. The AI just takes care of it.

Plan layering stacking correctly. Verified: three successive archive rounds (8+8+5 = 21 blocks total), all accumulating, all stripping correctly from every subsequent turn. That was the hardest thing to get right.

The cache-safe mutation design. Using Aperture doesn't make your API bills worse. All mutations preserve the cache prefix on both Anthropic and OpenAI — and that constraint, learned the hard way, made the architecture more principled.

Shipping honestly. This is beta software with documented known bugs and a known-issues section. It's not polished. But it works, it's useful, and it'll be in active use on future projects.

## What we learned

**LLMs don't attend uniformly to their context.** The primacy/recency effect is real and measurable. Building tooling around it — making zones explicit, targeting the middle for cleanup, letting the AI shift important content toward the edges — turns an invisible limitation into something you can actually work with.

**Prompt caching is subtle and expensive to get wrong.** The difference between a cache hit and a miss on a long session can be a 10x cost swing. Every context mutation has to be evaluated against the question: does this preserve or break the cache prefix? That constraint became a first-class design requirement.

**LLM coding tools are stateless.** Claude Code sends your full conversation history on every API call. Any context management layer has to be side-channel — applied transparently to the payload stream, every turn, without the tool knowing.

**Debugging AI-adjacent systems is its own discipline.** Bugs manifest as "the second plan never fired" or "the AI seems to have forgotten something." Not a stack trace in sight. Building the diagnostic tooling for these failures — structured tracing, log analysis, hypothesis tracking — was as much work as building the features themselves.

**Two weeks moves fast when you're not writing the code alone.** The AI handled most of the implementation. My job was architecture, decisions, and debugging. That dynamic is only going to become more common.

## What's next for Aperture

**Compression** — The `compress` mutation type is already in the planner. What's missing is reliable summary generation: the AI writes a replacement for a long block, Aperture stores it, and the block shrinks instead of disappearing. That's the next big feature.

**Autonomous budget management** — The heuristics for identifying stale, low-value blocks in the middle zone are already implemented. The next step is wiring them to automatic action at budget thresholds, without requiring the AI to initiate a cleanup.

**Memory checkpointing** — Save a context state and restore it later. Fork a session to try two different approaches. The engine's block versioning system was designed with this in mind.

**Better budget accuracy** — Closing the ~17.5% gap between Aperture's estimate and Claude Code's `/context` readout by including tool schema and system prompt overhead.

The longer-term vision is Aperture as the control plane for AI context — not just watching it, but actively shaping it so long sessions stay coherent, cheap, and on track.
