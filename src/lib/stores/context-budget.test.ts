/**
 * Frontend tests for Phase 3 Checkpoint F:
 * Budget ceiling, settings integration, block mutation notifications.
 */

import { beforeEach, describe, expect, it, vi } from "vitest";
import { contextStore } from "./context.svelte";
import { uiStore } from "./ui.svelte";
import type { Block } from "../types";

function makeBlock(
  id: string,
  role: Block["role"],
  content: string,
  tokens: number,
  compressionLevel: Block["compressionLevel"] = "original",
): Block {
  return {
    id,
    role,
    content,
    tokens,
    timestamp: new Date(),
    zone: role === "system" ? "primacy" : "middle",
    pinned: null,
    compressionLevel,
    compressedVersions: {
      original: { content, tokens },
    },
    usageHeat: 0,
    positionRelevance: 0,
    lastReferencedTurn: 0,
    referenceCount: 0,
    topicCluster: null,
    topicKeywords: [],
    metadata: {
      provider: "test",
      turnIndex: 0,
      filePaths: [],
    },
  };
}

describe("budget ceiling", () => {
  beforeEach(() => {
    contextStore.setContextMutationMode("proxy_mutable");
  });

  it("defaults to 0.80", () => {
    // Budget ceiling should be 0.80 by default (module-level default)
    expect(contextStore.budgetCeiling).toBeCloseTo(0.80, 2);
  });

  it("setBudgetCeiling clamps to valid range", async () => {
    // Below minimum
    await contextStore.setBudgetCeiling(0.1);
    expect(contextStore.budgetCeiling).toBeCloseTo(0.4, 2);

    // Above maximum
    await contextStore.setBudgetCeiling(2.0);
    expect(contextStore.budgetCeiling).toBeCloseTo(1.0, 2);

    // Normal value
    await contextStore.setBudgetCeiling(0.65);
    expect(contextStore.budgetCeiling).toBeCloseTo(0.65, 2);

    // Restore default
    await contextStore.setBudgetCeiling(0.80);
  });
});

describe("token budget calculation", () => {
  beforeEach(() => {
    contextStore.setContextMutationMode("proxy_mutable");
    contextStore.switchToWorkingState();
    contextStore.clearBlocks();
    contextStore.setActiveEngineSession("budget-test-session");
  });

  it("blocks reflect correct token sum after setEngineBlocks", () => {
    contextStore.setEngineBlocks([
      makeBlock("s1", "system", "System prompt", 50),
      makeBlock("u1", "user", "Hello", 10),
      makeBlock("a1", "assistant", "Hi there", 15),
    ]);

    expect(contextStore.blocks.length).toBe(3);
    const totalTokens = contextStore.blocks.reduce((sum, b) => sum + b.tokens, 0);
    expect(totalTokens).toBe(75);
  });

  it("tokenBudget has a positive limit", () => {
    const budget = contextStore.tokenBudget;
    expect(budget.limit).toBeGreaterThan(0);
  });
});

describe("block mutation notifications", () => {
  beforeEach(() => {
    contextStore.setContextMutationMode("proxy_mutable");
    contextStore.switchToWorkingState();
    contextStore.clearBlocks();
    contextStore.setActiveEngineSession("mutation-test-session");
  });

  it("detects archived blocks and surfaces toast", () => {
    // Set up initial blocks
    const initialBlocks = [
      makeBlock("b1", "user", "First message", 10),
      makeBlock("b2", "assistant", "Response", 15),
      makeBlock("b3", "user", "Second message", 10),
    ];
    contextStore.setEngineBlocks(initialBlocks);
    expect(contextStore.blocks.length).toBe(3);

    // Spy on toast before updating
    const toastSpy = vi.spyOn(uiStore, "showToast");

    // Remove b2 (simulating archival)
    const updatedBlocks = [
      makeBlock("b1", "user", "First message", 10),
      makeBlock("b3", "user", "Second message", 10),
    ];
    contextStore.setEngineBlocks(updatedBlocks);

    expect(contextStore.blocks.length).toBe(2);
    // Should have triggered a toast about the archival
    expect(toastSpy).toHaveBeenCalled();
    const toastArgs = toastSpy.mock.calls[0];
    expect(toastArgs[0]).toContain("archived");
    expect(toastArgs[1]).toBe("info");

    toastSpy.mockRestore();
  });

  it("detects compressed blocks and surfaces toast", () => {
    // Set up initial blocks
    contextStore.setEngineBlocks([
      makeBlock("b1", "user", "Message", 10, "original"),
    ]);

    const toastSpy = vi.spyOn(uiStore, "showToast");

    // Update with compression applied
    contextStore.setEngineBlocks([
      makeBlock("b1", "user", "Message (summarized)", 5, "summarized"),
    ]);

    expect(toastSpy).toHaveBeenCalled();
    const toastArgs = toastSpy.mock.calls[0];
    expect(toastArgs[0]).toContain("summarized");

    toastSpy.mockRestore();
  });

  it("no toast on initial load (empty to blocks)", () => {
    const toastSpy = vi.spyOn(uiStore, "showToast");

    contextStore.setEngineBlocks([
      makeBlock("b1", "user", "First message", 10),
    ]);

    // Initial load should NOT trigger mutation notification
    expect(toastSpy).not.toHaveBeenCalled();

    toastSpy.mockRestore();
  });

  it("treats session replacement as archival when block IDs are unrelated (current behavior)", () => {
    contextStore.setActiveEngineSession("session-a");
    contextStore.setEngineBlocks([
      makeBlock("a1", "user", "Session A message", 10),
      makeBlock("a2", "assistant", "Session A reply", 12),
    ]);

    const toastSpy = vi.spyOn(uiStore, "showToast");

    // Simulate backend active-session flip: new session payload has unrelated IDs.
    contextStore.setActiveEngineSession("session-b");
    contextStore.setEngineBlocks([
      makeBlock("b1", "user", "Session B message", 11),
    ]);

    expect(toastSpy).toHaveBeenCalled();
    expect(toastSpy.mock.calls[0][0]).toContain("archived");
    expect(toastSpy.mock.calls[0][1]).toBe("info");

    toastSpy.mockRestore();
  });
});
