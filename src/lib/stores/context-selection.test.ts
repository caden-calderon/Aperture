import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { contextStore } from "./context.svelte";
import { selectionStore } from "./selection.svelte";
import type { Block } from "../types";

function makeEngineBlock(id: string): Block {
  return {
    id,
    role: "user",
    content: `engine block ${id}`,
    tokens: 4,
    timestamp: new Date(),
    zone: "middle",
    pinned: null,
    compressionLevel: "original",
    compressedVersions: {
      original: { content: `engine block ${id}`, tokens: 4 },
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

describe("context/selection block-type semantics", () => {
  const customTypeId = "__test_custom_type__";

  beforeEach(() => {
    contextStore.setContextMutationMode("proxy_mutable");
    contextStore.loadDemoData();
    selectionStore.deselect();
  });

  afterEach(() => {
    contextStore.flushPendingWrites();
    selectionStore.deselect();
  });

  it("setBlocksType with built-in type updates canonical role and clears blockType", () => {
    const created = contextStore.createBlock(
      "middle",
      "user",
      "built-in assignment test",
      customTypeId
    );

    contextStore.setBlocksType([created.id], "assistant");
    const updated = contextStore.blocks.find((b) => b.id === created.id);

    expect(updated).toBeDefined();
    expect(updated?.role).toBe("assistant");
    expect(updated?.blockType).toBeUndefined();
  });

  it("setBlocksType with custom type preserves canonical role and sets blockType", () => {
    const created = contextStore.createBlock(
      "recency",
      "tool_result",
      "custom assignment test"
    );

    contextStore.setBlocksType([created.id], customTypeId);
    const updated = contextStore.blocks.find((b) => b.id === created.id);

    expect(updated).toBeDefined();
    expect(updated?.role).toBe("tool_result");
    expect(updated?.blockType).toBe(customTypeId);
  });

  it("selectByType matches display identity (blockType ?? role)", () => {
    const customBlock = contextStore.createBlock(
      "middle",
      "user",
      "custom select test",
      customTypeId
    );
    const builtInBlock = contextStore.createBlock(
      "middle",
      "tool_use",
      "built-in select test"
    );

    selectionStore.selectByType(customTypeId);
    expect(selectionStore.selectedIds.has(customBlock.id)).toBe(true);
    expect(selectionStore.selectedIds.has(builtInBlock.id)).toBe(false);

    selectionStore.selectByType("tool_use");
    expect(selectionStore.selectedIds.has(builtInBlock.id)).toBe(true);
  });

  it("prunes stale selection IDs when engine replaces block IDs", () => {
    const first = contextStore.blocks[0];
    expect(first).toBeDefined();
    if (!first) return;

    selectionStore.focus(first.id);
    expect(selectionStore.count).toBe(1);
    expect(selectionStore.focusedId).toBe(first.id);

    contextStore.setEngineBlocks([makeEngineBlock("replacement-1")]);

    expect(selectionStore.count).toBe(0);
    expect(selectionStore.selectedIds.has(first.id)).toBe(false);
    expect(selectionStore.focusedId).toBeNull();
  });
});
