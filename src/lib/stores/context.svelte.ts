/**
 * Context Store
 * Manages blocks, zones, and token budget state
 *
 * Uses Svelte 5 runes for reactive state management
 * Persists blocks and snapshots to localStorage
 */

import type {
  Block,
  Zone,
  TokenBudget,
  Snapshot,
  SnapshotZoneState,
  Role,
  CompressionSettings,
} from "../types";
import { isRole } from "../types";
import {
  generateDemoBlocks,
  generateDemoSnapshots,
  calculateTokenBudget,
} from "../mock-data";
import { editHistoryStore } from "./editHistory.svelte";
import { zonesStore } from "./zones.svelte";
import { uiStore } from "./ui.svelte";
import { invokeProxy } from "../api";
import {
  type ContextMutationMode,
} from "../utils/providerAdapters";

// ============================================================================
// State
// ============================================================================

let blocks = $state<Block[]>([]);
let snapshots = $state<Snapshot[]>([]);
let engineBudgetLimit = $state<number | null>(null);
let activeEngineSessionId = $state<string | null>(null);
let _activeEngineThreadIdentity = $state<string | null>(null);
let contextMutationMode = $state<ContextMutationMode>("proxy_mutable");
let budgetCeiling = $state<number>(loadBudgetCeiling());
let compressionSettings = $state<CompressionSettings>(defaultCompressionSettings());
let localHotPatchesBySession = $state<
  Record<string, Array<{ role: Role; originalContent: string; newContent: string }>>
>({});

// Branching state: null = working state, string = viewing a snapshot
let activeSnapshotId = $state<string | null>(null);
let workingStateCache = $state<{ blocks: Block[]; zoneState: SnapshotZoneState } | null>(null);

// ============================================================================
// Persistence
// ============================================================================

const STORAGE_KEY = "aperture-context";
const DEFAULT_PATCH_SESSION_KEY = "__default__";
const BUDGET_CEILING_KEY = "aperture:budget-ceiling";

function loadBudgetCeiling(): number {
  try {
    const stored = localStorage.getItem(BUDGET_CEILING_KEY);
    if (stored) {
      const val = parseFloat(stored);
      if (!isNaN(val) && val >= 0.4 && val <= 1.0) return val;
    }
  } catch { /* ignore */ }
  return 0.80;
}

function defaultCompressionSettings(): CompressionSettings {
  return {
    backend: "auto",
    model: null,
    timeoutMs: 12_000,
    maxTokens: 512,
    openrouterBaseUrl: null,
    openrouterApiKeyEnv: "OPENROUTER_API_KEY",
  };
}

function normalizeCompressionSettings(
  value: Partial<CompressionSettings> | null | undefined
): CompressionSettings {
  const fallback = defaultCompressionSettings();
  const backend = value?.backend;
  const validBackend =
    backend === "auto" ||
    backend === "disabled" ||
    backend === "anthropic" ||
    backend === "open_ai" ||
    backend === "open_router";

  return {
    backend: validBackend ? backend : fallback.backend,
    model: typeof value?.model === "string" ? value.model.trim() || null : fallback.model,
    timeoutMs:
      typeof value?.timeoutMs === "number" && Number.isFinite(value.timeoutMs)
        ? Math.max(500, Math.min(120000, Math.round(value.timeoutMs)))
        : fallback.timeoutMs,
    maxTokens:
      typeof value?.maxTokens === "number" && Number.isFinite(value.maxTokens)
        ? Math.max(64, Math.min(8192, Math.round(value.maxTokens)))
        : fallback.maxTokens,
    openrouterBaseUrl:
      typeof value?.openrouterBaseUrl === "string"
        ? value.openrouterBaseUrl.trim() || null
        : fallback.openrouterBaseUrl,
    openrouterApiKeyEnv:
      typeof value?.openrouterApiKeyEnv === "string"
        ? value.openrouterApiKeyEnv.trim() || null
        : fallback.openrouterApiKeyEnv,
  };
}

function patchSessionKey(sessionId: string | null): string {
  return sessionId ?? DEFAULT_PATCH_SESSION_KEY;
}

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

const STALENESS_DEFAULTS = {
  ageWeight: 1.0,
  tokenWeight: 0.5,
  baseRelevance: 1.0,
  referenceBoost: 0.5,
  maxScore: 100.0,
} as const;

function calculateStalenessScore(block: Block, currentTurn: number): number {
  const turnsSinceCreated = Math.max(0, currentTurn - block.metadata.turnIndex);
  const tokenCost = block.tokens / 1000;
  const ageFactor = turnsSinceCreated * STALENESS_DEFAULTS.ageWeight;
  const costFactor = tokenCost * STALENESS_DEFAULTS.tokenWeight;
  const relevanceBoost =
    STALENESS_DEFAULTS.baseRelevance +
    block.referenceCount * STALENESS_DEFAULTS.referenceBoost +
    block.usageHeat;
  const raw = relevanceBoost > 0 ? (ageFactor * costFactor) / relevanceBoost : 0;
  return Math.max(0, Math.min(STALENESS_DEFAULTS.maxScore, raw));
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

const effectiveLimit = $derived(engineBudgetLimit ?? 200_000);
const tokenBudget = $derived<TokenBudget>(calculateTokenBudget(blocks, effectiveLimit));

const activeStateLabel = $derived.by(() => {
  if (!activeSnapshotId) return "Working State";
  const snap = snapshots.find(s => s.id === activeSnapshotId);
  return snap?.name ?? "Unknown Snapshot";
});

// ============================================================================
// Actions
// ============================================================================

type EnginePolicyDecision = "allow" | "deny" | "require_confirmation";

interface EnginePolicyResult {
  decision: EnginePolicyDecision;
  reason?: string;
}

export interface MutationOutcome {
  applied: boolean;
  decision: EnginePolicyDecision | "not_found";
  reason?: string;
  affected: number;
}

function appliedOutcome(affected: number): MutationOutcome {
  return { applied: true, decision: "allow", affected };
}

function blockedOutcome(
  decision: Extract<EnginePolicyDecision, "deny" | "require_confirmation">,
  reason?: string
): MutationOutcome {
  return { applied: false, decision, reason, affected: 0 };
}

function notFoundOutcome(message = "Block not found"): MutationOutcome {
  return { applied: false, decision: "not_found", reason: message, affected: 0 };
}

function parsePolicyDecision(raw: unknown): EnginePolicyResult | null {
  const parsed =
    typeof raw === "string"
      ? (() => {
          try {
            return JSON.parse(raw) as unknown;
          } catch {
            return null;
          }
        })()
      : raw;

  if (parsed && typeof parsed === "object" && "decision" in parsed) {
    const decision = (parsed as { decision?: unknown }).decision;
    if (decision === "allow" || decision === "deny" || decision === "require_confirmation") {
      const reason = (parsed as { reason?: unknown }).reason;
      return {
        decision,
        reason: typeof reason === "string" ? reason : undefined,
      };
    }
  }
  return null;
}

async function invokePolicyDecision(
  command: string,
  payload: Record<string, unknown>
): Promise<EnginePolicyResult | null> {
  try {
    const result = await invokeProxy<unknown>(command, payload);
    return parsePolicyDecision(result);
  } catch {
    // Outside Tauri (web tests/dev), treat as no policy signal.
    return null;
  }
}

function requestPolicyConfirmation(reason?: string): boolean {
  const prompt = reason
    ? `This action requires confirmation.\n\n${reason}\n\nContinue?`
    : "This action requires confirmation. Continue?";

  const confirmFn =
    typeof window !== "undefined" && typeof window.confirm === "function"
      ? window.confirm.bind(window)
      : typeof globalThis.confirm === "function"
        ? globalThis.confirm.bind(globalThis)
        : null;
  if (!confirmFn) {
    return true;
  }

  try {
    return confirmFn(prompt);
  } catch {
    return true;
  }
}

function rememberLocalHotPatch(role: Role, originalContent: string, newContent: string): void {
  if (originalContent === newContent) return;
  const key = patchSessionKey(activeEngineSessionId);
  const current = localHotPatchesBySession[key] ?? [];
  localHotPatchesBySession = {
    ...localHotPatchesBySession,
    [key]: [...current, { role, originalContent, newContent }],
  };
}

function applyLocalHotPatches(nextBlocks: Block[]): Block[] {
  const localHotPatches = localHotPatchesBySession[patchSessionKey(activeEngineSessionId)] ?? [];
  if (localHotPatches.length === 0) return nextBlocks;

  return nextBlocks.map((block) => {
    let patchedContent = block.content;
    for (const patch of localHotPatches) {
      if (patch.role !== block.role) continue;
      if (!patchedContent.includes(patch.originalContent)) continue;
      patchedContent = patchedContent.split(patch.originalContent).join(patch.newContent);
    }
    if (patchedContent === block.content) return block;

    const tokens = Math.ceil(patchedContent.length / 4);
    return {
      ...block,
      content: patchedContent,
      tokens,
      compressedVersions: {
        ...block.compressedVersions,
        original: { content: patchedContent, tokens },
      },
    };
  });
}

async function ensureMutationAllowed(
  command: string,
  payload: Record<string, unknown>
): Promise<MutationOutcome> {
  const firstDecision = await invokePolicyDecision(command, payload);
  if (firstDecision === null || firstDecision.decision === "allow") {
    return appliedOutcome(0);
  }
  if (firstDecision.decision === "deny") {
    return blockedOutcome("deny", firstDecision.reason ?? "Action blocked by policy");
  }

  if (!requestPolicyConfirmation(firstDecision.reason)) {
    return blockedOutcome(
      "require_confirmation",
      firstDecision.reason ?? "Action cancelled"
    );
  }

  const confirmedDecision = await invokePolicyDecision(command, {
    ...payload,
    confirmed: true,
  });
  if (confirmedDecision === null || confirmedDecision.decision === "allow") {
    return appliedOutcome(0);
  }
  if (confirmedDecision.decision === "deny") {
    return blockedOutcome("deny", confirmedDecision.reason ?? "Action blocked by policy");
  }
  return blockedOutcome(
    "require_confirmation",
    confirmedDecision.reason ?? "Action still requires confirmation"
  );
}

function init(): void {
  loadFromLocalStorage();
  // Live proxy context is session-scoped: start empty on app launch.
  blocks = [];
  activeSnapshotId = null;
  workingStateCache = null;
  activeEngineSessionId = null;
  _activeEngineThreadIdentity = null;
  localHotPatchesBySession = {};
  compressionSettings = defaultCompressionSettings();
  saveToLocalStorage();
  void refreshCompressionSettings();
  // No demo data auto-load — start empty, blocks arrive via proxy capture.
  // Call loadDemoData() explicitly from the empty state UI if needed.
}

function loadDemoData(): void {
  blocks = generateDemoBlocks();
  snapshots = generateDemoSnapshots(blocks);
  saveToLocalStorage();
}

function getBlock(id: string): Block | undefined {
  return blocks.find((b) => b.id === id);
}

function getBlockStaleness(id: string): number | undefined {
  const block = blocks.find((candidate) => candidate.id === id);
  if (!block) return undefined;
  const currentTurn =
    blocks.length > 0 ? Math.max(...blocks.map((candidate) => candidate.metadata.turnIndex)) + 1 : 0;
  return calculateStalenessScore(block, currentTurn);
}

function getBlockIndex(id: string): number {
  return blocks.findIndex((b) => b.id === id);
}

async function moveBlock(blockId: string, targetZone: Zone): Promise<MutationOutcome> {
  const index = getBlockIndex(blockId);
  if (index === -1) return notFoundOutcome();

  const oldZone = blocks[index].zone;
  if (oldZone === targetZone) return appliedOutcome(0);

  const permission = await ensureMutationAllowed("engine_move_block", {
    blockId,
    zone: targetZone,
  });
  if (!permission.applied) return permission;

  editHistoryStore.recordEdit(blockId, "zone", { zone: oldZone }, { zone: targetZone });
  blocks[index] = { ...blocks[index], zone: targetZone };
  blocks = [...blocks];
  markDirty();
  return appliedOutcome(1);
}

async function moveBlocks(blockIds: string[], targetZone: Zone): Promise<MutationOutcome> {
  const idSet = new Set(blockIds);
  const existingIds = blocks.filter((b) => idSet.has(b.id)).map((b) => b.id);
  if (existingIds.length === 0) return notFoundOutcome();
  const movableIds = blocks
    .filter((b) => idSet.has(b.id) && b.zone !== targetZone)
    .map((b) => b.id);
  if (movableIds.length === 0) return appliedOutcome(0);
  const movableSet = new Set(movableIds);

  const permission = await ensureMutationAllowed("engine_bulk_move", {
    blockIds: movableIds,
    zone: targetZone,
  });
  if (!permission.applied) return permission;

  let changed = 0;
  blocks = blocks.map((b) => {
    if (!movableSet.has(b.id)) return b;
    editHistoryStore.recordEdit(b.id, "zone", { zone: b.zone }, { zone: targetZone });
    changed += 1;
    return { ...b, zone: targetZone };
  });

  if (changed > 0) {
    blocks = [...blocks];
    markDirty();
  }
  return appliedOutcome(changed);
}

async function removeBlock(blockId: string): Promise<MutationOutcome> {
  const index = getBlockIndex(blockId);
  if (index === -1) return notFoundOutcome();

  const permission = await ensureMutationAllowed("engine_remove_block", { blockId });
  if (!permission.applied) return permission;

  editHistoryStore.clearBlockHistory(blockId);
  blocks = blocks.filter((b) => b.id !== blockId);
  markDirty();
  return appliedOutcome(1);
}

async function removeBlocks(blockIds: string[]): Promise<MutationOutcome> {
  const idSet = new Set(blockIds);
  const existingIds = blocks.filter((b) => idSet.has(b.id)).map((b) => b.id);
  if (existingIds.length === 0) return notFoundOutcome();

  const permission = await ensureMutationAllowed("engine_bulk_remove", {
    blockIds: existingIds,
  });
  if (!permission.applied) return permission;

  for (const id of existingIds) {
    editHistoryStore.clearBlockHistory(id);
  }

  blocks = blocks.filter((b) => !idSet.has(b.id));
  markDirty();
  return appliedOutcome(existingIds.length);
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

async function updateBlockContent(blockId: string, content: string): Promise<MutationOutcome> {
  const index = getBlockIndex(blockId);
  if (index === -1) return notFoundOutcome();
  const block = blocks[index];
  const oldContent = block.content;
  if (oldContent === content) return appliedOutcome(0);

  const permission = await ensureMutationAllowed("engine_update_content", {
    blockId,
    content,
    model: "unknown",
  });
  if (!permission.applied) return permission;

  editHistoryStore.recordEdit(blockId, "content", { content: oldContent }, { content });

  // All supported transport paths are proxy-mutable, so content edits always
  // queue a persistent hot patch for upstream rewrite on subsequent requests.
  invokeProxy("queue_hot_patch", {
    role: block.role,
    originalContent: oldContent,
    newContent: content,
  }).catch(() => {
    // Ignore errors when proxy is not reachable
  });
  rememberLocalHotPatch(block.role, oldContent, content);

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
  return appliedOutcome(1);
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

async function setCompressionLevel(
  blockId: string,
  level: Block["compressionLevel"]
): Promise<MutationOutcome> {
  const index = getBlockIndex(blockId);
  if (index === -1) return notFoundOutcome();
  const oldLevel = blocks[index].compressionLevel;
  if (oldLevel === level) return appliedOutcome(0);

  const permission = await ensureMutationAllowed("engine_compress_block", { blockId, level });
  if (!permission.applied) return permission;

  editHistoryStore.recordEdit(
    blockId, "compression",
    { compressionLevel: oldLevel },
    { compressionLevel: level }
  );

  blocks[index] = { ...blocks[index], compressionLevel: level };
  blocks = [...blocks];
  markDirty();
  return appliedOutcome(1);
}

async function pinBlock(blockId: string, position: Block["pinned"]): Promise<MutationOutcome> {
  const index = getBlockIndex(blockId);
  if (index === -1) return notFoundOutcome();
  const oldPin = blocks[index].pinned;
  if (oldPin === position) return appliedOutcome(0);

  const permission = await ensureMutationAllowed("engine_pin_block", {
    blockId,
    position: position ?? null,
  });
  if (!permission.applied) return permission;

  editHistoryStore.recordEdit(
    blockId, "pin",
    { pinned: oldPin ?? null },
    { pinned: position ?? null }
  );

  blocks[index] = { ...blocks[index], pinned: position };
  blocks = [...blocks];
  markDirty();
  return appliedOutcome(1);
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
  activeEngineSessionId = null;
  _activeEngineThreadIdentity = null;
  localHotPatchesBySession = {};
  markDirty();
}

/** Purge all engine sessions/context and reset the live working state. */
async function clearEngineContext(): Promise<MutationOutcome> {
  const permission = await ensureMutationAllowed("engine_clear_context", {});
  if (!permission.applied) return permission;

  blocks = [];
  activeSnapshotId = null;
  workingStateCache = null;
  activeEngineSessionId = null;
  _activeEngineThreadIdentity = null;
  localHotPatchesBySession = {};
  markDirty();
  return appliedOutcome(1);
}

/** Replace blocks with engine-processed data (enriched with zones, tokens, heat). */
function setEngineBlocks(newBlocks: Block[]): void {
  if (activeSnapshotId !== null) return; // Don't overwrite snapshot view

  // Detect mutations (archival/compression) between old and new blocks
  notifyBlockMutations(blocks, newBlocks);

  blocks = applyLocalHotPatches(newBlocks);
  markDirty();
}

/** Compare old and new block sets, surface context mutations as toasts. */
function notifyBlockMutations(oldBlocks: Block[], newBlocks: Block[]): void {
  if (oldBlocks.length === 0) return; // Initial load, no notifications

  const oldIds = new Set(oldBlocks.map(b => b.id));
  const newIds = new Set(newBlocks.map(b => b.id));
  const newById = new Map(newBlocks.map(b => [b.id, b]));

  const mutations: string[] = [];

  // Detect archived blocks (present in old, absent in new)
  for (const id of oldIds) {
    if (!newIds.has(id)) {
      mutations.push(`archived #${id.slice(0, 6)}`);
    }
  }

  // Detect compressed blocks (compression level changed)
  for (const oldBlock of oldBlocks) {
    const newBlock = newById.get(oldBlock.id);
    if (newBlock && newBlock.compressionLevel !== oldBlock.compressionLevel
        && newBlock.compressionLevel !== "original") {
      mutations.push(`${newBlock.compressionLevel} #${oldBlock.id.slice(0, 6)}`);
    }
  }

  if (mutations.length === 0) return;

  const summary = mutations.length <= 3
    ? mutations.join(", ")
    : `${mutations.length} context updates applied`;
  uiStore.showToast(summary, "info");
}

/** Set the active engine session used for session-scoped local hot patch overlays. */
function setActiveEngineSession(sessionId: string | null, threadIdentity: string | null = null): void {
  activeEngineSessionId = sessionId;
  _activeEngineThreadIdentity = threadIdentity;
}

function setContextMutationMode(mode: ContextMutationMode): void {
  contextMutationMode = mode;
}

/** Set the engine-reported budget limit (overrides default 200k). */
function setEngineBudget(limit: number): void {
  engineBudgetLimit = limit;
}

/** Set the planner budget ceiling (0.40–1.0) and persist to localStorage. */
async function setBudgetCeiling(ceiling: number): Promise<void> {
  const clamped = Math.max(0.4, Math.min(1.0, ceiling));
  budgetCeiling = clamped;
  try {
    localStorage.setItem(BUDGET_CEILING_KEY, clamped.toString());
  } catch { /* ignore */ }
  try {
    await invokeProxy("engine_set_budget_ceiling", { ceiling: clamped });
  } catch { /* engine may not be available */ }
}

/** Refresh compression settings from engine state (best-effort outside Tauri). */
async function refreshCompressionSettings(): Promise<void> {
  try {
    const settings = await invokeProxy<Partial<CompressionSettings>>("engine_get_compression_settings");
    compressionSettings = normalizeCompressionSettings(settings);
  } catch {
    // Ignore when engine is unavailable (web tests/dev).
  }
}

/** Replace compression settings and sync to engine (fail-open on IPC errors). */
async function setCompressionSettings(settings: CompressionSettings): Promise<void> {
  const normalized = normalizeCompressionSettings(settings);
  compressionSettings = normalized;
  try {
    await invokeProxy("engine_update_compression_settings", { settings: normalized });
  } catch {
    // Keep local state even if backend is unavailable.
  }
}

/** Patch compression settings and sync merged result. */
async function updateCompressionSettings(patch: Partial<CompressionSettings>): Promise<void> {
  await setCompressionSettings({
    ...compressionSettings,
    ...patch,
  });
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
    return effectiveLimit;
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
  get contextMutationMode() {
    return contextMutationMode;
  },
  get budgetCeiling() {
    return budgetCeiling;
  },
  get compressionSettings() {
    return compressionSettings;
  },

  // Actions
  init,
  flushPendingWrites,
  loadDemoData,
  setLiveBlocks,
  setEngineBlocks,
  setActiveEngineSession,
  setContextMutationMode,
  setEngineBudget,
  clearBlocks,
  clearEngineContext,
  appendResponseBlocks,
  getBlock,
  getBlockStaleness,
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
  setBudgetCeiling,
  refreshCompressionSettings,
  setCompressionSettings,
  updateCompressionSettings,
};
