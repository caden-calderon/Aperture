/**
 * Block Handlers Composable
 *
 * Encapsulates block interaction handlers (select, drag, context menu, zone ops)
 * and context menu state.
 */

import type { selectionStore as SelectionStoreType } from "$lib/stores/selection.svelte";
import type { uiStore as UiStoreType } from "$lib/stores/ui.svelte";
import type { contextStore as ContextStoreType } from "$lib/stores/context.svelte";
import type { zonesStore as ZonesStoreType } from "$lib/stores/zones.svelte";
import type { blockTypesStore as BlockTypesStoreType } from "$lib/stores/blockTypes.svelte";
import type { Zone as ZoneType, Block } from "$lib/types";
import { resolveTypeSelection } from "$lib/utils/blockTypes";

interface BlockHandlerStores {
  selectionStore: typeof SelectionStoreType;
  uiStore: typeof UiStoreType;
  contextStore: typeof ContextStoreType;
  zonesStore: typeof ZonesStoreType;
  blockTypesStore: typeof BlockTypesStoreType;
}

export function createBlockHandlers(stores: BlockHandlerStores) {
  const { selectionStore, uiStore, contextStore, zonesStore, blockTypesStore } = stores;

  function blockedReason(reason: string | undefined, fallback = "Action blocked by policy"): string {
    return reason && reason.trim().length > 0 ? reason : fallback;
  }

  function ensureContextEditable(): boolean {
    if (!contextStore.isReadOnlyMode) return true;
    uiStore.showToast(contextStore.mutationBlockedReason, "warning");
    return false;
  }

  // Context menu state
  let contextMenuBlock = $state<string | null>(null);
  let contextMenuX = $state(0);
  let contextMenuY = $state(0);
  let contextMenuVisible = $state(false);

  function handleBlockSelect(id: string, event: { shiftKey: boolean; ctrlKey: boolean; metaKey: boolean }) {
    selectionStore.handleClick(id, event);
  }

  function handleBlockDoubleClick(id: string) {
    uiStore.openModal(id);
  }

  function handleBlockDragStart(ids: string[]) {
    uiStore.startDrag(ids);
  }

  function handleBlockDragEnd() {
    uiStore.endDrag();
  }

  function handleBlockContextMenu(id: string, e: MouseEvent) {
    e.preventDefault();
    contextMenuBlock = id;
    contextMenuX = e.clientX;
    contextMenuY = e.clientY;
    contextMenuVisible = true;
    // Select the block if not already selected
    if (!selectionStore.selectedIds.has(id)) {
      selectionStore.select(id);
    }
  }

  function closeContextMenu() {
    contextMenuVisible = false;
    contextMenuBlock = null;
  }

  async function handleZoneDrop(zone: ZoneType, blockIds: string[]) {
    const result = await contextStore.moveBlocks(blockIds, zone);
    if (!result.applied) {
      uiStore.showToast(blockedReason(result.reason), "warning");
      return;
    }

    const count = result.affected;
    const zoneName = zonesStore.getZoneById(zone)?.label ?? zone;
    if (count === 0) {
      uiStore.showToast(`No blocks moved to ${zoneName}`, "info");
    } else {
      uiStore.showToast(`Moved ${count > 1 ? `${count} blocks` : '1 block'} to ${zoneName}`, "info");
    }
  }

  function handleZoneReorder(zone: ZoneType, blockIds: string[], insertIndex: number) {
    if (!ensureContextEditable()) return;
    contextStore.reorderBlocksInZone(zone, blockIds, insertIndex);
    const count = blockIds.length;
    uiStore.showToast(`Reordered ${count > 1 ? `${count} blocks` : 'block'}`, "info");
  }

  function handleCreateBlock(zone: ZoneType, typeId: string) {
    if (!ensureContextEditable()) return;
    // Create a new block with the specified type
    const blockTypeInfo = blockTypesStore.getTypeById(typeId);
    const label = blockTypeInfo?.label ?? typeId;

    // New blocks default to "user" role unless a built-in role is selected.
    const { role, blockType } = resolveTypeSelection(typeId, "user");
    const content = `New ${label} block`;
    const newBlock = contextStore.createBlock(zone, role, content, blockType);
    selectionStore.select(newBlock.id);
    uiStore.openModal(newBlock.id);
    const zoneName = zonesStore.getZoneById(zone)?.label ?? zone;
    uiStore.showToast(`Created ${label} block in ${zoneName}`, "success");
  }

  function handleToggleZoneCollapse(zone: ZoneType) {
    uiStore.toggleZoneCollapse(zone);
  }

  // Context menu action handlers
  async function handleContextMenuPin(pos: "top" | "bottom" | null) {
    if (contextMenuBlock) {
      const result = await contextStore.pinBlock(contextMenuBlock, pos);
      if (!result.applied) {
        uiStore.showToast(blockedReason(result.reason), 'warning');
        return;
      }
      const label = pos ? `Pinned to ${pos}` : 'Unpinned';
      uiStore.showToast(label, 'success');
    }
  }

  async function handleContextMenuMove(zone: ZoneType) {
    if (contextMenuBlock) {
      const result = await contextStore.moveBlock(contextMenuBlock, zone);
      if (!result.applied) {
        uiStore.showToast(blockedReason(result.reason), 'warning');
        return;
      }
      const zoneName = zonesStore.getZoneById(zone)?.label ?? zone;
      uiStore.showToast(`Moved to ${zoneName}`, 'info');
    }
  }

  async function handleContextMenuCompress(level: Block["compressionLevel"]) {
    if (contextMenuBlock) {
      const result = await contextStore.setCompressionLevel(contextMenuBlock, level);
      if (!result.applied) {
        uiStore.showToast(blockedReason(result.reason), 'warning');
        return;
      }
      uiStore.showToast(`Set to ${level}`, 'success');
    }
  }

  function handleContextMenuCopy() {
    if (contextMenuBlock) {
      const block = contextStore.getBlock(contextMenuBlock);
      if (block) {
        navigator.clipboard.writeText(block.content);
        uiStore.showToast('Copied to clipboard', 'success');
      }
    }
  }

  async function handleContextMenuRemove() {
    if (contextMenuBlock) {
      const result = await contextStore.removeBlock(contextMenuBlock);
      if (!result.applied) {
        uiStore.showToast(blockedReason(result.reason), 'warning');
        return;
      }
      uiStore.showToast('Block removed', 'success');
    }
  }

  function handleContextMenuOpen() {
    if (contextMenuBlock) {
      uiStore.openModal(contextMenuBlock);
    }
  }

  return {
    // Context menu state
    get contextMenuBlock() { return contextMenuBlock; },
    get contextMenuX() { return contextMenuX; },
    get contextMenuY() { return contextMenuY; },
    get contextMenuVisible() { return contextMenuVisible; },

    // Handlers
    handleBlockSelect,
    handleBlockDoubleClick,
    handleBlockDragStart,
    handleBlockDragEnd,
    handleBlockContextMenu,
    closeContextMenu,
    handleZoneDrop,
    handleZoneReorder,
    handleCreateBlock,
    handleToggleZoneCollapse,
    handleContextMenuPin,
    handleContextMenuMove,
    handleContextMenuCompress,
    handleContextMenuCopy,
    handleContextMenuRemove,
    handleContextMenuOpen,
  };
}
