# Phase 4 Codebase Refactor — Master Plan

**Goal**: Systematic, exhaustive codebase review and refactoring across backend, frontend, and docs.
Zero regressions. Every file audited. No stone left unturned. This is done once, done right.

**Approach**: Explore first, execute later. Build complete knowledge before making changes.
Do not touch source files during exploration. The audit table is the deliverable.

**Standard**: Staff engineer reviewing an inherited codebase before a major refactor.
Not a skim — a deep read. Every function. Every design decision. Every implicit assumption.
The audit table must be complete enough that another engineer could execute the refactor
without reading the source files themselves.

**What to look for beyond the obvious:**
- What's *missing* — no error handling for a real failure mode, no tests for an edge case, no doc on a non-obvious invariant
- Coupling that shouldn't exist — a file knowing about things outside its responsibility boundary
- Asymmetry — two similar things done differently for no apparent reason
- Duplication across files — same logic copy-pasted, same wrapper function defined twice
- Dead code — computed fields never read, wrapper functions that add nothing, unreachable branches
- Implicit contracts — code that only works if callers follow an undocumented rule

**Philosophy on splitting**: Line counts are guidelines for flagging, not mandates for splitting.
A 70-line function that does one coherent thing is better than 3 awkward helpers that fragment
the logic. A 500-line file that is genuinely cohesive stays as-is. The question is always:
"Does splitting this actually make it easier to understand, maintain, and work with — or does
it just make the number smaller?" If the answer is the latter, leave it. Unnecessary abstractions
and indirection for its own sake are worse than a slightly larger file.

---

## Phase A: Backend (Rust) — `src-tauri/src/`

### A.0: Exploration (Sessions 1-N)

> **Working instructions**: See `context.md` → "A.0 Exploration — Session Protocol" for the
> full session rules (batch sizes, update cadence, synthesis requirements, context limits).
> **Current progress**: See `context.md` → "A.0 Current Progress" for what's been read and what's next.

Go file-by-file through every `.rs` file. For each file, document:
- **Purpose**: What does this file do? What module does it belong to?
- **Size**: Line count. Flag >400 for review (not automatic split — reason about it).
- **Tests**: Inline `#[cfg(test)]` modules? Where should they go?
- **Functions**: Any notably large? Are they doing one coherent thing or multiple concerns?
- **Code quality**: Dead code, outdated patterns, unwrap/expect, error handling.
- **Dependencies**: What does it import? What imports it? How tightly coupled?
- **Comments**: Are they explaining "why"? Missing where logic is complex?
- **Bugs/concerns**: Anything suspicious, risky, or architecturally wrong?
- **Organization**: Is this file in the right place? Should it be grouped with related files?

#### Detail Standard for Audit Entries

Entries must be specific enough that a developer can implement the fix without re-reading the source file. Vague entries are useless at execution time.

**Bad**: `"Tool chain matching bug — fix it"`
**Good**: `"build_dependencies() line 181 matches ToolUse→ToolResult by tool_name string. If read_file is called twice in one session, find() returns the first match for both ToolUse blocks — second one gets a stale or wrong result. Fix: add tool_use_id: Option<String> to BlockMetadata, populate in the parser, match by ID in build_dependencies() with name as fallback."`

For every issue logged:
- **Where exactly**: file + function + line number if possible
- **What it does wrong**: the precise mechanism, not just "it's wrong"
- **Why it matters**: what breaks, under what conditions
- **How to fix it**: concrete enough to start writing code from

#### Uncertain Findings

If you spot something that looks wrong but you'd need to see another file to confirm — **log it anyway** in the Uncertain Findings table below. Do not discard observations just because you can't fully verify them from the current file. Mark it uncertain, note what would confirm or deny it, and move on. The reconciliation pass will investigate.

Build this into a **file-by-file audit table** below as we explore.

### A.1: Test Extraction
**Convention**: Tests go in `tests/` directories, one test file per concern.
```
engine/
  mod.rs
  tests/              ← NOT a single tests.rs
    mod.rs
    ingest_tests.rs
    session_tests.rs
    budget_tests.rs
    ...
engine/planner/
  mod.rs
  tests/
    mod.rs
    heuristic_tests.rs
    cleanup_tests.rs
    applicator_tests.rs
    ...
```
- NO inline `#[cfg(test)] mod tests {}` in source files
- NO single mega `tests.rs` that becomes a token bomb
- Each test file focused on one concern, named descriptively
- Test files import from parent module, use `super::*` or explicit imports

### A.2: File Splitting (Only Where It Makes Sense)
Candidates identified during exploration. Each one gets a reasoned decision:
- Does splitting improve clarity, or just shrink the number?
- Are there clean responsibility boundaries, or would the split be artificial?
- Would a reader need to jump between files to understand the logic?

Preliminary candidates (to be validated during exploration):
- `cleanup.rs` (1212 lines) — 3 format handlers, possibly natural split
- `heuristics.rs` (1221 lines) — review for cohesion before deciding
- `metacog/tools.rs` (752 lines) — 5 independent tools, likely good split
- `applicator.rs` (812 lines) — review; may be one coherent pipeline
- `engine/mod.rs` (937 lines) — review; coordinator files are naturally larger

### A.3: Module Organization
Group related files into directories where it makes sense. Move things that are in the wrong place.

### A.4: Code Quality Pass
- Remove dead code, unused imports
- Fix any remaining unwrap/expect
- Improve error handling patterns
- Add "why" comments where logic is non-obvious
- Fix any bugs found during exploration

### A.5: Verify
- `cargo test` — all tests pass
- `cargo clippy` — zero warnings
- `cargo fmt` — clean formatting

---

## Phase B: Frontend (Svelte/TS) — `src/`

### B.0: Exploration
Same file-by-file audit as Phase A.

### B.1: Component Decomposition
- `+page.svelte` (1463 lines) → extract subcomponents
- `Modal.svelte` (1095 lines) → extract editor/diff/timeline
- Other large components → split at responsibility boundaries

### B.2: Store Refactoring
- `context.svelte.ts` (1147 lines) → split into focused stores
- Relocate `mock-data.ts` out of main lib

### B.3: Type Safety
- Replace `as unknown as` casts with type guards
- Add JSDoc to public store APIs

### B.4: Verify
- `npm run test` — all tests pass
- `npm run check` — zero diagnostics

---

## Phase C: Docs & Project Structure

### C.1: Directory Navigation
- Full `REPO_STRUCTURE.md` rewrite with tree + descriptions
- Update `DOCS_INDEX.md`

### C.2: Archive Stale Docs
- Move completed `dev/active/` items to `dev/archive/`
- Archive outdated phase docs

### C.3: Sync Documentation
- `CLAUDE.md` — sync with current architecture
- `memory/bugs.md` — prune outdated bug index entries
- Phase docs — verify accuracy

---
