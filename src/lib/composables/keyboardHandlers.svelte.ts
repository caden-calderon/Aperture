/**
 * Keyboard Handlers Composable
 *
 * Encapsulates keyboard navigation (J/K/Arrow) and shortcut handling.
 * Owns the `allBlocksFlat` derived list and `navigateBlock` helper.
 */

import type { selectionStore as SelectionStoreType } from "$lib/stores/selection.svelte";
import type { uiStore as UiStoreType } from "$lib/stores/ui.svelte";
import type { contextStore as ContextStoreType } from "$lib/stores/context.svelte";
import type { searchStore as SearchStoreType } from "$lib/stores/search.svelte";
import type { terminalStore as TerminalStoreType } from "$lib/stores/terminal.svelte";
import type { zonesStore as ZonesStoreType } from "$lib/stores/zones.svelte";
import type TerminalPanel from "$lib/components/layout/TerminalPanel.svelte";

interface KeyboardHandlerStores {
  selectionStore: typeof SelectionStoreType;
  uiStore: typeof UiStoreType;
  contextStore: typeof ContextStoreType;
  searchStore: typeof SearchStoreType;
  terminalStore: typeof TerminalStoreType;
  zonesStore: typeof ZonesStoreType;
}

interface KeyboardHandlerRefs {
  readonly terminalPanelComponentRef: ReturnType<typeof TerminalPanel> | null;
  readonly terminalPanelRef: HTMLElement | null;
}

export function createKeyboardHandlers(stores: KeyboardHandlerStores, refs: KeyboardHandlerRefs) {
  const { selectionStore, uiStore, contextStore, searchStore, terminalStore, zonesStore } = stores;

  function blockedReason(reason: string | undefined, fallback = "Action blocked by policy"): string {
    return reason && reason.trim().length > 0 ? reason : fallback;
  }

  // Flat ordered list of all visible blocks (zone display order)
  const allBlocksFlat = $derived(
    zonesStore.zonesByDisplayOrder.flatMap(z => contextStore.blocksByZone[z.id] ?? [])
  );

  // Navigate blocks with keyboard (J/K/Arrow)
  function navigateBlock(direction: 'up' | 'down'): void {
    const flat = allBlocksFlat;
    if (flat.length === 0) return;

    const currentFocused = selectionStore.focusedId;
    const currentIdx = currentFocused ? flat.findIndex(b => b.id === currentFocused) : -1;

    let nextIdx: number;
    if (currentIdx === -1) {
      // No current focus -- start from first or last
      nextIdx = direction === 'down' ? 0 : flat.length - 1;
    } else {
      nextIdx = direction === 'down' ? currentIdx + 1 : currentIdx - 1;
    }

    // Clamp to bounds
    if (nextIdx < 0 || nextIdx >= flat.length) return;
    selectionStore.focus(flat[nextIdx].id);
  }

  // Keyboard shortcuts
  async function handleKeydown(e: KeyboardEvent) {
    // Ctrl+T toggles terminal, even from inputs
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "t") {
      e.preventDefault();
      // If terminal is visible and focused, hide it. If visible but not focused, focus it.
      // If hidden, show and focus it.
      if (terminalStore.isVisible) {
        const termEl = refs.terminalPanelRef?.querySelector('.xterm-helper-textarea');
        if (termEl && document.activeElement === termEl) {
          terminalStore.hide();
        } else {
          requestAnimationFrame(() => refs.terminalPanelComponentRef?.focusTerminal());
        }
      } else {
        terminalStore.show();
        requestAnimationFrame(() => refs.terminalPanelComponentRef?.focusTerminal());
      }
      return;
    }

    // Ctrl+F always opens search, even from inputs
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "f") {
      e.preventDefault();
      searchStore.toggle();
      return;
    }

    if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) {
      return;
    }

    if (uiStore.hasModal || uiStore.commandPaletteOpen) {
      return;
    }

    // F3 / Shift+F3 for search navigation (when search is open)
    if (e.key === "F3" && searchStore.isOpen) {
      e.preventDefault();
      if (e.shiftKey) searchStore.previousMatch();
      else searchStore.nextMatch();
      return;
    }

    switch (e.key.toLowerCase()) {
      case "a":
        if (!e.ctrlKey && !e.metaKey) {
          e.preventDefault();
          selectionStore.selectAll();
          uiStore.showToast("Selected all blocks", "info");
        }
        break;
      case "escape":
        e.preventDefault();
        if (searchStore.isOpen) {
          searchStore.close();
        } else {
          selectionStore.deselect();
        }
        break;
      case "delete":
      case "backspace":
        if (selectionStore.hasSelection) {
          e.preventDefault();
          const result = await contextStore.removeBlocks([...selectionStore.selectedIds]);
          if (result.applied) {
            selectionStore.deselect();
            uiStore.showToast(`Removed ${result.affected} block(s)`, "success");
          } else {
            uiStore.showToast(blockedReason(result.reason), "warning");
          }
        }
        break;
      case "s":
        if (!e.ctrlKey && !e.metaKey) {
          e.preventDefault();
          const snap = contextStore.saveSnapshot(`Snapshot ${contextStore.snapshots.length + 1}`);
          uiStore.showToast(`Saved: ${snap.name}`, "success");
        }
        break;
      case "k":
        if (e.ctrlKey || e.metaKey) {
          e.preventDefault();
          uiStore.toggleCommandPalette();
        } else {
          e.preventDefault();
          navigateBlock('up');
        }
        break;
      case "[":
        if (e.ctrlKey || e.metaKey) {
          e.preventDefault();
          uiStore.toggleSidebar();
        }
        break;
      case "j":
        if (!e.ctrlKey && !e.metaKey) {
          e.preventDefault();
          navigateBlock('down');
        }
        break;
      case "arrowdown":
        e.preventDefault();
        navigateBlock('down');
        break;
      case "arrowup":
        e.preventDefault();
        navigateBlock('up');
        break;
      case "enter":
        if (selectionStore.focusedId) {
          e.preventDefault();
          uiStore.openModal(selectionStore.focusedId);
        }
        break;
    }
  }

  return {
    handleKeydown,
  };
}
