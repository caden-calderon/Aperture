/**
 * Context Store
 * Manages blocks, zones, and token budget state
 *
 * Uses Svelte 5 runes for reactive state management
 * Persists blocks and snapshots to localStorage
 */

import type { Block, Zone, TokenBudget, Snapshot, SnapshotZoneState, Role } from "../types";
import { isRole } from "../types";
import {
  generateDemoBlocks,
  generateDemoSnapshots,
  calculateTokenBudget,
} from "../mock-data";
import { editHistoryStore } from "./editHistory.svelte";
import { zonesStore } from "./zones.svelte";
import { invoke } from "@tauri-apps/api/core";

// ============================================================================
// State
// ============================================================================

let blocks = $state<Block[]>([]);
let snapshots = $state<Snapshot[]>([]);
const tokenLimit = 200000;

// Branching state: null = working state, string = viewing a snapshot
let activeSnapshotId = $state<string | null>(null);
let workingStateCache = $state<{ blocks: Block[]; zoneState: SnapshotZoneState } | null>(null);

// ============================================================================
// Persistence
// ============================================================================

const STORAGE_KEY = "aperture-context";

// Debounced persistence
let _dirty = false;
let _saveTimer: ReturnType<typeof setTimeout> | undefined;

function markDirty(): void {
  _dirty = true;
  if (_saveTimer) clearTimeout(_saveTimer);
  _saveTimer = setTimeout(() => {
    saveToLocalStorage();
    _dirty = false;
  }, 1500);
}

function flushPendingWrites(): void {
  if (_dirty) {
    saveToLocalStorage();
    _dirty = false;
  }
  if (_saveTimer) {
    clearTimeout(_saveTimer);
    _saveTimer = undefined;
  }
}

function saveToLocalStorage(): void {
  if (typeof localStorage === "undefined") return;
  try {
    localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({ blocks, snapshots, activeSnapshotId, workingStateCache })
    );
  } catch (e) {
    console.error("Failed to save context:", e);
  }
}

function loadFromLocalStorage(): boolean {
  if (typeof localStorage === "undefined") return false;
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (!stored) return false;

    const data = JSON.parse(stored);
    const storedBlocks: Block[] = Array.isArray(data.blocks) ? data.blocks : [];

    // Restore Date objects (JSON serializes them as strings)
    blocks = storedBlocks.map((b: Block) => ({
      ...b,
      timestamp: new Date(b.timestamp),
    }));
    // Migrate snapshots: add zoneState/parentSnapshotId if missing
    snapshots = (data.snapshots ?? []).map((s: Snapshot) => ({
      ...s,
      timestamp: new Date(s.timestamp),
      blocks: s.blocks.map((b: Block) => ({
        ...b,
        timestamp: new Date(b.timestamp),
      })),
      zoneState: s.zoneState ?? null,
      parentSnapshotId: s.parentSnapshotId ?? null,
    }));

    // Restore branching state (defaults for migration)
    activeSnapshotId = data.activeSnapshotId ?? null;
    workingStateCache = data.workingStateCache ?? null;

    return blocks.length > 0 || snapshots.length > 0;
  } catch (e) {
    console.error("Failed to load context:", e);
    return false;
  }
}

// ============================================================================
// Derived State
// ============================================================================

// Sort blocks within zone: pinned top first, then unpinned, then pinned bottom
function sortBlocksWithPins(zoneBlocks: Block[]): Block[] {
  const pinnedTop = zoneBlocks.filter((b) => b.pinned === "top");
  const pinnedBottom = zoneBlocks.filter((b) => b.pinned === "bottom");
  const unpinned = zoneBlocks.filter((b) => b.pinned === null);
  return [...pinnedTop, ...unpinned, ...pinnedBottom];
}

// Dynamic blocks by zone - supports custom zones
const blocksByZone = $derived.by(() => {
  const result: Record<string, Block[]> = {};
  // Group blocks by zone and sort with pins
  const grouped = new Map<string, Block[]>();
  for (const block of blocks) {
    const zoneBlocks = grouped.get(block.zone) ?? [];
    zoneBlocks.push(block);
    grouped.set(block.zone, zoneBlocks);
  }
  // Sort each zone's blocks with pins
  for (const [zoneId, zoneBlocks] of grouped) {
    result[zoneId] = sortBlocksWithPins(zoneBlocks);
  }
  // Ensure built-in zones exist even if empty
  if (!result.primacy) result.primacy = [];
  if (!result.middle) result.middle = [];
  if (!result.recency) result.recency = [];
  return result;
});

const tokenBudget = $derived<TokenBudget>(calculateTokenBudget(blocks));

const blockMap = $derived(new Map(blocks.map((b) => [b.id, b])));

const activeStateLabel = $derived.by(() => {
  if (!activeSnapshotId) return "Working State";
  const snap = snapshots.find(s => s.id === activeSnapshotId);
  return snap?.name ?? "Unknown Snapshot";
});

// ============================================================================
// Actions
// ============================================================================

function init(): void {
  loadFromLocalStorage();
  // Live proxy context is session-scoped: start empty on app launch.
  blocks = [];
  activeSnapshotId = null;
  workingStateCache = null;
  saveToLocalStorage();
  // No demo data auto-load — start empty, blocks arrive via proxy capture.
  // Call loadDemoData() explicitly from the empty state UI if needed.
}

function loadDemoData(): void {
  blocks = generateDemoBlocks();
  snapshots = generateDemoSnapshots(blocks);
  saveToLocalStorage();
}

function getBlock(id: string): Block | undefined {
  return blockMap.get(id);
}

function getBlockIndex(id: string): number {
  return blocks.findIndex((b) => b.id === id);
}

function moveBlock(blockId: string, targetZone: Zone): void {
  const index = getBlockIndex(blockId);
  if (index === -1) return;

  const oldZone = blocks[index].zone;
  if (oldZone !== targetZone) {
    editHistoryStore.recordEdit(blockId, "zone", { zone: oldZone }, { zone: targetZone });
  }

  blocks[index] = { ...blocks[index], zone: targetZone };
  blocks = [...blocks];
  markDirty();
}

function moveBlocks(blockIds: string[], targetZone: Zone): void {
  const idSet = new Set(blockIds);
  blocks = blocks.map((b) => (idSet.has(b.id) ? { ...b, zone: targetZone } : b));
  markDirty();
}

function removeBlock(blockId: string): void {
  editHistoryStore.clearBlockHistory(blockId);
  blocks = blocks.filter((b) => b.id !== blockId);
  markDirty();
}

function removeBlocks(blockIds: string[]): void {
  const idSet = new Set(blockIds);
  for (const id of idSet) {
    editHistoryStore.clearBlockHistory(id);
  }
  blocks = blocks.filter((b) => !idSet.has(b.id));
  markDirty();
}

function createBlock(
  zone: Zone,
  role: Role,
  content = "",
  blockType?: string
): Block {
  const defaultContent = content || `New ${role} block`;
  const newBlock: Block = {
    id: `block-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
    role,
    blockType,
    content: defaultContent,
    tokens: Math.ceil(defaultContent.length / 4),
    timestamp: new Date(),
    zone,
    pinned: null,
    compressionLevel: "original",
    compressedVersions: {
      original: { content: defaultContent, tokens: Math.ceil(defaultContent.length / 4) },
    },
    usageHeat: 0.5,
    positionRelevance: 0.5,
    lastReferencedTurn: 0,
    referenceCount: 0,
    topicCluster: null,
    topicKeywords: [],
    metadata: { provider: "manual", turnIndex: blocks.length, filePaths: [] },
  };
  blocks = [...blocks, newBlock];
  markDirty();
  return newBlock;
}

function updateBlockContent(blockId: string, content: string): void {
  const index = getBlockIndex(blockId);
  if (index === -1) return;

  const block = blocks[index];
  const oldContent = block.content;
  if (oldContent !== content) {
    editHistoryStore.recordEdit(blockId, "content", { content: oldContent }, { content });

    // Queue hot patch so the edit takes effect on the next outbound request
    invoke("queue_hot_patch", {
      role: block.role,
      originalContent: oldContent,
      newContent: content,
    }).catch(() => {
      // Ignore errors when running outside Tauri (e.g., browser dev)
    });
  }

  const tokens = Math.ceil(content.length / 4);
  blocks[index] = {
    ...blocks[index],
    content,
    tokens,
    compressedVersions: {
      ...blocks[index].compressedVersions,
      original: { content, tokens },
    },
  };
  blocks = [...blocks];
  markDirty();
}

function setBlockRole(blockId: string, role: Role, blockType?: string): void {
  const index = getBlockIndex(blockId);
  if (index === -1) return;

  const oldRole = blocks[index].role;
  const oldBlockType = blocks[index].blockType ?? null;
  if (oldRole !== role || oldBlockType !== (blockType ?? null)) {
    editHistoryStore.recordEdit(
      blockId, "role",
      { role: oldRole, blockType: oldBlockType },
      { role, blockType: blockType ?? null }
    );
  }

  blocks[index] = { ...blocks[index], role, blockType };
  blocks = [...blocks];
  markDirty();
}

function setBlocksRole(blockIds: string[], role: Role, blockType?: string): void {
  const idSet = new Set(blockIds);
  const nextBlockType = blockType ?? undefined;
  let changed = false;

  blocks = blocks.map((b) => {
    if (!idSet.has(b.id)) return b;

    const oldBlockType = b.blockType ?? null;
    const newBlockType = nextBlockType ?? null;
    if (b.role === role && oldBlockType === newBlockType) return b;

    editHistoryStore.recordEdit(
      b.id,
      "role",
      { role: b.role, blockType: oldBlockType },
      { role, blockType: newBlockType }
    );

    changed = true;
    return { ...b, role, blockType: nextBlockType };
  });

  if (!changed) return;
  blocks = [...blocks];
  markDirty();
}

function setBlocksType(blockIds: string[], typeId: string): void {
  if (isRole(typeId)) {
    setBlocksRole(blockIds, typeId);
    return;
  }

  const idSet = new Set(blockIds);
  let changed = false;

  blocks = blocks.map((b) => {
    if (!idSet.has(b.id)) return b;

    const oldBlockType = b.blockType ?? null;
    if (oldBlockType === typeId) return b;

    editHistoryStore.recordEdit(
      b.id,
      "role",
      { role: b.role, blockType: oldBlockType },
      { role: b.role, blockType: typeId }
    );

    changed = true;
    return { ...b, blockType: typeId };
  });

  if (!changed) return;
  blocks = [...blocks];
  markDirty();
}

function reorderBlock(blockId: string, newIndex: number): void {
  const currentIndex = getBlockIndex(blockId);
  if (currentIndex === -1) return;

  const block = blocks[currentIndex];
  const newBlocks = [...blocks];
  newBlocks.splice(currentIndex, 1);
  newBlocks.splice(newIndex, 0, block);
  blocks = newBlocks;
  markDirty();
}

function reorderBlocksInZone(
  zone: Zone,
  blockIds: string[],
  insertIndex: number
): void {
  const zoneBlocks = sortBlocksWithPins(blocks.filter((b) => b.zone === zone));
  const otherBlocks = blocks.filter((b) => b.zone !== zone);
  const idSet = new Set(blockIds);

  const movingPinnedBlocks = zoneBlocks.filter(
    (b) => idSet.has(b.id) && b.pinned !== null
  );
  if (movingPinnedBlocks.length > 0) {
    return;
  }

  const movingBlocks = zoneBlocks.filter((b) => idSet.has(b.id));
  const stayingBlocks = zoneBlocks.filter((b) => !idSet.has(b.id));

  const pinnedTopCount = stayingBlocks.filter((b) => b.pinned === "top").length;
  const pinnedBottomCount = stayingBlocks.filter((b) => b.pinned === "bottom").length;
  const minIndex = pinnedTopCount;
  const maxIndex = stayingBlocks.length - pinnedBottomCount;

  const clampedIndex = Math.max(minIndex, Math.min(insertIndex, maxIndex));
  stayingBlocks.splice(clampedIndex, 0, ...movingBlocks);

  blocks = [...otherBlocks, ...stayingBlocks];
  markDirty();
}

function setCompressionLevel(
  blockId: string,
  level: Block["compressionLevel"]
): void {
  const index = getBlockIndex(blockId);
  if (index === -1) return;

  const oldLevel = blocks[index].compressionLevel;
  if (oldLevel !== level) {
    editHistoryStore.recordEdit(
      blockId, "compression",
      { compressionLevel: oldLevel },
      { compressionLevel: level }
    );
  }

  blocks[index] = { ...blocks[index], compressionLevel: level };
  blocks = [...blocks];
  markDirty();
}

function pinBlock(blockId: string, position: Block["pinned"]): void {
  const index = getBlockIndex(blockId);
  if (index === -1) return;

  const oldPin = blocks[index].pinned;
  if (oldPin !== position) {
    editHistoryStore.recordEdit(
      blockId, "pin",
      { pinned: oldPin ?? null },
      { pinned: position ?? null }
    );
  }

  blocks[index] = { ...blocks[index], pinned: position };
  blocks = [...blocks];
  markDirty();
}

function getValidDropRange(zone: Zone): { min: number; max: number } {
  const zoneBlocks = sortBlocksWithPins(blocks.filter((b) => b.zone === zone));
  const pinnedTopCount = zoneBlocks.filter((b) => b.pinned === "top").length;
  const pinnedBottomCount = zoneBlocks.filter((b) => b.pinned === "bottom").length;
  return {
    min: pinnedTopCount,
    max: zoneBlocks.length - pinnedBottomCount,
  };
}

/** Add blocks received from the live proxy (replaces current context). */
function setLiveBlocks(requestBlocks: Block[], responseBlocks: Block[]): void {
  // Replace blocks with the full conversation from the latest request
  // (request_blocks contain the entire conversation history sent to the API)
  const allBlocks = [...requestBlocks, ...responseBlocks];
  if (allBlocks.length === 0) return;
  if (isLikelyCliStartupNoise(requestBlocks, responseBlocks)) return;

  blocks = allBlocks;
  markDirty();
}

/**
 * Ignore known startup probe noise (e.g., tiny single-word user payloads like
 * "count"/"foo" with no model response) so they don't clobber real context.
 */
function isLikelyCliStartupNoise(requestBlocks: Block[], responseBlocks: Block[]): boolean {
  if (responseBlocks.length > 0) return false;
  if (requestBlocks.length === 0 || requestBlocks.length > 2) return false;

  return requestBlocks.every((block) => {
    if (block.role !== "user") return false;
    if (block.tokens > 2) return false;

    const text = block.content.trim();
    if (text.length === 0 || text.length > 12) return false;
    return /^[a-z0-9_.-]+$/i.test(text);
  });
}

/** Clear all live blocks (used on session reset). */
function clearBlocks(): void {
  blocks = [];
  markDirty();
}

/** Append new response blocks to existing context (incremental). */
function appendResponseBlocks(newBlocks: Block[]): void {
  if (newBlocks.length === 0) return;
  blocks = [...blocks, ...newBlocks];
  markDirty();
}

function updateBlockHeat(blockId: string, heat: number): void {
  const index = getBlockIndex(blockId);
  if (index === -1) return;

  blocks[index] = {
    ...blocks[index],
    usageHeat: Math.max(0, Math.min(1, heat)),
  };
  blocks = [...blocks];
  markDirty();
}

function saveSnapshot(name: string): Snapshot {
  const snapshot: Snapshot = {
    id: `snap-${Date.now()}`,
    name,
    timestamp: new Date(),
    blocks: $state.snapshot(blocks),
    totalTokens: tokenBudget.used,
    type: "soft",
    zoneState: zonesStore.captureState(),
    parentSnapshotId: activeSnapshotId,
  };

  snapshots = [...snapshots, snapshot];
  markDirty();
  return snapshot;
}

function restoreSnapshot(snapshotId: string): boolean {
  return switchToSnapshot(snapshotId);
}

function deleteSnapshot(snapshotId: string): void {
  // If deleting the active snapshot, switch to working state first
  if (activeSnapshotId === snapshotId) {
    switchToWorkingState();
  }
  snapshots = snapshots.filter((s) => s.id !== snapshotId);
  markDirty();
}

function switchToSnapshot(snapshotId: string): boolean {
  const target = snapshots.find(s => s.id === snapshotId);
  if (!target) return false;

  // Save current state back
  if (activeSnapshotId === null) {
    // Save working state to cache
    workingStateCache = {
      blocks: $state.snapshot(blocks),
      zoneState: zonesStore.captureState(),
    };
  } else {
    // Save current blocks back to the active snapshot
    const activeIdx = snapshots.findIndex(s => s.id === activeSnapshotId);
    if (activeIdx !== -1) {
      snapshots[activeIdx] = {
        ...snapshots[activeIdx],
        blocks: $state.snapshot(blocks),
        zoneState: zonesStore.captureState(),
        totalTokens: tokenBudget.used,
      };
      snapshots = [...snapshots];
    }
  }

  // Load target snapshot
  blocks = $state.snapshot(target.blocks);
  if (target.zoneState) {
    zonesStore.restoreState(target.zoneState);
  }
  activeSnapshotId = snapshotId;
  markDirty();
  return true;
}

function switchToWorkingState(): void {
  if (activeSnapshotId === null) return;

  // Save current blocks back to active snapshot
  const activeIdx = snapshots.findIndex(s => s.id === activeSnapshotId);
  if (activeIdx !== -1) {
    snapshots[activeIdx] = {
      ...snapshots[activeIdx],
      blocks: $state.snapshot(blocks),
      zoneState: zonesStore.captureState(),
      totalTokens: tokenBudget.used,
    };
    snapshots = [...snapshots];
  }

  // Restore working state from cache
  if (workingStateCache) {
    blocks = $state.snapshot(workingStateCache.blocks);
    if (workingStateCache.zoneState) {
      zonesStore.restoreState(workingStateCache.zoneState);
    }
    workingStateCache = null;
  }

  activeSnapshotId = null;
  markDirty();
}

function renameSnapshot(snapshotId: string, name: string): void {
  const idx = snapshots.findIndex(s => s.id === snapshotId);
  if (idx === -1) return;
  snapshots[idx] = { ...snapshots[idx], name };
  snapshots = [...snapshots];
  markDirty();
}

// ============================================================================
// Export Store Interface
// ============================================================================

export const contextStore = {
  // Reactive getters
  get blocks() {
    return blocks;
  },
  get blocksByZone() {
    return blocksByZone;
  },
  get tokenBudget() {
    return tokenBudget;
  },
  get tokenLimit() {
    return tokenLimit;
  },
  get snapshots() {
    return snapshots;
  },
  get activeSnapshotId() {
    return activeSnapshotId;
  },
  get activeStateLabel() {
    return activeStateLabel;
  },

  // Actions
  init,
  flushPendingWrites,
  loadDemoData,
  setLiveBlocks,
  clearBlocks,
  appendResponseBlocks,
  getBlock,
  getBlockIndex,
  moveBlock,
  moveBlocks,
  removeBlock,
  removeBlocks,
  createBlock,
  updateBlockContent,
  setBlockRole,
  setBlocksRole,
  setBlocksType,
  reorderBlock,
  reorderBlocksInZone,
  setCompressionLevel,
  pinBlock,
  getValidDropRange,
  updateBlockHeat,
  saveSnapshot,
  restoreSnapshot,
  deleteSnapshot,
  switchToSnapshot,
  switchToWorkingState,
  renameSnapshot,
};
