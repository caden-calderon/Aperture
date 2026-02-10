import { beforeEach, describe, expect, it } from "vitest";

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
  beforeEach(() => {
    contextStore.setContextMutationMode("proxy_mutable");
  });

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

  it("computes staleness scores so older blocks are staler", () => {
    contextStore.switchToWorkingState();
    contextStore.clearBlocks();

    const old = contextStore.createBlock("middle", "user", "same-sized-content");
    for (let i = 0; i < 8; i += 1) {
      contextStore.createBlock("middle", "assistant", `filler-${i}`);
    }
    const recent = contextStore.createBlock("middle", "assistant", "same-sized-content");

    const oldScore = contextStore.getBlockStaleness(old.id);
    const recentScore = contextStore.getBlockStaleness(recent.id);
    expect(oldScore).toBeDefined();
    expect(recentScore).toBeDefined();
    expect((oldScore ?? 0) > (recentScore ?? 0)).toBe(true);
  });

  it("reapplies edited content when engine refresh sends original content", async () => {
    contextStore.switchToWorkingState();
    contextStore.clearBlocks();
    contextStore.setActiveEngineSession("session-a");

    const initial = makeBlock(
      "u-1",
      "user",
      "Remember this number 52",
      6
    );
    contextStore.setEngineBlocks([initial]);
    expect(contextStore.blocks.length).toBe(1);

    const edited = await contextStore.updateBlockContent(initial.id, "Remember this number 73");
    expect(edited.applied).toBe(true);
    expect(contextStore.blocks[0].content).toContain("73");

    // Simulate next engine ingest with fresh IDs but original upstream content.
    const refreshedOriginal = makeBlock(
      "u-2",
      "user",
      "Remember this number 52",
      6
    );
    contextStore.setEngineBlocks([refreshedOriginal]);

    expect(contextStore.blocks.length).toBe(1);
    expect(contextStore.blocks[0].id).toBe("u-2");
    expect(contextStore.blocks[0].content).toContain("73");
    expect(contextStore.blocks[0].content).not.toContain("52");
  });

  it("scopes local edited overlays to the active engine session", async () => {
    contextStore.switchToWorkingState();
    contextStore.clearBlocks();

    contextStore.setActiveEngineSession("session-a");
    const sessionABlock = makeBlock("a-1", "user", "Remember this number 52", 6);
    contextStore.setEngineBlocks([sessionABlock]);
    const edited = await contextStore.updateBlockContent(sessionABlock.id, "Remember this number 73");
    expect(edited.applied).toBe(true);
    expect(contextStore.blocks[0].content).toContain("73");

    contextStore.setActiveEngineSession("session-b");
    const sessionBBlock = makeBlock("b-1", "user", "Remember this number 52", 6);
    contextStore.setEngineBlocks([sessionBBlock]);
    expect(contextStore.blocks[0].content).toContain("52");
    expect(contextStore.blocks[0].content).not.toContain("73");

    contextStore.setActiveEngineSession("session-a");
    const sessionARefresh = makeBlock("a-2", "user", "Remember this number 52", 6);
    contextStore.setEngineBlocks([sessionARefresh]);
    expect(contextStore.blocks[0].content).toContain("73");
  });
});
