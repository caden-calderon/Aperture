/**
 * Modal Handlers Composable
 *
 * Encapsulates modal action handlers for the block detail modal
 * (close, compress, move, pin, remove, content edit, role change).
 */

import type { uiStore as UiStoreType } from "$lib/stores/ui.svelte";
import type { contextStore as ContextStoreType } from "$lib/stores/context.svelte";
import type { blockTypesStore as BlockTypesStoreType } from "$lib/stores/blockTypes.svelte";
import type { Zone as ZoneType, Block } from "$lib/types";

interface ModalHandlerStores {
  uiStore: typeof UiStoreType;
  contextStore: typeof ContextStoreType;
  blockTypesStore: typeof BlockTypesStoreType;
}

export function createModalHandlers(stores: ModalHandlerStores) {
  const { uiStore, contextStore, blockTypesStore } = stores;

  function blockedReason(reason: string | undefined, fallback = "Action blocked by policy"): string {
    return reason && reason.trim().length > 0 ? reason : fallback;
  }

  function handleModalClose() {
    uiStore.closeModal();
  }

  async function handleModalCompress(level: Block["compressionLevel"]) {
    if (uiStore.modalBlockId) {
      const result = await contextStore.setCompressionLevel(uiStore.modalBlockId, level);
      if (!result.applied) {
        uiStore.showToast(blockedReason(result.reason), "warning");
      }
    }
  }

  async function handleModalMove(zone: ZoneType) {
    if (uiStore.modalBlockId) {
      const result = await contextStore.moveBlock(uiStore.modalBlockId, zone);
      if (!result.applied) {
        uiStore.showToast(blockedReason(result.reason), "warning");
      }
    }
  }

  async function handleModalPin(position: Block["pinned"]) {
    if (uiStore.modalBlockId) {
      const result = await contextStore.pinBlock(uiStore.modalBlockId, position);
      if (!result.applied) {
        uiStore.showToast(blockedReason(result.reason), "warning");
      }
    }
  }

  async function handleModalRemove() {
    if (uiStore.modalBlockId) {
      const result = await contextStore.removeBlock(uiStore.modalBlockId);
      if (!result.applied) {
        uiStore.showToast(blockedReason(result.reason), "warning");
        return;
      }
      uiStore.closeModal();
      uiStore.showToast("Block removed", "success");
    }
  }

  async function handleModalContentEdit(content: string) {
    if (uiStore.modalBlockId) {
      const result = await contextStore.updateBlockContent(uiStore.modalBlockId, content);
      if (!result.applied) {
        uiStore.showToast(blockedReason(result.reason), "warning");
        return;
      }
      uiStore.showToast("Content updated", "success");
    }
  }

  function handleModalRoleChange(role: Block["role"], blockType?: string) {
    if (uiStore.modalBlockId) {
      contextStore.setBlockRole(uiStore.modalBlockId, role, blockType);
      const label = blockType
        ? blockTypesStore.getTypeById(blockType)?.label ?? blockType
        : role;
      uiStore.showToast(`Changed type to ${label}`, "success");
    }
  }

  return {
    handleModalClose,
    handleModalCompress,
    handleModalMove,
    handleModalPin,
    handleModalRemove,
    handleModalContentEdit,
    handleModalRoleChange,
  };
}
