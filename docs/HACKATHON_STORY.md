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

Once both sides have visibility, the dynamic changes entirely. Instead of context filling up until something breaks, the AI can reason about its own memory, identify what's dead weight, and remove it cleanly. Those blocks are permanently stripped from every subsequent request for the rest of the session. The freed space stays freed.

No compaction. No summarization fog. No starting over.

## What it does

There's a layer of design underneath the visibility that matters: **zones**.

Research on how LLMs attend to their context shows the attention isn't uniform. Models recall things at the very beginning and very end of their context window much better than things buried in the middle. Content in the middle gets lost quietly. It's not a bug. It's how attention works across long sequences. It's just usually invisible.

Aperture makes zones explicit. Every block is assigned to one of three zones: **primacy** (stable top: system prompts, key instructions), **recency** (active bottom: recent turns), or **middle** (everything in between, where things get forgotten). The archival logic targets the middle intelligently. That's where stale blocks pile up and where cleanup gives the most value. The AI can also act on zones directly: promoting something critical from the middle toward primacy where the model pays more attention, or pushing something to the trailing edge to keep it fresh. Zones stop being an invisible implementation detail and become something the AI can reason about and act on.

**What metacognition actually looks like.**

You've been in a session for two hours. The frontend work is done: components render, tests pass, styling locked in. You type a new prompt: "Let's build the backend API." No instructions about context. No "please clean up first." Just the next task.

The AI, running through Aperture, can see its own context state. It knows it's at 75% capacity. It can see that half the blocks are frontend material: component tree discussions, CSS iteration, debugging runs that never made it into the final code. It can see the backend architecture notes from the start of the session, now buried in the middle zone, quietly deprioritized.

Without being told to, it reasons. The next task is clearly different from the last one. The frontend work is complete. Those blocks are dead weight now. The backend architecture notes are exactly what's needed, and they're at risk of being ignored because they've drifted into the middle. This is the right moment to clean.

It stages a plan: archive the frontend deep-dives, keep the final component structure (the API needs to know what it's serving), promote the backend architecture notes back toward the top. Commits it. Then answers your prompt, already starting from a cleaner, better-loaded context, before it's written a single line of backend code.

The session can run for another hour without compaction. And you didn't have to manage any of it.

What makes this metacognition rather than just automation is the judgment. Sometimes the AI looks at the incoming task and reasons the opposite: the next task is a small targeted fix, the overhead of a cleanup round isn't worth it, just do the work. It's not a mandatory ritual run on a timer. It's a decision made from context, about context.

**Why cleanup is batched.**

Frequent small modifications to the conversation are expensive. Every change touches the cache signature that prompt caching relies on, which can trigger cache misses on downstream content. If you clean constantly, the cache never settles and the cost savings from archiving blocks get eaten by the overhead of invalidating the cache on every turn.

So Aperture batches cleanup at meaningful moments: threshold warnings, task boundaries, explicit decision points. One larger clean at the right time rather than constant micro-adjustments. The cache stays warm between cleanup rounds. The savings are real and the disruption is minimal. This is still far more surgical than a full compact. You choose exactly which blocks go and which stay, at a moment that makes semantic sense, rather than having the tool decide everything at once when the buffer is full.

The visualization layer makes all of this tangible: a real-time block list with zone coloring, a live token budget bar with configurable soft, medium, and hard thresholds, a session selector. You can watch the AI manage its own context in real time. And if you want to intervene directly, you can pin blocks, archive manually, or adjust the budget ceiling from the settings panel.

## How I built it

Solo project, about two weeks. I'm a creative technologist, the kind of person who has ideas like this and builds prototypes to prove them out. I'm not writing compilers or airplane control systems. What I am doing is directing: architecture decisions, design choices, when to push back on an approach. The AI implements. It's genuine back-and-forth: brainstorming, debugging sessions, challenging assumptions. Not "go build this and figure it out." AI turns ideas that would have taken months into working prototypes in weeks. This project is a good example of that.

The architecture is three layers:

**A Rust proxy** (axum + tokio) handles the actual traffic. It's on the hot path for every API call, so it's zero-copy stream passthrough with async-first design throughout. When the AI commits a cleanup plan, this layer rewrites the outgoing payload to strip archived blocks before they ever reach the provider.

**A context engine** that parses every payload into semantic blocks, tracks zones, token counts, access heat, and staleness, then fires real-time events to the frontend via Tauri IPC.

**A Tauri desktop app** (Svelte 5) for the visualization layer. Real-time block list, budget bar, session selector, settings panel.

The development was phase-disciplined: UI mockup first to validate the design, then proxy core, then engine, then the AI self-management layer, then stability hardening. Design doc before implementation for each phase. 696 tests across the stack.

Late in development, the environment itself was proxied through Aperture. I watched the AI edit Aperture's own source code while Aperture tracked which context blocks it was using. Mostly for testing and validation, but it confirmed a core principle: when the proxy is working right, you forget it's there.

## Challenges

**The time I torched all my tokens.**

The zone model gave me an obvious idea: if primacy is where LLMs pay the most attention, and I want the AI to always be aware of its context status, why not inject a live status summary into the top of every request? System-level information, always visible, always fresh.

I built it. It worked great. The AI always knew its context state. And then I looked at what was happening under the hood.

Anthropic's prompt caching is prefix-based. The cache key for any given block is a hash of everything that came before it. Change anything in the sequence and every block after it is a cache miss. A dynamic summary, changing on every turn as token counts shifted, meant the entire message cache was invalidated on every single request.

The data from one diagnostic session told the story: **46 requests, 5.34 million tokens processed, 0 cache hits.** Zero. A request that should have cost around $0.12 was costing $1.24. Ten times more, per message. I hit my monthly plan limit mid-session and spilled into extra credits. The summary was burning money every time the AI said anything.

The fix required rethinking how Aperture communicates context state entirely. The injection came out. The design rule became: never touch the stable prefix. Apply all mutations at the trailing edge of the conversation, the only place you can modify without invalidating everything before it. The AI gets context awareness by explicitly calling for it, not through ambient injection. This is also what drove the batched cleanup model. Frequent updates compound the cache cost, so you batch them at moments that matter instead.

Painful, but it made the architecture cleaner than the original design would have been.

**The plan layering failure.** The hardest technical bug.

The feature: the AI commits a cleanup plan on turn 10, cleans again on turn 20, stacks another round on turn 30. Multiple passes across a long session. The bug: only the first plan ever fired. Every subsequent commit was silently a no-op.

Ten rounds of debugging. Log analysis. Diagnostic tracing wired into the session state machine. Hypothesis tracking across sessions. Each round resolved one problem only to expose another layer underneath. The actual fix was a handful of lines. The commit function was setting the pending plan but never updating the persistent ID list, so archived blocks only stayed gone if the rewriter happened to consume the plan in that exact turn, which it didn't always do.

Ten rounds to find it. The diagnostic history is in `dev/diagnostics/`.

## Accomplishments

The end-to-end loop works. The AI sees its own context, reasons about it, commits a plan, and the next request arrives lighter, without the user doing anything. That's the thing I wanted to exist when I started building this, and it exists.

Plan layering stacking correctly was the hardest verification: three successive cleanup rounds accumulating to 21 blocks persistently stripped from every subsequent turn. Getting that right is what makes multi-hour sessions actually viable.

Shipping honestly. This is beta software with documented known bugs. It works, it's useful, and the known limitations are written down rather than hidden. That's worth something.

## What I learned

The primacy/recency effect is real, and it matters more than most developers think. LLMs don't attend uniformly to their context. The beginning and end of the context window get much better recall than the middle. Once you know this, it changes how you think about context management entirely: it's not just about what's *in* the context, it's about *where* it lives. Making zones explicit and targeting the bloated middle for cleanup isn't an optimization. It's the right model.

Prompt caching is subtle enough to be a trap. The difference between a cache hit and a miss on a long session can swing costs by an order of magnitude. Every mutation, every injection, every dynamic string added to the conversation has to be evaluated against one question: does this preserve or break the prefix? That became a first-class design constraint, one I only understood after breaking it hard.

Debugging AI-adjacent systems is its own discipline. Bugs don't manifest as stack traces. They manifest as "the second plan never fired" or "the model seems to have forgotten something." You have to build the observability tooling yourself (structured tracing, log analysis, cross-session hypothesis tracking) because nothing exists for this yet. That work was as significant as building the features themselves.

The AI development workflow is still evolving, and I think most people are underestimating where it's going. Two weeks moved fast because I wasn't writing the code alone. That's not going to become less true. The projects that get built in the next few years will be shaped by how well you can direct, not how fast you can type.

## What's next for Aperture

The cleanup model currently requires the AI to initiate. The next layer is making it automatic. The heuristics for identifying stale, low-value blocks in the middle zone are already built. Wiring them to fire at threshold crossings, without any AI involvement, turns Aperture from a tool the AI uses into infrastructure that manages context on its own.

**Compression** is the other big one. Right now, archival removes a block entirely. Compression would replace it with a shorter version: a summary the AI writes, preserving the shape of what happened at a fraction of the token cost. The mutation type is already in the planner. What's missing is reliable summary generation and quality scoring.

**Memory checkpointing** would let you save a context state and restore it later, or fork a session to try two approaches from the same starting point. The engine's block versioning system was designed with this in mind.

The longer-term vision is Aperture as the control plane for AI context: not just watching it, but actively shaping it. A system that recognizes a task transition before you describe it, that clears dead weight and recalls what you'll need next, that manages the budget the way a senior engineer manages attention: deliberately, with judgment, across the full arc of a long session.

Context management is going to become one of the core competencies of working with AI. Right now it's invisible and uncontrolled. Aperture is an early step toward making it visible, deliberate, and eventually autonomous. The problem only gets more important as sessions get longer, models get more capable, and the work done inside a single context window gets more complex.
