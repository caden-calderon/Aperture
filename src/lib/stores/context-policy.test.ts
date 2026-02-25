import { beforeEach, describe, expect, it, vi } from "vitest";

const { invokeMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
}));

vi.mock("$lib/api", () => ({
  invokeProxy: invokeMock,
}));

import { contextStore } from "./context.svelte";

describe("context store engine policy gating", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    contextStore.loadDemoData();
    contextStore.setContextMutationMode("proxy_mutable");
  });

  it("does not remove a block when confirmation is required and user cancels", async () => {
    const block = contextStore.blocks[0];
    expect(block).toBeDefined();
    if (!block) return;

    invokeMock.mockResolvedValueOnce(
      JSON.stringify({ decision: "require_confirmation", reason: "protected block" })
    );
    const confirmMock = vi.fn().mockReturnValue(false);
    vi.stubGlobal("confirm", confirmMock);

    const result = await contextStore.removeBlock(block.id);

    expect(result.applied).toBe(false);
    expect(result.decision).toBe("require_confirmation");
    expect(confirmMock).toHaveBeenCalledOnce();
    expect(invokeMock).toHaveBeenCalledTimes(1);
    expect(contextStore.blocks.some((b) => b.id === block.id)).toBe(true);
  });

  it("retries remove with confirmed=true after user confirms", async () => {
    const block = contextStore.blocks[0];
    expect(block).toBeDefined();
    if (!block) return;

    invokeMock
      .mockResolvedValueOnce(
        JSON.stringify({ decision: "require_confirmation", reason: "protected block" })
      )
      .mockResolvedValueOnce(JSON.stringify({ decision: "allow" }));
    const confirmMock = vi.fn().mockReturnValue(true);
    vi.stubGlobal("confirm", confirmMock);

    const result = await contextStore.removeBlock(block.id);

    expect(result.applied).toBe(true);
    expect(confirmMock).toHaveBeenCalledOnce();
    expect(invokeMock).toHaveBeenNthCalledWith(
      2,
      "engine_remove_block",
      expect.objectContaining({ blockId: block.id, confirmed: true })
    );
    expect(contextStore.blocks.some((b) => b.id === block.id)).toBe(false);
  });

  it("uses bulk move IPC for multi-select move", async () => {
    const moveTargets = contextStore.blocks.filter((b) => b.zone !== "recency").slice(0, 2);
    expect(moveTargets.length).toBeGreaterThan(0);
    const ids = moveTargets.map((b) => b.id);

    invokeMock.mockResolvedValueOnce(JSON.stringify({ decision: "allow" }));

    const result = await contextStore.moveBlocks(ids, "recency");

    expect(result.applied).toBe(true);
    expect(invokeMock).toHaveBeenCalledWith(
      "engine_bulk_move",
      expect.objectContaining({ blockIds: ids, zone: "recency" })
    );
    for (const id of ids) {
      expect(contextStore.blocks.find((b) => b.id === id)?.zone).toBe("recency");
    }
  });

  it("does not bulk remove when engine denies", async () => {
    const ids = contextStore.blocks.slice(0, 3).map((b) => b.id);
    const beforeCount = contextStore.blocks.length;

    invokeMock.mockResolvedValueOnce(JSON.stringify({ decision: "deny", reason: "blocked" }));

    const result = await contextStore.removeBlocks(ids);

    expect(result.applied).toBe(false);
    expect(result.decision).toBe("deny");
    expect(result.reason).toBe("blocked");
    expect(contextStore.blocks.length).toBe(beforeCount);
  });

  it("skips engine IPC for no-op move within same zone", async () => {
    const block = contextStore.blocks[0];
    expect(block).toBeDefined();
    if (!block) return;

    const result = await contextStore.moveBlock(block.id, block.zone);

    expect(result.applied).toBe(true);
    expect(result.affected).toBe(0);
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("skips engine IPC for no-op content edits", async () => {
    const block = contextStore.blocks[0];
    expect(block).toBeDefined();
    if (!block) return;

    const result = await contextStore.updateBlockContent(block.id, block.content);

    expect(result.applied).toBe(true);
    expect(result.affected).toBe(0);
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("accepts empty engine snapshots and clears working blocks", () => {
    expect(contextStore.blocks.length).toBeGreaterThan(0);

    contextStore.setEngineBlocks([]);

    expect(contextStore.blocks.length).toBe(0);
  });

  it("keeps edits mutable under unified proxy mode", async () => {
    const block = contextStore.blocks[0];
    expect(block).toBeDefined();
    if (!block) return;

    contextStore.setContextMutationMode("proxy_mutable");
    invokeMock
      .mockResolvedValueOnce(JSON.stringify({ decision: "allow" }))
      .mockResolvedValueOnce(undefined);
    const updatedContent = `${block.content} updated`;
    const result = await contextStore.updateBlockContent(block.id, updatedContent);

    expect(result.applied).toBe(true);
    expect(invokeMock).toHaveBeenCalledWith(
      "queue_hot_patch",
      expect.objectContaining({
        role: block.role,
        originalContent: block.content,
        newContent: updatedContent,
      })
    );
    expect(contextStore.blocks.find((candidate) => candidate.id === block.id)?.content).toBe(
      updatedContent
    );
  });

  it("routes content edits through hot-patch queue in proxy mode", async () => {
    const block = contextStore.blocks[0];
    expect(block).toBeDefined();
    if (!block) return;

    contextStore.setContextMutationMode("proxy_mutable");
    invokeMock
      .mockResolvedValueOnce(JSON.stringify({ decision: "allow" }))
      .mockResolvedValueOnce(undefined);

    const result = await contextStore.updateBlockContent(block.id, `${block.content} updated`);

    expect(result.applied).toBe(true);
    expect(invokeMock).toHaveBeenNthCalledWith(
      1,
      "engine_update_content",
      expect.objectContaining({ blockId: block.id })
    );
    expect(invokeMock).toHaveBeenCalledWith(
      "queue_hot_patch",
      expect.objectContaining({
        role: block.role,
        originalContent: block.content,
        newContent: `${block.content} updated`,
      })
    );
  });

  it("does not clear engine context when confirmation is required and user cancels", async () => {
    const beforeCount = contextStore.blocks.length;
    expect(beforeCount).toBeGreaterThan(0);

    invokeMock.mockResolvedValueOnce(
      JSON.stringify({ decision: "require_confirmation", reason: "destructive action" })
    );
    const confirmMock = vi.fn().mockReturnValue(false);
    vi.stubGlobal("confirm", confirmMock);

    const result = await contextStore.clearEngineContext();

    expect(result.applied).toBe(false);
    expect(result.decision).toBe("require_confirmation");
    expect(contextStore.blocks.length).toBe(beforeCount);
    expect(confirmMock).toHaveBeenCalledOnce();
  });

  it("clears engine context after confirmed policy approval", async () => {
    expect(contextStore.blocks.length).toBeGreaterThan(0);

    invokeMock
      .mockResolvedValueOnce(
        JSON.stringify({ decision: "require_confirmation", reason: "destructive action" })
      )
      .mockResolvedValueOnce(JSON.stringify({ decision: "allow" }));
    const confirmMock = vi.fn().mockReturnValue(true);
    vi.stubGlobal("confirm", confirmMock);

    const result = await contextStore.clearEngineContext();

    expect(result.applied).toBe(true);
    expect(contextStore.blocks.length).toBe(0);
    expect(invokeMock).toHaveBeenNthCalledWith(
      2,
      "engine_clear_context",
      expect.objectContaining({ confirmed: true })
    );
  });
});
