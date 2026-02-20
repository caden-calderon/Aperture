## Inspiration

I was mid-session with Claude Code, deep into a refactor with multiple files open and good momentum, when it compacted. Just like that. A little summary note, and a model that had quietly forgotten half of what we'd built together that session.

The frustrating part wasn't losing the context. It was not knowing *what* was lost. There's no way to see inside the context window. No way to know what got summarized into mush, what got dropped entirely, what the model still had a grip on. You just have to keep going and hope.

It happened enough times that I changed my workflow entirely. I started preferring full `/clear` sessions over letting it compact. Better to start clean and re-read the files explicitly than work with a context I couldn't trust. That should tell you something: the "right" answer to compaction drift had become *throwing away the context on purpose*.

And it kept nagging at me. This is fundamental infrastructure for anyone doing serious work with AI coding tools, and it's completely invisible. Not just to you. The AI itself has no way to see what it knows, how full its memory is, or what's about to get dropped. It's flying blind too.

That's what Aperture is about.

## What it does

Aperture is a transparent local proxy. You point Claude Code (or Codex, or any OpenAI-compatible tool) at `localhost:5400` instead of the real API, and Aperture sits in the middle: intercepting every request, parsing the conversation into semantic blocks, and giving you a live view of what's actually in context.

At the most basic level: you can finally *see* it. Every block. Every tool call result. Every file read. How many tokens each one costs. How stale it is.

But there's a layer of design underneath the visualization that matters: **zones**.

There's well-studied research on how LLMs actually attend to their context. It's not uniform. Models tend to recall things at the very beginning (primacy) and very end (recency) of their context window much better than things in the middle. Content in the middle gets lost. This isn't a bug; it's how attention mechanisms distribute across long sequences. It's just usually invisible.

Aperture makes zones explicit. Every block is assigned to one of three zones: **primacy** (stable top: system prompts, key instructions), **recency** (active bottom: recent turns), or **middle** (everything in between, where things quietly get forgotten). The UI renders these zones clearly. The archival heuristics target them intelligently. The middle is where stale blocks pile up, and where cleanup gives the most value.

The AI can also act on zones directly. The `shift_to` mutation lets it move a block from the middle to primacy if it's actually important, or push something to the trailing edge so it stays fresh in recency. The whole zone model becomes a first-class tool rather than an invisible implementation detail.

Beyond visualization: Aperture exposes 5 MCP tools the AI can call to inspect and manage its own context. The workflow: the AI calls `aperture_context_preview`, sees everything in its memory with token counts and zone assignments, identifies the dead weight (old tool results, file reads from three tasks ago, all sitting in the middle), stages an archival plan, and commits it. Those blocks get permanently stripped from every subsequent request for the rest of the session. The freed tokens stay freed. It doesn't need to clean up again next turn.

It's not just a debugging tool. It's a memory manager the AI can actually use.

## How we built it

Solo project, about two weeks. I'm a creative technologist, the kind of person who comes up with ideas like this and builds prototypes to prove them out. I'm not writing compilers or airplane control systems. What I am doing is directing: the architecture decisions, the design choices, what to build and why, when to push back on an approach. The AI implements. It's genuine back-and-forth: brainstorming, debugging sessions, challenging assumptions. Not "go build this and figure it out." AI turns ideas that would have taken months into working prototypes in weeks. This project is a good example of that.

The architecture is three layers:

**A Rust proxy** (axum + tokio) handles the actual traffic. It's on the hot path for every API call, so it's zero-copy stream passthrough with async-first design throughout. This layer also handles the mutation rewriting. When the AI commits a plan, Aperture modifies the outgoing payload to strip archived blocks before they ever reach the provider.

**A context engine** that parses every payload into semantic blocks, tracks zones, token counts, access heat, and staleness, then fires real-time events to the frontend via Tauri IPC.

**A Tauri desktop app** (Svelte 5) for the visualization layer. Real-time block list with zone coloring, token budget bar with configurable thresholds, session selector, settings panel.

The development was phase-disciplined: UI mockup first to validate the design, then proxy core, then engine, then the metacognition layer (MCP tools + staged planning), then stability hardening. Design doc before implementation for each phase. 696 tests across the stack.

Late in development, the environment itself was proxied through Aperture. We watched the AI edit Aperture's own source code while Aperture tracked which context blocks it was using. Mostly for testing and validation, but it confirmed a core principle: when the proxy is working right, you forget it's there.

## Challenges we ran into

**The time we torched all our tokens.**

The zone system gave us an obvious idea: if primacy is where LLMs pay the most attention, and we want the AI to always be aware of its context status, why not inject a live context manifest into the top of every request? System-level information, always visible, always fresh.

We built it. It worked great. The AI always knew its context state. And then we looked at what was happening under the hood.

Anthropic's prompt caching is prefix-based. The cache key for any given block is a hash of everything that came before it. Change anything in the sequence and every block after it is a cache miss. A dynamic manifest, changing on every turn as token counts shifted, meant the *entire* message cache was invalidated on every single request.

The data from one diagnostic session told the story clearly: **46 requests, 5.34 million cache_creation tokens, 0 cache reads**. Zero. Not a single cache hit across the whole session. A request that should have cost around $0.12 was costing $1.24. Ten times more, per message. I hit my monthly plan limit mid-session and spilled into extra credits. The manifest was burning money every time the AI said anything.

The fix required pulling the manifest out entirely and redesigning how Aperture communicates context state. Now instead of injecting information that breaks the cache, Aperture applies mutations at the trailing edge of the conversation, the only place you can safely modify without invalidating everything before it. The AI gets context awareness through the MCP tools it explicitly calls, not through ambient injection. The cache stays intact. The costs stay sane.

A painful lesson, but it shaped the whole mutation rewriting architecture into something cleaner.

**The plan layering failure.** The hardest technical bug.

The feature: the AI commits an archival plan on turn 10, cleans again on turn 20, stacks another round on turn 30. Multiple cleanup passes across a long session. The bug: only the first plan ever fired. Every subsequent commit was silently a no-op.

Ten rounds of debugging. JSONL log analysis. Diagnostic tracing wired into the session state machine. Hypothesis tracking across sessions. Each round resolved one problem only to expose another layer underneath. The actual fix was a handful of lines. `commit_staged_plan_for_session()` was setting the pending plan but never updating `persistent_archived_ids`, so IDs only persisted if the rewriter consumed the plan in the same turn, which it didn't always do.

Ten rounds to find it. The diagnostic history is in `dev/diagnostics/`.

## Accomplishments that we're proud of

Getting the end-to-end loop working: the AI sees its own context, proposes a plan, commits it, and the next request is lighter. No manual intervention. No interrupting the conversation. The AI just takes care of it.

Plan layering stacking correctly. Verified: three successive archive rounds (8+8+5 = 21 blocks total), all accumulating, all stripping correctly from every subsequent turn. That was the hardest thing to get right.

The cache-safe mutation design. Using Aperture doesn't make your API bills worse. All mutations preserve the cache prefix on both Anthropic and OpenAI, and that constraint, learned the hard way, made the architecture more principled.

Shipping honestly. This is beta software with documented known bugs and a known-issues section. It's not polished. But it works, it's useful, and it'll be in active use on future projects.

## What we learned

The primacy/recency effect is real, and it matters more than most developers think. LLMs don't attend uniformly to their context. The beginning and end of the context window get much better recall than the middle. Once you know this, it changes how you think about context management entirely: it's not just about what's *in* the context, it's about *where* it lives. Making zones explicit and targeting the bloated middle for cleanup isn't an optimization, it's the right model.

Prompt caching is subtle enough to be a trap. The difference between a cache hit and a miss on a long session can swing costs by an order of magnitude. Every mutation, every injection, every dynamic string you add to the conversation has to be evaluated against one question: does this preserve or break the prefix? That became a first-class design constraint, one we only understood after breaking it hard. And debugging AI-adjacent systems is its own discipline. Bugs manifest as "the second plan never fired" or "the AI seems to have forgotten something." No stack traces. No obvious error states. Building the diagnostic tooling (structured tracing, JSONL log analysis, cross-session hypothesis tracking) was as much work as building the features themselves. You have to instrument everything, because the signal is subtle.

The AI development workflow is still evolving, and I think most people are underestimating where it's going. Two weeks moved fast because I wasn't writing the code alone. That's not going to become less true. The projects that get built in the next few years will be shaped by how well you can direct, not how fast you can type.

## What's next for Aperture

**Compression.** The `compress` mutation type is already in the planner. What's missing is reliable summary generation: the AI writes a replacement for a long block, Aperture stores it, and the block shrinks instead of disappearing. That's the next big feature.

**Autonomous budget management.** The heuristics for identifying stale, low-value blocks in the middle zone are already implemented. The next step is wiring them to automatic action at budget thresholds, without requiring the AI to initiate a cleanup.

**Memory checkpointing.** Save a context state and restore it later. Fork a session to try two different approaches. The engine's block versioning system was designed with this in mind.

**Better budget accuracy.** Closing the ~17.5% gap between Aperture's estimate and Claude Code's `/context` readout by including tool schema and system prompt overhead.

The longer-term vision is Aperture as the control plane for AI context: not just watching it, but actively shaping it so long sessions stay coherent, cheap, and on track.
