/**
 * Selection Store
 * Manages block selection state with support for:
 * - Single click selection
 * - Shift+click range selection
 * - Ctrl/Cmd+click toggle selection
 * - Select all / deselect
 */

import { SvelteSet } from "svelte/reactivity";
import { contextStore } from "./context.svelte";
import { matchesDisplayType } from "$lib/utils/blockTypes";

// ============================================================================
// State
// ============================================================================

const selectedIds = $state(new SvelteSet<string>());
let _lastSelectedId = $state<string | null>(null);
let lastSelectedIndex = $state<number | null>(null);
let focusedId = $state<string | null>(null);

// ============================================================================
// Derived State
// ============================================================================

const selectedBlocks = $derived(
  contextStore.blocks.filter((b) => selectedIds.has(b.id))
);

const selectedTokens = $derived(
  selectedBlocks.reduce((sum, b) => sum + b.tokens, 0)
);

const hasSelection = $derived(selectedIds.size > 0);

// ============================================================================
// Actions
// ============================================================================

function select(blockId: string): void {
  selectedIds.clear();
  selectedIds.add(blockId);
  _lastSelectedId = blockId;
  lastSelectedIndex = contextStore.getBlockIndex(blockId);
}

function toggle(blockId: string): void {
  if (selectedIds.has(blockId)) {
    selectedIds.delete(blockId);
  } else {
    selectedIds.add(blockId);
  }
  _lastSelectedId = blockId;
  lastSelectedIndex = contextStore.getBlockIndex(blockId);
}

function rangeSelect(blockId: string): void {
  if (lastSelectedIndex === null) {
    select(blockId);
    return;
  }

  const blocks = contextStore.blocks;
  const targetIndex = contextStore.getBlockIndex(blockId);
  if (targetIndex === -1) return;

  const start = Math.min(lastSelectedIndex, targetIndex);
  const end = Math.max(lastSelectedIndex, targetIndex);

  for (let i = start; i <= end; i++) {
    selectedIds.add(blocks[i].id);
  }
}

function selectAll(): void {
  selectedIds.clear();
  for (const b of contextStore.blocks) {
    selectedIds.add(b.id);
  }
}

function selectZone(zone: string): void {
  const zoneBlocks = contextStore.blocksByZone[zone] ?? [];
  selectedIds.clear();
  for (const b of zoneBlocks) {
    selectedIds.add(b.id);
  }
  lastSelectedIndex =
    zoneBlocks.length > 0 ? contextStore.getBlockIndex(zoneBlocks[0].id) : null;
}

function selectByType(typeId: string): void {
  const matchingBlocks = contextStore.blocks.filter((b) =>
    matchesDisplayType(b, typeId)
  );
  selectedIds.clear();
  for (const b of matchingBlocks) {
    selectedIds.add(b.id);
  }
  lastSelectedIndex =
    matchingBlocks.length > 0
      ? contextStore.getBlockIndex(matchingBlocks[0].id)
      : null;
}

function selectByRole(role: string): void {
  selectByType(role);
}

function deselect(): void {
  selectedIds.clear();
  _lastSelectedId = null;
  lastSelectedIndex = null;
  focusedId = null;
}

/**
 * Focus a specific block by ID (keyboard navigation).
 * Also selects it as a single selection.
 */
function focus(blockId: string): void {
  select(blockId);
  focusedId = blockId;
}

function isSelected(blockId: string): boolean {
  return selectedIds.has(blockId);
}

function handleClick(
  blockId: string,
  event: { shiftKey: boolean; ctrlKey: boolean; metaKey: boolean }
): void {
  if (event.shiftKey) {
    rangeSelect(blockId);
  } else if (event.ctrlKey || event.metaKey) {
    toggle(blockId);
  } else {
    select(blockId);
  }
}

function pruneStaleSelection(): void {
  const validIds = new Set(contextStore.blocks.map((b) => b.id));
  for (const id of selectedIds) {
    if (!validIds.has(id)) {
      selectedIds.delete(id);
    }
  }
  if (focusedId !== null && !validIds.has(focusedId)) {
    focusedId = null;
  }
  if (_lastSelectedId !== null && !validIds.has(_lastSelectedId)) {
    _lastSelectedId = null;
    lastSelectedIndex = null;
  }
}

// ============================================================================
// Export Store Interface
// ============================================================================

export const selectionStore = {
  // Reactive getters
  get selectedIds() {
    pruneStaleSelection();
    return selectedIds;
  },
  get selectedBlocks() {
    pruneStaleSelection();
    return selectedBlocks;
  },
  get selectedTokens() {
    pruneStaleSelection();
    return selectedTokens;
  },
  get hasSelection() {
    pruneStaleSelection();
    return hasSelection;
  },
  get count() {
    pruneStaleSelection();
    return selectedIds.size;
  },
  get focusedId() {
    pruneStaleSelection();
    return focusedId;
  },

  // Actions
  select,
  toggle,
  rangeSelect,
  selectAll,
  selectZone,
  selectByType,
  selectByRole,
  deselect,
  isSelected,
  handleClick,
  focus,
  pruneStaleSelection,
};
