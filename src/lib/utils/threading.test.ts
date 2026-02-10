import { describe, expect, it } from "vitest";

import type { Block, Role } from "$lib/types";
import { computeThreadPositions } from "./threading";

function makeBlock(id: string, role: Role, turnIndex: number): Block {
  return {
    id,
    role,
    content: id,
    tokens: 1,
    timestamp: new Date("2026-01-01T00:00:00Z"),
    zone: "recency",
    pinned: null,
    compressionLevel: "original",
    compressedVersions: { original: { content: id, tokens: 1 } },
    usageHeat: 0,
    positionRelevance: 0,
    lastReferencedTurn: turnIndex,
    referenceCount: 0,
    topicCluster: null,
    topicKeywords: [],
    metadata: {
      provider: "test",
      turnIndex,
      filePaths: [],
    },
  };
}

describe("threading utils", () => {
  it("groups user->assistant->tool_result chains with turn continuity", () => {
    const blocks = [
      makeBlock("u", "user", 5),
      makeBlock("a", "assistant", 6),
      makeBlock("tr", "tool_result", 6),
    ];
    const positions = computeThreadPositions(blocks);

    expect(positions.get("u")).toBe("first");
    expect(positions.get("a")).toBe("middle");
    expect(positions.get("tr")).toBe("last");
  });

  it("does not connect blocks when turn gap is too large", () => {
    const blocks = [
      makeBlock("u", "user", 1),
      makeBlock("a", "assistant", 4),
    ];
    const positions = computeThreadPositions(blocks);
    expect(positions.size).toBe(0);
  });

  it("breaks threads across system/thinking blocks", () => {
    const blocks = [
      makeBlock("u", "user", 1),
      makeBlock("s", "system", 2),
      makeBlock("a", "assistant", 2),
    ];
    const positions = computeThreadPositions(blocks);
    expect(positions.size).toBe(0);
  });

  it("does not connect assistant->user transitions", () => {
    const blocks = [
      makeBlock("a", "assistant", 1),
      makeBlock("u", "user", 2),
    ];
    const positions = computeThreadPositions(blocks);
    expect(positions.size).toBe(0);
  });

  it("connects tool_use/tool_result chains in the same turn", () => {
    const blocks = [
      makeBlock("a", "assistant", 7),
      makeBlock("tu", "tool_use", 7),
      makeBlock("tr", "tool_result", 7),
    ];
    const positions = computeThreadPositions(blocks);

    expect(positions.get("a")).toBe("first");
    expect(positions.get("tu")).toBe("middle");
    expect(positions.get("tr")).toBe("last");
  });
});
