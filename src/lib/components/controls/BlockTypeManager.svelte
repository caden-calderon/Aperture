<script lang="ts">
  import { blockTypesStore, type BlockType } from '$lib/stores';
  import { contextStore, selectionStore, uiStore } from '$lib/stores';
  import { isBuiltInType, matchesDisplayType } from '$lib/utils/blockTypes';

  let expanded = $state(true);
  let showAddDialog = $state(false);
  let editingType = $state<BlockType | null>(null);
  let confirmingDeleteId = $state<string | null>(null);

  // Form state
  let newLabel = $state('');
  let newShortLabel = $state('');
  let newColor = $state('#808080');

  function formatNumber(n: number): string {
    return n.toLocaleString();
  }

  // Get token count for a block type
  function getTokenCount(typeId: string): number {
    if (isBuiltInType(typeId)) {
      return contextStore.tokenBudget.byRole[typeId];
    }
    return contextStore.blocks
      .filter((block) => matchesDisplayType(block, typeId))
      .reduce((sum, block) => sum + block.tokens, 0);
  }

  function handleAdd() {
    if (newLabel.trim() && newShortLabel.trim()) {
      blockTypesStore.addCustomType(newLabel.trim(), newShortLabel.trim(), newColor);
      resetForm();
      showAddDialog = false;
    }
  }

  function handleEdit(type: BlockType) {
    editingType = type;
    newLabel = type.label;
    newShortLabel = type.shortLabel;
    newColor = type.color;
  }

  function handleSaveEdit() {
    if (editingType && newLabel.trim() && newShortLabel.trim()) {
      blockTypesStore.updateCustomType(editingType.id, {
        label: newLabel.trim(),
        shortLabel: newShortLabel.trim(),
        color: newColor,
      });
      resetForm();
      editingType = null;
    }
  }

  function handleDeleteClick(id: string) {
    confirmingDeleteId = id;
  }

  function handleConfirmDelete() {
    if (confirmingDeleteId) {
      blockTypesStore.deleteCustomType(confirmingDeleteId);
      confirmingDeleteId = null;
    }
  }

  function handleCancelDelete() {
    confirmingDeleteId = null;
  }

  function resetForm() {
    newLabel = '';
    newShortLabel = '';
    newColor = '#808080';
  }

  function cancelDialog() {
    showAddDialog = false;
    editingType = null;
    resetForm();
  }

  // Click on type: assign selected blocks OR select all blocks of that type
  function handleTypeClick(typeId: string) {
    const label = blockTypesStore.getTypeById(typeId)?.label ?? typeId;

    if (selectionStore.hasSelection) {
      // Assign selected blocks to this display type
      contextStore.setBlocksType([...selectionStore.selectedIds], typeId);
      const count = selectionStore.count;
      uiStore.showToast(`Assigned ${count} block(s) to ${label}`, 'success');
    } else {
      // Select all blocks of this type
      selectionStore.selectByType(typeId);
      const count = selectionStore.count;
      if (count > 0) {
        uiStore.showToast(`Selected ${count} ${label} block(s)`, 'info');
      } else {
        uiStore.showToast(`No ${label} blocks found`, 'info');
      }
    }
  }

  // Drag from sidebar to create new block
  function handleDragStart(e: DragEvent, type: BlockType) {
    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = 'copy';
      e.dataTransfer.setData('application/x-block-type', JSON.stringify({
        action: 'create',
        typeId: type.id,
        label: type.label,
      }));
    }
  }
</script>

<div class="block-type-manager">
  <button class="section-header" onclick={() => expanded = !expanded}>
    <span class="section-title">Block Types</span>
    <span class="section-toggle">{expanded ? '−' : '+'}</span>
  </button>

  {#if expanded}
    <div class="section-content">
      <!-- Built-in types -->
      <div class="type-list">
        {#each blockTypesStore.builtInTypes as type (type.id)}
          <button
            class="type-item"
            class:has-selection={selectionStore.hasSelection}
            onclick={() => handleTypeClick(type.id)}
            draggable="true"
            ondragstart={(e) => handleDragStart(e, type)}
            title={selectionStore.hasSelection ? `Click to assign ${selectionStore.count} block(s)` : 'Drag to canvas to create block'}
          >
            <span class="type-dot" style:background={type.color}></span>
            <span class="type-label">{type.label}</span>
            <span class="type-badge">{type.shortLabel}</span>
            <span class="type-count">{formatNumber(getTokenCount(type.id))}</span>
          </button>
        {/each}
      </div>

      <!-- Custom types -->
      {#if blockTypesStore.customTypes.length > 0}
        <div class="custom-divider">Custom</div>
        <div class="type-list">
          {#each blockTypesStore.customTypes as type (type.id)}
            {#if confirmingDeleteId === type.id}
              <!-- Inline delete confirmation -->
              <div class="delete-confirm">
                <span class="delete-confirm-text">Delete "{type.label}"?</span>
                <div class="delete-confirm-actions">
                  <button class="confirm-btn confirm-btn-cancel" onclick={handleCancelDelete}>No</button>
                  <button class="confirm-btn confirm-btn-delete" onclick={handleConfirmDelete}>Yes</button>
                </div>
              </div>
            {:else}
              <div class="type-item type-item-custom">
                <button
                  class="type-item-main"
                  class:has-selection={selectionStore.hasSelection}
                  onclick={() => handleTypeClick(type.id)}
                  draggable="true"
                  ondragstart={(e) => handleDragStart(e, type)}
                  title={selectionStore.hasSelection ? `Click to assign ${selectionStore.count} block(s)` : 'Drag to canvas to create block'}
                >
                  <span class="type-dot" style:background={type.color}></span>
                  <span class="type-label">{type.label}</span>
                  <span class="type-badge">{type.shortLabel}</span>
                </button>
                <div class="type-actions">
                  <button class="type-action-btn" onclick={() => handleEdit(type)} title="Edit" aria-label="Edit">
                    <span>...</span>
                  </button>
                  <button class="type-action-btn type-action-delete" onclick={() => handleDeleteClick(type.id)} title="Delete" aria-label="Delete">
                    <span>×</span>
                  </button>
                </div>
              </div>
            {/if}
          {/each}
        </div>
      {/if}

      <!-- Add button -->
      <button class="add-type-btn" onclick={() => showAddDialog = true}>
        + Add Custom Type
      </button>

      <!-- Add/Edit Dialog -->
      {#if showAddDialog || editingType}
        <div class="dialog">
          <div class="dialog-title">{editingType ? 'Edit Type' : 'New Block Type'}</div>
          <div class="dialog-field">
            <label for="type-label">Label</label>
            <input
              id="type-label"
              type="text"
              placeholder="e.g. Memory"
              bind:value={newLabel}
            />
          </div>
          <div class="dialog-field">
            <label for="type-short">Short (4 chars max)</label>
            <input
              id="type-short"
              type="text"
              placeholder="e.g. MEM"
              maxlength="4"
              bind:value={newShortLabel}
            />
          </div>
          <div class="dialog-field">
            <label for="type-color">Color</label>
            <div class="color-input-row">
              <input
                id="type-color"
                type="color"
                bind:value={newColor}
              />
              <span class="color-preview" style:background={newColor}></span>
              <span class="color-value">{newColor}</span>
            </div>
          </div>
          <div class="dialog-actions">
            <button class="dialog-btn dialog-btn-cancel" onclick={cancelDialog}>Cancel</button>
            <button
              class="dialog-btn dialog-btn-save"
              onclick={editingType ? handleSaveEdit : handleAdd}
              disabled={!newLabel.trim() || !newShortLabel.trim()}
            >
              {editingType ? 'Save' : 'Add'}
            </button>
          </div>
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .block-type-manager {
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
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .type-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .type-item,
  .type-item-main {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 6px;
    font-size: 11px;
    background: transparent;
    border: none;
    border-radius: 3px;
    cursor: grab;
    transition: background 0.1s ease;
    text-align: left;
    width: 100%;
    min-width: 0;
  }

  .type-item:hover,
  .type-item-main:hover {
    background: var(--bg-hover);
  }

  .type-item.has-selection,
  .type-item-main.has-selection {
    cursor: pointer;
  }

  .type-item.has-selection:hover,
  .type-item-main.has-selection:hover {
    background: var(--accent-subtle);
  }

  .type-item-custom {
    display: flex;
    align-items: center;
    padding: 0;
    background: transparent;
  }

  .type-item-custom:hover .type-actions {
    opacity: 1;
  }

  .type-dot {
    width: 6px;
    height: 6px;
    border-radius: 1px;
    flex-shrink: 0;
  }

  .type-label {
    color: var(--text-secondary);
    flex: 1;
    font-size: 10px;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .type-badge {
    font-family: var(--font-mono);
    font-size: 8px;
    padding: 1px 3px;
    background: var(--bg-muted);
    border-radius: 2px;
    color: var(--text-muted);
    flex-shrink: 0;
  }

  .type-count {
    font-family: var(--font-mono);
    font-size: 9px;
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
    min-width: 28px;
    text-align: right;
    flex-shrink: 0;
  }

  .type-actions {
    display: flex;
    gap: 2px;
    opacity: 0;
    transition: opacity 0.1s ease;
    margin-left: auto;
  }

  .type-action-btn {
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

  .type-action-btn:hover {
    background: var(--bg-hover);
    color: var(--text-primary);
  }

  .type-action-delete:hover {
    background: var(--semantic-danger);
    color: white;
    border-color: var(--semantic-danger);
  }

  .custom-divider {
    font-size: 8px;
    text-transform: uppercase;
    letter-spacing: 0.4px;
    color: var(--text-faint);
    padding-top: 4px;
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

  .add-type-btn {
    width: 100%;
    padding: 6px;
    font-size: 10px;
    font-family: var(--font-mono);
    background: var(--bg-elevated);
    border: 1px dashed var(--border-default);
    border-radius: 4px;
    color: var(--text-muted);
    cursor: pointer;
    transition: all 0.1s ease;
  }

  .add-type-btn:hover {
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

  .dialog-title {
    font-size: 10px;
    font-weight: 600;
    color: var(--text-primary);
    margin-bottom: 10px;
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

  .color-preview {
    width: 20px;
    height: 20px;
    border-radius: 3px;
    border: 1px solid var(--border-subtle);
  }

  .color-value {
    font-family: var(--font-mono);
    font-size: 9px;
    color: var(--text-muted);
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
