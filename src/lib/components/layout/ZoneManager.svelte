<script lang="ts">
  import { contextStore, selectionStore, uiStore, zonesStore } from '$lib/stores';
  import type { Zone } from '$lib/types';

  let expanded = $state(true);
  let showAddDialog = $state(false);
  let editingZone = $state<typeof zonesStore.allZones[0] | null>(null);
  let confirmingDeleteId = $state<string | null>(null);
  let draggedZoneId = $state<string | null>(null);
  let dragOverZoneId = $state<string | null>(null);

  // Form state
  let newLabel = $state('');
  let newColor = $state('#6b8e23');

  function formatNumber(n: number): string {
    if (n >= 1000) return (n / 1000).toFixed(1) + 'k';
    return n.toString();
  }

  function getBlockCount(zoneId: string): number {
    return contextStore.blocksByZone[zoneId]?.length ?? 0;
  }

  function getTokenCount(zoneId: string): number {
    const zoneBlocks = contextStore.blocksByZone[zoneId] ?? [];
    return zoneBlocks.reduce((sum, b) => sum + b.tokens, 0);
  }

  function blockedReason(reason: string | undefined, fallback = "Action blocked by policy"): string {
    return reason && reason.trim().length > 0 ? reason : fallback;
  }

  // Click on zone: if no selection, select all in zone; if has selection, move to zone
  async function handleZoneClick(zoneId: string) {
    if (selectionStore.hasSelection) {
      const blockIds = [...selectionStore.selectedIds];
      const result = await contextStore.moveBlocks(blockIds, zoneId as Zone);
      if (!result.applied) {
        uiStore.showToast(blockedReason(result.reason), 'warning');
        return;
      }
      const count = result.affected;
      const zone = zonesStore.getZoneById(zoneId);
      if (count === 0) {
        uiStore.showToast(`No blocks moved to ${zone?.label ?? zoneId}`, 'info');
      } else {
        uiStore.showToast(`Moved ${count} block(s) to ${zone?.label ?? zoneId}`, 'success');
      }
    } else {
      selectionStore.selectZone(zoneId as "primacy" | "middle" | "recency");
      const count = selectionStore.count;
      const zone = zonesStore.getZoneById(zoneId);
      if (count > 0) {
        uiStore.showToast(`Selected ${count} block(s) in ${zone?.label ?? zoneId}`, 'info');
      } else {
        uiStore.showToast(`No blocks in ${zone?.label ?? zoneId}`, 'info');
      }
    }
  }

  // Handle drop on zone (blocks)
  async function handleBlockDrop(e: DragEvent, zoneId: string) {
    e.preventDefault();
    e.stopPropagation();
    const data = e.dataTransfer?.getData('text/plain');
    if (data) {
      try {
        const blockIds: string[] = JSON.parse(data);
        if (Array.isArray(blockIds)) {
          const result = await contextStore.moveBlocks(blockIds, zoneId as Zone);
          if (!result.applied) {
            uiStore.showToast(blockedReason(result.reason), 'warning');
            return;
          }
          const zone = zonesStore.getZoneById(zoneId);
          if (result.affected === 0) {
            uiStore.showToast(`No blocks moved to ${zone?.label ?? zoneId}`, 'info');
          } else {
            uiStore.showToast(`Moved ${result.affected} block(s) to ${zone?.label ?? zoneId}`, 'success');
          }
        }
      } catch {
        const result = await contextStore.moveBlock(data, zoneId as Zone);
        if (!result.applied) {
          uiStore.showToast(blockedReason(result.reason), 'warning');
          return;
        }
        const zone = zonesStore.getZoneById(zoneId);
        uiStore.showToast(`Moved block to ${zone?.label ?? zoneId}`, 'success');
      }
    }
  }

  function handleDragOver(e: DragEvent) {
    e.preventDefault();
    if (e.dataTransfer) {
      e.dataTransfer.dropEffect = 'move';
    }
  }

  // Zone reordering via drag
  function handleZoneDragStart(e: DragEvent, zoneId: string) {
    draggedZoneId = zoneId;
    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = 'move';
      e.dataTransfer.setData('application/x-zone-reorder', zoneId);
    }
  }

  function handleZoneDragOver(e: DragEvent, zoneId: string) {
    e.preventDefault();
    if (draggedZoneId && draggedZoneId !== zoneId) {
      dragOverZoneId = zoneId;
    }
  }

  function handleZoneDragLeave() {
    dragOverZoneId = null;
  }

  async function handleZoneDrop(e: DragEvent, targetZoneId: string) {
    e.preventDefault();
    e.stopPropagation();

    // Check if this is a zone reorder
    const draggedId = e.dataTransfer?.getData('application/x-zone-reorder');
    if (draggedId && draggedId !== targetZoneId) {
      // Reorder zones
      const currentOrder = zonesStore.zonesByDisplayOrder.map(z => z.id);
      const fromIndex = currentOrder.indexOf(draggedId);
      const toIndex = currentOrder.indexOf(targetZoneId);

      if (fromIndex !== -1 && toIndex !== -1) {
        currentOrder.splice(fromIndex, 1);
        currentOrder.splice(toIndex, 0, draggedId);
        zonesStore.reorderZonesDisplay(currentOrder);
        uiStore.showToast('Zones reordered', 'info');
      }
    } else {
      // It's a block drop
      await handleBlockDrop(e, targetZoneId);
    }

    draggedZoneId = null;
    dragOverZoneId = null;
  }

  function handleZoneDragEnd() {
    draggedZoneId = null;
    dragOverZoneId = null;
  }

  // CRUD operations
  function handleAdd() {
    if (newLabel.trim()) {
      zonesStore.addCustomZone(newLabel.trim(), newColor);
      resetForm();
      showAddDialog = false;
      uiStore.showToast(`Created zone "${newLabel.trim()}"`, 'success');
    }
  }

  function handleEdit(zone: typeof zonesStore.allZones[0]) {
    editingZone = zone;
    newLabel = zone.label;
    // For built-in zones, try to get the current effective color
    // If it's a CSS variable, use a sensible default based on zone
    if (zone.color.startsWith('var(')) {
      // Get computed value if possible, or use defaults
      if (zone.id === 'primacy') newColor = '#4a9d9a';
      else if (zone.id === 'middle') newColor = '#b8a04a';
      else if (zone.id === 'recency') newColor = '#a85d5d';
      else newColor = '#6b8e23';
    } else {
      newColor = zone.color;
    }
  }

  function handleSaveEdit() {
    if (editingZone && newLabel.trim()) {
      zonesStore.updateZone(editingZone.id, {
        label: newLabel.trim(),
        color: newColor,
      });
      resetForm();
      editingZone = null;
      uiStore.showToast('Zone updated', 'success');
    }
  }

  function handleDeleteClick(id: string) {
    confirmingDeleteId = id;
  }

  function handleConfirmDelete() {
    if (confirmingDeleteId) {
      const zone = zonesStore.getZoneById(confirmingDeleteId);
      zonesStore.deleteZone(confirmingDeleteId);
      uiStore.showToast(`Deleted zone "${zone?.label}"`, 'success');
      confirmingDeleteId = null;
    }
  }

  function handleCancelDelete() {
    confirmingDeleteId = null;
  }

  function resetForm() {
    newLabel = '';
    newColor = '#6b8e23';
  }

  function cancelDialog() {
    showAddDialog = false;
    editingZone = null;
    resetForm();
  }

  function handleResetBuiltIn() {
    if (editingZone?.isBuiltIn) {
      zonesStore.resetBuiltInZone(editingZone.id);
      uiStore.showToast(`Reset ${editingZone.label} to defaults`, 'success');
      editingZone = null;
      resetForm();
    }
  }
</script>

<div class="zone-manager">
  <button class="section-header" onclick={() => expanded = !expanded}>
    <span class="section-title">Zones</span>
    <span class="section-toggle">{expanded ? '−' : '+'}</span>
  </button>

  {#if expanded}
    <div class="section-content">
      <p class="section-hint">Drag zones to reorder display. Click to select/move blocks.</p>

      <div class="zone-list">
        {#each zonesStore.zonesByDisplayOrder as zone (zone.id)}
          {#if confirmingDeleteId === zone.id}
            <!-- Inline delete confirmation -->
            <div class="delete-confirm">
              <span class="delete-confirm-text">Delete "{zone.label}"?</span>
              <div class="delete-confirm-actions">
                <button class="confirm-btn confirm-btn-cancel" onclick={handleCancelDelete}>No</button>
                <button class="confirm-btn confirm-btn-delete" onclick={handleConfirmDelete}>Yes</button>
              </div>
            </div>
          {:else}
            <div
              class="zone-item-wrapper"
              class:drag-over={dragOverZoneId === zone.id}
              class:dragging={draggedZoneId === zone.id}
            >
              <button
                class="zone-item"
                class:has-selection={selectionStore.hasSelection}
                class:is-built-in={zone.isBuiltIn}
                onclick={() => handleZoneClick(zone.id)}
                draggable="true"
                ondragstart={(e) => handleZoneDragStart(e, zone.id)}
                ondragover={(e) => { handleDragOver(e); handleZoneDragOver(e, zone.id); }}
                ondragleave={handleZoneDragLeave}
                ondrop={(e) => handleZoneDrop(e, zone.id)}
                ondragend={handleZoneDragEnd}
                title={selectionStore.hasSelection
                  ? `Click to move ${selectionStore.count} block(s) to ${zone.label}`
                  : `Click to select all blocks in ${zone.label}. Drag to reorder.`}
              >
                <span class="zone-drag-handle">⋮⋮</span>
                <span class="zone-dot" style:background={zone.color}></span>
                <span class="zone-label">{zone.label}</span>
                <span class="zone-stats">
                  <span class="zone-count">{getBlockCount(zone.id)}</span>
                  <span class="zone-tokens">{formatNumber(getTokenCount(zone.id))}</span>
                </span>
              </button>
              <div class="zone-actions">
                <button class="zone-action-btn" onclick={() => handleEdit(zone)} title="Edit">
                  <span>...</span>
                </button>
                {#if !zone.isBuiltIn}
                  <button class="zone-action-btn zone-action-delete" onclick={() => handleDeleteClick(zone.id)} title="Delete">
                    <span>×</span>
                  </button>
                {/if}
              </div>
            </div>
          {/if}
        {/each}
      </div>

      <!-- Add button -->
      <button class="add-zone-btn" onclick={() => showAddDialog = true}>
        + Add Custom Zone
      </button>

      <!-- Add/Edit Dialog -->
      {#if showAddDialog || editingZone}
        <div class="dialog">
          <div class="dialog-header">
            <div class="dialog-title">{editingZone ? 'Edit Zone' : 'New Zone'}</div>
            {#if editingZone?.isBuiltIn}
              <button class="dialog-reset-btn" onclick={handleResetBuiltIn} title="Reset to defaults">
                Reset
              </button>
            {/if}
          </div>
          <div class="dialog-field">
            <label for="zone-label">Label</label>
            <input
              id="zone-label"
              type="text"
              placeholder="e.g. Important"
              bind:value={newLabel}
            />
          </div>
          <div class="dialog-field">
            <label for="zone-color">Color</label>
            <div class="color-input-row">
              <input
                id="zone-color"
                type="color"
                bind:value={newColor}
              />
              <button
                class="color-hex-btn"
                onclick={() => { navigator.clipboard.writeText(newColor); uiStore.showToast('Copied ' + newColor, 'info'); }}
                title="Click to copy"
              >
                {newColor}
              </button>
            </div>
          </div>
          <div class="dialog-actions">
            <button class="dialog-btn dialog-btn-cancel" onclick={cancelDialog}>Cancel</button>
            <button
              class="dialog-btn dialog-btn-save"
              onclick={editingZone ? handleSaveEdit : handleAdd}
              disabled={!newLabel.trim()}
            >
              {editingZone ? 'Save' : 'Add'}
            </button>
          </div>
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .zone-manager {
    border-top: 1px solid var(--border-subtle);
  }

  .section-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    padding: 10px 12px;
    background: transparent;
    border: none;
    cursor: pointer;
    text-align: left;
  }

  .section-header:hover {
    background: var(--bg-hover);
  }

  .section-title {
    font-size: 9px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--text-muted);
  }

  .section-toggle {
    font-size: 12px;
    color: var(--text-muted);
  }

  .section-content {
    padding: 0 12px 12px;
  }

  .section-hint {
    font-size: 9px;
    color: var(--text-faint);
    margin: 0 0 8px 0;
    font-style: italic;
  }

  .zone-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .zone-item-wrapper {
    position: relative;
    display: flex;
    align-items: center;
    border-radius: 4px;
    transition: all 0.1s ease;
  }

  .zone-item-wrapper:hover .zone-actions {
    opacity: 1;
  }

  .zone-item-wrapper.drag-over {
    background: var(--accent-subtle);
    box-shadow: inset 0 0 0 1px var(--accent);
  }

  .zone-item-wrapper.dragging {
    opacity: 0.5;
  }

  .zone-item {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 8px;
    font-size: 11px;
    background: transparent;
    border: 1px solid transparent;
    border-radius: 4px;
    cursor: grab;
    transition: all 0.1s ease;
    text-align: left;
    width: 100%;
  }

  .zone-item:hover {
    background: var(--bg-hover);
    border-color: var(--border-subtle);
  }

  .zone-item.has-selection {
    cursor: pointer;
  }

  .zone-item.has-selection:hover {
    background: var(--accent-subtle);
    border-color: var(--accent);
  }

  .zone-drag-handle {
    color: var(--text-faint);
    font-size: 8px;
    letter-spacing: -1px;
    opacity: 0.5;
    cursor: grab;
  }

  .zone-item:hover .zone-drag-handle {
    opacity: 1;
  }

  .zone-dot {
    width: 8px;
    height: 8px;
    border-radius: 2px;
    flex-shrink: 0;
  }

  .zone-label {
    color: var(--text-secondary);
    font-weight: 500;
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .zone-stats {
    display: flex;
    gap: 6px;
    font-family: var(--font-mono);
    font-size: 9px;
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
  }

  .zone-count {
    min-width: 16px;
    text-align: right;
  }

  .zone-tokens {
    min-width: 30px;
    text-align: right;
  }

  .zone-actions {
    display: flex;
    gap: 2px;
    opacity: 0;
    transition: opacity 0.1s ease;
    position: absolute;
    right: 4px;
  }

  .zone-action-btn {
    width: 16px;
    height: 16px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--bg-elevated);
    border: 1px solid var(--border-subtle);
    border-radius: 2px;
    cursor: pointer;
    font-size: 10px;
    color: var(--text-muted);
    padding: 0;
  }

  .zone-action-btn:hover {
    background: var(--bg-hover);
    color: var(--text-primary);
  }

  .zone-action-delete:hover {
    background: var(--semantic-danger);
    color: white;
    border-color: var(--semantic-danger);
  }

  /* Delete confirmation */
  .delete-confirm {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 6px 8px;
    background: color-mix(in srgb, var(--semantic-danger) 10%, var(--bg-elevated));
    border: 1px solid var(--semantic-danger);
    border-radius: 4px;
    gap: 8px;
  }

  .delete-confirm-text {
    font-size: 10px;
    color: var(--text-primary);
    flex: 1;
  }

  .delete-confirm-actions {
    display: flex;
    gap: 4px;
  }

  .confirm-btn {
    padding: 3px 8px;
    font-size: 9px;
    font-family: var(--font-mono);
    border-radius: 3px;
    cursor: pointer;
    border: none;
  }

  .confirm-btn-cancel {
    background: var(--bg-surface);
    color: var(--text-secondary);
    border: 1px solid var(--border-default);
  }

  .confirm-btn-cancel:hover {
    background: var(--bg-hover);
  }

  .confirm-btn-delete {
    background: var(--semantic-danger);
    color: white;
  }

  .confirm-btn-delete:hover {
    opacity: 0.9;
  }

  .add-zone-btn {
    width: 100%;
    padding: 6px;
    margin-top: 8px;
    font-size: 10px;
    font-family: var(--font-mono);
    background: var(--bg-elevated);
    border: 1px dashed var(--border-default);
    border-radius: 4px;
    color: var(--text-muted);
    cursor: pointer;
    transition: all 0.1s ease;
  }

  .add-zone-btn:hover {
    background: var(--bg-hover);
    color: var(--text-primary);
    border-style: solid;
  }

  /* Dialog */
  .dialog {
    margin-top: 8px;
    padding: 10px;
    background: var(--bg-elevated);
    border: 1px solid var(--border-default);
    border-radius: 6px;
  }

  .dialog-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 10px;
  }

  .dialog-title {
    font-size: 10px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .dialog-reset-btn {
    font-size: 9px;
    font-family: var(--font-mono);
    padding: 2px 6px;
    background: transparent;
    border: 1px solid var(--border-default);
    border-radius: 3px;
    color: var(--text-muted);
    cursor: pointer;
    transition: all 0.1s ease;
  }

  .dialog-reset-btn:hover {
    background: var(--bg-hover);
    color: var(--text-primary);
  }

  .dialog-field {
    margin-bottom: 8px;
  }

  .dialog-field label {
    display: block;
    font-size: 9px;
    color: var(--text-muted);
    margin-bottom: 3px;
  }

  .dialog-field input[type="text"] {
    width: 100%;
    padding: 5px 8px;
    font-size: 10px;
    font-family: var(--font-mono);
    background: var(--bg-surface);
    border: 1px solid var(--border-default);
    border-radius: 4px;
    color: var(--text-primary);
  }

  .color-input-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .dialog-field input[type="color"] {
    width: 28px;
    height: 28px;
    padding: 0;
    border: 1px solid var(--border-default);
    border-radius: 4px;
    cursor: pointer;
    background: transparent;
  }

  .color-hex-btn {
    font-family: var(--font-mono);
    font-size: 9px;
    color: var(--text-muted);
    background: transparent;
    border: none;
    cursor: pointer;
    padding: 2px 4px;
    border-radius: 2px;
    transition: all 0.1s ease;
  }

  .color-hex-btn:hover {
    background: var(--bg-hover);
    color: var(--text-primary);
  }

  .dialog-actions {
    display: flex;
    gap: 6px;
    margin-top: 10px;
  }

  .dialog-btn {
    flex: 1;
    padding: 6px;
    font-size: 10px;
    font-family: var(--font-mono);
    border-radius: 4px;
    cursor: pointer;
    border: 1px solid var(--border-default);
    transition: all 0.1s ease;
  }

  .dialog-btn-cancel {
    background: var(--bg-surface);
    color: var(--text-secondary);
  }

  .dialog-btn-cancel:hover {
    background: var(--bg-hover);
  }

  .dialog-btn-save {
    background: var(--text-primary);
    color: var(--bg-surface);
    border-color: var(--text-primary);
  }

  .dialog-btn-save:hover:not(:disabled) {
    opacity: 0.9;
  }

  .dialog-btn-save:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
</style>
