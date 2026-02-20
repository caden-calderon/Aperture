## Inspiration

I was mid-session with Claude Code, deep into a refactor with multiple files open and good momentum, when it compacted. Just like that. A little summary note, and a model that had quietly forgotten half of what we'd built together that session.

The frustrating part wasn't losing the context. It was not knowing *what* was lost. There's no way to see inside the context window. No way to know what got summarized into mush, what got dropped entirely, what the model still had a grip on. You just have to keep going and hope.

It happened enough times that I changed my workflow entirely. I started preferring full `/clear` sessions over letting it compact. Better to start clean and re-read the files explicitly than work with a context I couldn't trust. That should tell you something: the "right" answer to compaction drift had become *throwing away the context on purpose*.

And it kept nagging at me. This is fundamental infrastructure for anyone doing serious work with AI coding tools, and it's completely invisible. Not just to you. The AI itself has no way to see what it knows, how full its memory is, or what's about to get dropped. It's flying blind too. It can't tell you what it still remembers from two hours ago. It can't warn you that it's about to lose something important. It just continues until it can't, and then everything gets compressed into a summary you have to hope is good enough.

That's what Aperture is about.

## What it is

When you work with an AI coding assistant, something invisible is happening in the background. Every message you send, every file the AI reads, every response it gives: all of it accumulates in what's called a context window. Think of it as the AI's working memory. It can only hold so much at once.

When that memory fills up, the tool makes a decision without asking: it compacts. It summarizes recent history, drops some of it, and continues. You get a brief notice. The AI gets a foggy, compressed version of what came before and has to work from that. Sometimes it's fine. Often it isn't. And you have no visibility into which one you got.

Aperture is a local proxy that sits between your AI tool and the provider (Anthropic, OpenAI, etc.) and intercepts every request before it goes out. It parses every message in the conversation into individual blocks (each exchange, each file read, each tool output) and tracks them in real time. You can see exactly what's in the AI's memory and how much space each piece is taking up.

More importantly: the AI can see it too.

Once both sides have visibility, you can actually manage context instead of just reacting to compaction. The AI inspects its own memory, identifies what's stale or irrelevant (old file reads, tool outputs from tasks finished an hour ago, early exploration that went nowhere) and removes them cleanly. Those blocks are stripped from every subsequent request for the rest of the session. The freed space stays freed.

No compaction. No summarization fog. No starting over.

## What it does

There's a layer of design underneath the visibility that matters: **zones**.

Research on how LLMs attend to their context shows the attention isn't uniform. Models recall things at the very beginning and very end of their context window much better than things buried in the middle. Content in the middle gets lost quietly. It's not a bug. It's how attention works across long sequences. It's just usually invisible.

Aperture makes zones explicit. Every block is assigned to one of three zones: **primacy** (stable top: system prompts, key instructions), **recency** (active bottom: recent turns), or **middle** (everything in between, where things get forgotten). The archival logic targets the middle intelligently. That's where stale blocks pile up, and where cleanup gives the most value.

The AI can also act on zones directly. If something in the middle turns out to be critical, it can promote it toward primacy where the model pays more attention. If something needs to stay fresh, it gets pushed to the trailing edge where the model sees it last. Zones go from being an invisible implementation detail to something the AI can reason about and act on.

**A real workflow.**

You're two hours into a session. You just finished the frontend work: the components render, the tests pass, the styling is locked in. The next task is the backend API. Aperture's budget bar hits 75% and fires a soft warning.

This is the moment. You tell the AI you're switching to the backend now. The AI inspects what's in context. Half of it is frontend material: component tree discussions, CSS iteration, debugging runs from attempts that didn't make it into the final code. All of it was useful an hour ago. None of it is relevant to what comes next.

The AI stages a plan: archive the frontend deep-dives, keep the final component structure (the API needs to know what it's serving), recall the backend architecture notes from the beginning of the session that got pushed into the middle and forgotten. Commits it. The next request is 30% lighter. The session can run for another hour without compaction, with a model that actually has the right things in front of it.

That's the use case. Not "AI manages context" in the abstract. It's strategic context switching at task boundaries, with enough control to keep what matters and drop what doesn't. The difference between a session that degrades into compaction fog and one that stays sharp across multiple hours of work.

**Batched cleanup.**

One thing the cache torching incident made clear: you can't do continuous micro-cleanup. Every time a block is added or removed, you're modifying the cache prefix, which means a cache miss on everything downstream. Do that constantly and you're back to paying full price on every request.

The design shifted to batched cleanup instead. Rather than small, frequent adjustments, Aperture batches cleanup at meaningful moments: threshold warnings, task boundaries, explicit AI decision points. One larger clean rather than many small ones. This means the cache stays warm between cleanup rounds, and the savings from archival aren't eaten up by constant cache churn. It's less granular than pure continuous management, but still far more controlled than a full compact. You're choosing what goes and what stays, at a moment that makes semantic sense, rather than having the tool decide everything at once when the buffer is full.

The visualization itself is a real-time block list with zone coloring and a live token budget bar with configurable soft, medium, and hard thresholds. Beyond that, the AI has a set of tools it can call to inspect and manage its own context. It can search its own memory for something specific, read the full content of any block, check how close it is to the budget ceiling, and commit cleanup plans. All of this happens inside the conversation without manual intervention.

It's not just a debugging tool. It's a memory manager the AI can actually use, at the moments that matter.

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

This incident fundamentally shaped the architecture. The manifest injection came out. The design rule became: never touch the stable prefix. Apply all mutations at the trailing edge of the conversation, the only place you can modify without invalidating everything before it. And batch the cleanup. If you're doing frequent small changes, you're paying for cache misses after every one. Do one meaningful batch at a task boundary instead. The cache stays warm between rounds, and the net savings are real.

The AI gets context awareness by explicitly calling for it, not through ambient injection. The cache stays intact. The costs stay sane. That constraint, painful as it was to discover, made the whole architecture cleaner.

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

The batched cleanup model works, but it still requires the AI to initiate. The next layer is making that automatic. The heuristics for identifying stale, low-value blocks in the middle zone are already implemented. Wiring them to fire at threshold crossings, without any AI involvement, turns Aperture from a tool the AI uses into a system that manages context on its own.

**Compression** is the other big one. Archival removes a block entirely. Compression replaces it with a shorter version, a summary the AI writes. Keep the shape of what happened, lose the token weight. The compression mutation type is already in the planner. What's missing is reliable summary generation and quality scoring. That's the next phase.

**Memory checkpointing** would let you save a context state and restore it later, or fork a session to try two different approaches from the same starting point. The engine's block versioning system was designed with this in mind. It's a natural extension of the archival model.

The longer-term vision is Aperture as the control plane for AI context: not just watching it, but actively shaping it. A system that knows you're switching tasks before you say anything, that pre-emptively clears the dead weight and recalls what you'll need next, that manages the budget the way a senior engineer manages attention: deliberately, strategically, across the full arc of a long session.

Context management is going to become one of the core competencies of working with AI. Right now it's invisible and manual. Aperture is an early step toward making it visible, controllable, and eventually autonomous. The problem is only going to get more important as sessions get longer, models get more capable, and the work done inside a single context window gets more complex.
