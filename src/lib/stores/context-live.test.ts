import { describe, expect, it } from "vitest";

import { contextStore } from "./context.svelte";
import type { Block } from "../types";

function makeBlock(id: string, role: Block["role"], content: string, tokens: number): Block {
  return {
    id,
    role,
    content,
    tokens,
    timestamp: new Date(),
    zone: role === "system" ? "primacy" : "recency",
    pinned: null,
    compressionLevel: "original",
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

describe("context live ingest", () => {
  it("ignores tiny startup probe captures with no response", () => {
    contextStore.loadDemoData();
    const initialCount = contextStore.blocks.length;

    contextStore.setLiveBlocks(
      [makeBlock("probe-1", "user", "count", 2)],
      []
    );

    expect(contextStore.blocks.length).toBe(initialCount);
  });

  it("accepts normal request/response captures", () => {
    contextStore.clearBlocks();

    contextStore.setLiveBlocks(
      [makeBlock("req-1", "user", "Write a short joke about Rust", 8)],
      [makeBlock("res-1", "assistant", "Fearless concurrency walked into a bar.", 10)]
    );

    expect(contextStore.blocks.length).toBe(2);
    expect(contextStore.blocks[0].content).toContain("Write a short joke");
    expect(contextStore.blocks[1].role).toBe("assistant");
  });
});
