## Inspiration

I was mid-session with Claude Code, deep into a refactor with multiple files open and good momentum, when it compacted. Just like that. A little summary note, and a model that had quietly forgotten half of what we'd built together that session.

The frustrating part wasn't losing the context. It was not knowing *what* was lost. There's no way to see inside the context window. No way to know what got summarized into mush, what got dropped entirely, what the model still had a grip on. You just have to keep going and hope.

It happened enough times that I changed my workflow entirely. I started preferring full `/clear` sessions over letting it compact. Better to start clean and re-read the files explicitly than work with a context I couldn't trust. That should tell you something: the "right" answer to compaction drift had become *throwing away the context on purpose*.

And it kept nagging at me. This is fundamental infrastructure for anyone doing serious work with AI coding tools, and it's completely invisible. Not just to you. The AI itself has no way to see what it knows, how full its memory is, or what's about to get dropped. It's flying blind too.

That's what Aperture is about.

## What it is

When you work with an AI coding assistant, something invisible is happening in the background. Every message you send, every file the AI reads, every response it gives: all of it accumulates in what's called a context window. Think of it as the AI's working memory. It can only hold so much at once.

When that memory fills up, the tool makes a decision without asking: it compacts. It summarizes recent history, drops some of it, and continues. You get a brief notice. The AI gets a foggy, compressed version of what came before and has to work from that. Sometimes it's fine. Often it isn't.

Aperture is a local proxy that sits between your AI tool and the provider (Anthropic, OpenAI, etc.) and intercepts every request before it goes out. It parses every message in the conversation into individual blocks (each exchange, each file read, each tool output) and tracks them in real time. You can see exactly what's in the AI's memory and how much space each piece is taking up.

More importantly: the AI can see it too.

Once both sides have visibility, you can actually manage context instead of just reacting to compaction. The AI inspects its own memory, identifies what's stale or irrelevant (old file reads, tool outputs from tasks finished an hour ago, early exploration that's long been resolved) and removes them cleanly. Those blocks are stripped from every subsequent request for the rest of the session. The freed space stays freed.

No compaction. No summarization fog. No starting over.

## What it does

There's a layer of design underneath the visibility that matters: **zones**.

Research on how LLMs attend to their context shows the attention isn't uniform. Models recall things at the very beginning and very end of their context window much better than things buried in the middle. Content in the middle gets lost quietly. It's not a bug. It's how attention works across long sequences. It's just usually invisible.

Aperture makes zones explicit. Every block is assigned to one of three zones: **primacy** (stable top: system prompts, key instructions), **recency** (active bottom: recent turns), or **middle** (everything in between, where things get forgotten). The archival logic targets the middle intelligently. That's where stale blocks pile up, and where cleanup gives the most value.

The AI can also act on zones directly. If something in the middle turns out to be critical, it can promote it toward primacy where the model pays more attention. If something needs to stay fresh, it gets pushed to the trailing edge where the model sees it last.

The visualization itself is a real-time block list with zone coloring and a live token budget bar. Beyond that, the AI has a set of tools it can call to inspect and manage its own context. It can search its own memory for something specific, read the full content of any block, check how close it is to the budget ceiling, and commit cleanup plans. All of this happens inside the conversation without any manual intervention.

It's not just a debugging tool. It's a memory manager the AI can actually use.

## How I built it

Solo project, about two weeks. I'm a creative technologist, the kind of person who comes up with ideas like this and builds prototypes to prove them out. I'm not writing compilers or airplane control systems. What I am doing is directing: the architecture decisions, the design choices, what to build and why, when to push back on an approach. The AI implements. It's genuine back-and-forth: brainstorming, debugging sessions, challenging assumptions. Not "go build this and figure it out." AI turns ideas that would have taken months into working prototypes in weeks. This project is a good example of that.

The architecture is three layers:

**A Rust proxy** (axum + tokio) handles the actual traffic. It's on the hot path for every API call, so it's zero-copy stream passthrough with async-first design throughout. This layer also handles the mutation rewriting. When the AI commits a cleanup plan, Aperture modifies the outgoing payload to strip archived blocks before they ever reach the provider.

**A context engine** that parses every payload into semantic blocks, tracks zones, token counts, access heat, and staleness, then fires real-time events to the frontend via Tauri IPC.

**A Tauri desktop app** (Svelte 5) for the visualization layer. Real-time block list with zone coloring, token budget bar with configurable thresholds, session selector, settings panel.

The development was phase-disciplined: UI mockup first to validate the design, then proxy core, then engine, then the AI self-management layer, then stability hardening. Design doc before implementation for each phase. 696 tests across the stack.

Late in development, the environment itself was proxied through Aperture. I watched the AI edit Aperture's own source code while Aperture tracked which context blocks it was using. Mostly for testing and validation, but it confirmed a core principle: when the proxy is working right, you forget it's there.

## Challenges

**The time I torched all my tokens.**

The zone model gave me an obvious idea: if primacy is where LLMs pay the most attention, and I want the AI to always be aware of its context status, why not inject a live status summary into the top of every request? System-level information, always visible, always fresh.

I built it. It worked great. The AI always knew its context state. And then I looked at what was happening under the hood.

Anthropic's prompt caching is prefix-based. The cache key for any given block is a hash of everything that came before it. Change anything in the sequence and every block after it is a cache miss. A dynamic summary, changing on every turn as token counts shifted, meant the entire message cache was invalidated on every single request.

The data from one diagnostic session told the story clearly: **46 requests, 5.34 million tokens processed, 0 cache hits.** Zero. A request that should have cost around $0.12 was costing $1.24. Ten times more, per message. I hit my monthly plan limit mid-session and spilled into extra credits. The summary was burning money every time the AI said anything.

The fix required pulling the injection out entirely and redesigning how Aperture communicates context state. Instead of injecting information that breaks the cache, Aperture now applies all mutations at the trailing edge of the conversation, the only place you can safely modify without invalidating everything before it. The AI gets context awareness by explicitly asking for it, not through ambient injection. The cache stays intact. The costs stay sane.

A painful lesson, but it shaped the whole architecture into something cleaner.

**The plan layering failure.** The hardest technical bug.

The feature: the AI commits a cleanup plan on turn 10, cleans again on turn 20, stacks another round on turn 30. Multiple passes across a long session. The bug: only the first plan ever fired. Every subsequent commit was silently a no-op.

Ten rounds of debugging. Log analysis. Diagnostic tracing wired into the session state machine. Hypothesis tracking across sessions. Each round resolved one problem only to expose another layer underneath. The actual fix was a handful of lines. The commit function was setting the pending plan but never updating the persistent ID list, so archived blocks only stayed gone if the rewriter happened to consume the plan in that exact turn, which it didn't always do.

Ten rounds to find it. The diagnostic history is in `dev/diagnostics/`.

## Accomplishments

Getting the end-to-end loop working: the AI sees its own context, proposes a plan, commits it, and the next request is lighter. No manual intervention. No interrupting the conversation. The AI just takes care of it.

Plan layering stacking correctly. Verified: three successive cleanup rounds (8+8+5 = 21 blocks total), all accumulating, all stripping correctly from every subsequent turn. That was the hardest thing to get right.

The cache-safe mutation design. Using Aperture doesn't make API bills worse. All mutations preserve the cache prefix on both Anthropic and OpenAI. That constraint, learned the hard way, made the architecture more principled.

Shipping honestly. This is beta software with documented known bugs. It's not polished. But it works, it's useful, and it'll be in active use on future projects.

## What I learned

The primacy/recency effect is real, and it matters more than most developers think. LLMs don't attend uniformly to their context. The beginning and end of the context window get much better recall than the middle. Once you know this, it changes how you think about context management entirely: it's not just about what's *in* the context, it's about *where* it lives. Making zones explicit and targeting the bloated middle for cleanup isn't an optimization. It's the right model.

Prompt caching is subtle enough to be a trap. The difference between a cache hit and a miss on a long session can swing costs by an order of magnitude. Every mutation, every injection, every dynamic string added to the conversation has to be evaluated against one question: does this preserve or break the prefix? That became a first-class design constraint, one I only understood after breaking it hard. And debugging AI-adjacent systems is its own discipline. Bugs manifest as "the second plan never fired" or "the AI seems to have forgotten something." No stack traces. No obvious error states. Building diagnostic tooling (structured tracing, log analysis, cross-session hypothesis tracking) was as much work as building the features themselves.

The AI development workflow is still evolving, and I think most people are underestimating where it's going. Two weeks moved fast because I wasn't writing the code alone. That's not going to become less true. The projects that get built in the next few years will be shaped by how well you can direct, not how fast you can type.

## What's next for Aperture

**Compression.** The compression mutation type is already in the planner. What's missing is reliable summary generation: the AI writes a replacement for a long block, Aperture stores it, and the block shrinks instead of disappearing. That's the next big feature.

**Autonomous budget management.** The heuristics for identifying stale, low-value blocks in the middle zone are already implemented. The next step is wiring them to automatic action at budget thresholds, without requiring the AI to initiate a cleanup.

**Memory checkpointing.** Save a context state and restore it later. Fork a session to try two different approaches. The engine's block versioning system was designed with this in mind.

**Better budget accuracy.** Closing the gap between Aperture's estimate and what the AI tool reports by including tool schema and system prompt overhead.

The longer-term vision is Aperture as the control plane for AI context: not just watching it, but actively shaping it so long sessions stay coherent, cheap, and on track.
