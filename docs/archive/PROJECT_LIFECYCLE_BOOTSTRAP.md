# Project Lifecycle Bootstrap Guide

> **Give this document to a new Claude Code session to set up structured project lifecycle management.**
>
> This system enables multi-session continuity, phased development, and context preservation across LLM conversations.

---

## Philosophy: Agent Handoff at Scale

### The Core Insight

LLM agents lose context between sessions. But with proper documentation, **each agent becomes a relay runner** — picking up exactly where the last one left off, running their leg, and handing off cleanly.

The result: **minimal prompting from the human**. After setting up this system, your entire prompt becomes:

```
Read .context/RESUME.md, follow its reading instructions, gather your initial context, and start coding.
```

That's it. The documentation does the rest.

### How It Works

1. **RESUME.md is the entry point** — Every agent reads this first
2. **Phase docs contain everything needed** — Context from previous phases, deliverables, success criteria
3. **Updates happen constantly** — As work progresses, docs update
4. **Handoff is automatic** — The next agent reads updated docs and continues

### The Handoff Pattern

```
Agent 1: Reads RESUME.md → Does Phase N work → Updates docs → Completes

Agent 2: Reads RESUME.md → Sees Phase N complete → Reads Phase N+1 → Continues
```

Each agent is **self-sufficient**. They don't need you to explain what happened before. They read, understand, and continue.

---

## Core Principles

1. **RESUME.md is the source of truth** — Always start here
2. **Phases stay under 80% context** — Each phase completes in one session
3. **Context flows forward** — Completed phases add context to next phase docs
4. **State survives compaction** — Critical info lives in files, not conversation
5. **Decisions are logged** — Key Decisions Log prevents re-debates
6. **Nothing ships without tests** — Unit, integration, AND manual tests

---

## Quality Standards: FAANG-Level Without Over-Engineering

### The Balance

We aim for production-quality code that could ship at a top tech company:
- Clean architecture
- Comprehensive tests
- Strict typing
- Consistent style

**But we don't over-engineer:**
- No premature abstraction
- No speculative features
- No complexity without clear benefit
- Simple solutions preferred

### Testing Philosophy: We Don't Continue Until It Works

Every phase includes three test categories:

| Type | Purpose | When Run |
|------|---------|----------|
| **Unit Tests** | Test individual functions/classes in isolation | `make test` (automated) |
| **Integration Tests** | Test components working together | `make test-int` (automated) |
| **Manual Tests** | Test real-world scenarios with actual services | Human-verified |

**The rule:** A phase is not complete until ALL tests pass.

This isn't bureaucracy — it's how you build reliable systems. Tests catch bugs early, document behavior, and give confidence for refactoring.

---

## Repository Structure

### Directory Layout

```
your-project/
├── .context/                    # LLM context (START HERE)
│   ├── RESUME.md               # Entry point for every session
│   ├── CODE_STANDARDS.md       # Code quality rules
│   ├── phases/                 # Phase documentation
│   │   ├── phase-1.md
│   │   ├── phase-2.md
│   │   └── ...
│   └── components/             # Component specifications
│       └── ...
│
├── src/project/                # Source code (clean, organized)
│   ├── __init__.py
│   ├── module_a/
│   ├── module_b/
│   └── shared/                 # Shared utilities
│
├── tests/                      # Test suite (mirrors src structure)
│   ├── unit/                   # Unit tests
│   │   ├── module_a/
│   │   └── module_b/
│   ├── integration/            # Integration tests
│   └── manual/                 # Manual test scripts
│       ├── README.md           # How to run manual tests
│       ├── run_phase1_tests.py
│       └── run_phase2_tests.py
│
├── config/                     # YAML configuration files
│   ├── module_a.yaml
│   └── module_b.yaml
│
├── docs/                       # User-facing documentation
│   └── ARCHITECTURE.md
│
├── Makefile                    # Developer commands
├── pyproject.toml              # Project config + tool settings
├── .env.example                # Environment template
└── .gitignore
```

### Test Directory Mirrors Source

Tests mirror the source structure exactly:

```
src/project/           →    tests/unit/
├── athena/            →    ├── athena/
│   ├── session.py     →    │   └── test_session.py
│   └── extractor.py   →    │   └── test_extractor.py
├── plugins/           →    ├── plugins/
│   ├── weather/       →    │   └── weather/
│   └── quant/         →    │   └── quant/
└── safety/            →    └── safety/
```

This makes finding tests trivial: `src/project/foo/bar.py` → `tests/unit/foo/test_bar.py`

---

## Quality Tooling: The Makefile

Every project needs a Makefile with these commands:

```makefile
.PHONY: install dev lint typecheck test test-int check clean

# Quality Checks
lint:
	ruff check src tests

typecheck:
	mypy src

test:
	pytest tests/unit

test-int:
	pytest tests/integration -m integration

# THE IMPORTANT ONE: Run before completing any phase
check: lint typecheck test
	@echo "All checks passed!"

# Cleanup
clean:
	rm -rf .pytest_cache/ .mypy_cache/ .ruff_cache/
	find . -type d -name __pycache__ -exec rm -rf {} +
```

**The critical command is `make check`** — it runs lint, typecheck, and tests. A phase is NOT complete until `make check` passes.

---

## pyproject.toml: Modern Python Configuration

All tooling configuration lives in `pyproject.toml`:

```toml
[project]
name = "your-project"
version = "0.1.0"
requires-python = ">=3.11"

dependencies = [
    "pydantic>=2.0",
    "httpx>=0.25",
    # ...
]

[project.optional-dependencies]
dev = [
    "pytest>=8.0",
    "pytest-asyncio>=0.23",
    "ruff>=0.4",
    "mypy>=1.8",
]

# ============================================================================
# Ruff (Linting)
# ============================================================================
[tool.ruff]
target-version = "py311"
line-length = 100

[tool.ruff.lint]
select = [
    "E", "W",     # pycodestyle
    "F",          # Pyflakes
    "I",          # isort
    "B",          # flake8-bugbear
    "UP",         # pyupgrade
]

# ============================================================================
# Mypy (Type Checking)
# ============================================================================
[tool.mypy]
python_version = "3.11"
strict = true
disallow_untyped_defs = true
warn_return_any = true

# ============================================================================
# Pytest
# ============================================================================
[tool.pytest.ini_options]
asyncio_mode = "auto"
testpaths = ["tests"]
addopts = ["-ra", "-q", "--strict-markers", "--ignore=tests/manual"]
markers = [
    "integration: marks tests as integration tests",
]
```

---

## Testing In Detail

### Unit Tests

Test individual functions and classes in isolation. Mock external dependencies.

```python
# tests/unit/calculator/test_kelly.py

def test_kelly_calculator_with_zero_edge_returns_zero():
    # Arrange
    calculator = KellyCalculator()

    # Act
    result = calculator.calculate(probability=0.5, odds=1.0)

    # Assert
    assert result.bet_fraction == 0.0

def test_kelly_calculator_with_positive_edge_returns_fraction():
    # Arrange
    calculator = KellyCalculator()

    # Act
    result = calculator.calculate(probability=0.6, odds=1.0)

    # Assert
    assert result.bet_fraction == pytest.approx(0.2)
```

**Naming pattern:** `test_<what>_<condition>_<expected>`

### Integration Tests

Test components working together. May use real databases, but mock external APIs.

```python
# tests/integration/test_pipeline_flow.py

@pytest.mark.integration
async def test_full_pipeline_produces_recommendation():
    """Test Scout → Orchestrator → Validator flow."""
    # Set up real components with test database
    db = await create_test_database()
    pipeline = TradingPipeline(db=db, paper_mode=True)

    # Run the pipeline
    result = await pipeline.process_opportunity(mock_opportunity)

    # Verify end-to-end behavior
    assert result.recommendation is not None
    assert result.recommendation.action in ["BUY_YES", "BUY_NO", "PASS"]
```

### Manual Tests

Scripts that verify real-world behavior. Run by human, require credentials.

```python
#!/usr/bin/env python3
"""Manual tests for Phase N.

Run with: python -m tests.manual.run_phaseN_tests
"""

async def test_imports() -> bool:
    """Test 1: All modules import correctly."""
    print("\nTest 1: Imports")
    print("-" * 40)
    try:
        from project.module import Class1, Class2
        print("[PASS] All imports successful")
        return True
    except Exception as e:
        print(f"[FAIL] Import error: {e}")
        return False

async def test_real_api() -> bool:
    """Test 2: Real API connectivity."""
    print("\nTest 2: API Connectivity")
    print("-" * 40)
    try:
        client = RealAPIClient()
        result = await client.ping()
        print(f"[PASS] API responded: {result}")
        return True
    except Exception as e:
        print(f"[FAIL] API error: {e}")
        return False

async def main() -> int:
    """Run all manual tests."""
    print("=" * 50)
    print("Phase N: [Name] - Manual Tests")
    print("=" * 50)

    tests = [test_imports, test_real_api]
    results = [await test() for test in tests]

    print("\n" + "=" * 50)
    passed = sum(results)
    print(f"Total: {passed}/{len(results)} passed")
    return 0 if passed == len(results) else 1

if __name__ == "__main__":
    import asyncio
    exit(asyncio.run(main()))
```

### Manual Test README

Every `tests/manual/` directory needs a README:

```markdown
# Manual Tests

## Prerequisites
1. Activate environment
2. Configure `.env` with credentials
3. For API tests: Set up sandbox keys

## Test Scripts

### run_phase1_tests.py
Tests Phase 1 deliverables.
```bash
python -m tests.manual.run_phase1_tests
```

### run_phase2_tests.py
Tests Phase 2 deliverables.
```bash
python -m tests.manual.run_phase2_tests
```

## Manual Testing Checklist
- [ ] Test 1 description
- [ ] Test 2 description
```

---

## Context Management: The 80% Rule

### Why 80%?

Phases should complete within ~80% of context:
- Leaves room for debugging
- Prevents rushed work
- Allows follow-up questions
- Reduces mid-task compaction risk

### How to Size Phases

| Deliverable Type | Estimated Context |
|-----------------|-------------------|
| Simple data model | ~2-3k |
| Class with tests | ~8-10k |
| Complex algorithm + tests | ~12-15k |
| Integration wiring | ~5-8k |
| Command + handler | ~5k |

**Good phase (~40-60k):**
- 3-5 core deliverables
- ~30-50 unit tests
- ~8-15 integration tests
- 6 manual tests

**Split if:**
- Estimated >70k
- More than 5 major files
- Multiple complex algorithms

---

## File Templates

### 1. RESUME.md (The Hub)

```markdown
# [Project Name] Resume Context

> **Read this file first when starting a fresh session.**
> It tells you where we are, what to read, and what to do next.

---

## Current State

| Field | Value |
|-------|-------|
| **Phase** | N — [Phase Name] |
| **Status** | IN PROGRESS / COMPLETE |
| **Last Updated** | [Date] |
| **Blocking Issues** | None |
| **Next Step** | [Specific next action] |

---

## Quick Context (30 seconds)

**[Project Name]** is [one-sentence description].

### Key Architecture Decisions
- [Decision 1] — [Rationale]
- [Decision 2] — [Rationale]

---

## Implementation Phases

| Phase | Name | Status | Focus |
|-------|------|--------|-------|
| 1 | [Name] | COMPLETE | [Brief] |
| 2 | [Name] | IN PROGRESS | [Brief] |
| 3 | [Name] | PENDING | [Brief] |

**Read phase details**: `.context/phases/phase-{N}.md`

---

## What To Read

### Starting a Phase (Read in Order)
1. `.context/RESUME.md` — This file
2. `docs/ARCHITECTURE.md` — System architecture
3. `.context/phases/phase-{N}.md` — Current phase details
4. `.context/CODE_STANDARDS.md` — Before writing code

---

## Phase Summaries

### Phase 1 Summary (Complete)

**Delivered:**
- [Deliverable 1]
- [Deliverable 2]

**Test coverage:** X unit + Y integration + Z manual tests

**Key files:** `src/module/`

---

## Environment Setup

```bash
# Activate environment
[your command]

# Quality checks (run before completing phases)
make check

# Tests
make test        # unit tests
make test-int    # integration tests
```

---

## Key Decisions Log

| Date | Decision | Rationale |
|------|----------|-----------|
| [Date] | [Decision] | [Why] |

---

## Session Workflow

### Starting Fresh
1. Read this file
2. Read current phase file
3. Continue from checkpoint

### Before Compaction (~70% context)
1. Update "Current State"
2. Update phase file progress
3. Commit changes
4. Ask: "Ready to compact?"

### Completing a Phase
1. `make check` passes
2. Manual tests pass
3. Update RESUME.md
4. Add "Context from Phase N" to next phase
5. Commit: `phase-N: complete`
```

---

### 2. Phase Document (Mature Pattern)

```markdown
# Phase N: [Phase Name]

**Status**: PENDING | IN PROGRESS | COMPLETE
**Goal**: [One-line goal]
**Prerequisites**: Phase N-1 complete
**Estimated Scope**: ~Xk context

---

## Context from Phase N-1

Phase N-1 delivered:
- `ComponentA` - [Purpose]
- `ComponentB` - [Purpose]

**Key files:**
- `src/module/` - Package description
- `config/settings.yaml` - Config

**Key imports:**
```python
from project.module import ClassA, ClassB
```

**Integration points:** [What this phase builds on]

---

## Problem Statement

1. **[Gap 1]**: What's missing
2. **[Gap 2]**: What's missing

---

## Deliverables

### 1. [Deliverable Name]

Create `src/module/file.py`:
- Capability 1
- Capability 2

---

## Key Files to Create/Modify

| File | Action | Purpose |
|------|--------|---------|
| `src/module/new.py` | **NEW** | Purpose |
| `src/module/existing.py` | Modify | Changes |

---

## Implementation Steps

### Step 1: [Name] (~Xk context)
1. Task
2. Task
3. Unit tests

### Step 2: [Name] (~Xk context)
1. Task
2. Integration tests
3. Manual tests

---

## Test Coverage

### Unit Tests (~X tests)
| File | Tests | Focus |
|------|-------|-------|
| `tests/unit/module/test_file.py` | X | Focus |

### Integration Tests (~X tests)
| File | Tests | Focus |
|------|-------|-------|
| `tests/integration/test_flow.py` | X | Flow |

### Manual Tests (X tests)
| Test | Description |
|------|-------------|
| `test_name` | What it verifies |

---

## Success Criteria

- [ ] Specific criterion 1
- [ ] Specific criterion 2
- [ ] `make check` passes
- [ ] X+ unit tests passing
- [ ] Y+ integration tests passing
- [ ] Manual tests documented and passing

---

## Implementation Progress

### Step 1: [Name] (COMPLETE)
- [x] Created `src/module/file.py` (~300 lines)
- [x] Created `tests/unit/module/test_file.py` (X tests)

### Step 2: [Name] (IN PROGRESS)
- [x] Done item
- [ ] Remaining item

---

## Test Summary

**Total: X tests**
- Unit: Y tests
- Integration: Z tests
- Manual: W tests

**All passing** (or note failures)

---

## Key Imports for Next LLM

```python
from project.module import (
    MainClass,
    HelperClass,
)
```
```

---

### 3. CODE_STANDARDS.md

```markdown
# Code Standards

> **Read before writing code. Quality now prevents pain later.**

---

## Environment

```bash
# Activate
[your command]

# Before completing any phase
make check
```

---

## Principles

### SOLID
- **Single Responsibility**: One thing per class/function
- **Open/Closed**: Extend, don't modify
- **Dependency Inversion**: Depend on abstractions

### DRY + KISS
- Extract on third occurrence, not before
- Simplest solution that works
- No clever tricks without documentation

---

## Naming

```python
# Bad
x = get_data()
def process(d): ...

# Good
market_prices = get_market_data()
def calculate_edge_from_signals(signals: list[Signal]) -> float: ...
```

---

## Type Hints

Required everywhere. Use modern syntax:

```python
def analyze(market: Market, context: dict[str, Any] | None = None) -> Signal:
    ...
```

---

## Testing

```python
# Pattern: test_<what>_<condition>_<expected>
def test_calculator_with_zero_edge_returns_zero():
    # Arrange
    calc = Calculator()

    # Act
    result = calc.calculate(0)

    # Assert
    assert result == 0
```

---

## Before Committing

1. `make check` passes
2. Would a senior engineer approve?
3. Are names self-documenting?
4. Did you add tests?
```

---

## Workflow Protocols

### Session Start

```
Human: "Read .context/RESUME.md, follow its instructions, gather initial context, and start coding."

Agent:
1. Reads RESUME.md
2. Notes current phase and status
3. Reads current phase file
4. Checks "Context from Phase N-1"
5. Checks "Implementation Progress"
6. Asks: "I see we're on Phase N. Continue from [checkpoint]?"
```

### Phase Completion

```
1. Verify:
   - [ ] `make check` passes
   - [ ] Manual tests pass
   - [ ] All success criteria met

2. Update phase file:
   - Implementation Progress
   - Test Summary
   - Key Imports for Next LLM

3. Update RESUME.md:
   - Phase status → COMPLETE
   - Add phase summary
   - Update Current State

4. Add to next phase:
   - "Context from Phase N" section

5. Commit: "phase-N: [brief description]"
```

### Context Preservation (~70%)

```
1. Update RESUME.md Current State
2. Update phase Implementation Progress
3. Commit changes
4. Ask: "Context at ~70%. Ready to compact?"
```

---

## Initial Setup Checklist

```
[ ] Create .context/ directory
[ ] Create .context/RESUME.md
[ ] Create .context/CODE_STANDARDS.md
[ ] Create .context/phases/phase-1.md
[ ] Create .context/components/ (if needed)
[ ] Create docs/ARCHITECTURE.md
[ ] Create Makefile with: lint, typecheck, test, check
[ ] Configure pyproject.toml with ruff + mypy + pytest
[ ] Create tests/unit/, tests/integration/, tests/manual/
[ ] Create tests/manual/README.md
[ ] Commit: "init: project lifecycle structure"
```

---

## Real Examples: Phase Sizing

From a production project (22 phases completed):

**Phase 19: Quant & Analysis (~50k)**
- 4 new plugin files
- 48 unit + 9 integration + 6 manual tests

**Phase 20: Backtesting (~60k)**
- 6 new files + 1 enhanced
- 112 unit + 12 integration + 6 manual tests

**Phase 21: Market Intelligence (~45k)**
- 5 new files
- 59 unit + 8 integration + 6 manual tests

**Phase 22: Meta & Contrarian (~35k)**
- 4 new files
- 78 unit + 8 integration + 6 manual tests

Each phase: self-contained, one session, clear deliverables, comprehensive tests.

---

## The Bottom Line

This system enables:

1. **Minimal prompting** — Just "read RESUME.md and start coding"
2. **Perfect handoffs** — Each agent picks up exactly where the last left off
3. **Quality code** — Tests, types, and standards enforced
4. **Traceable decisions** — Everything documented, nothing forgotten
5. **Sustainable pace** — 80% context rule prevents burnout

Set it up once. Benefit forever.
