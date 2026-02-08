<script lang="ts">
  import type { Block, Role, Snapshot } from "$lib/types";
  import { blockTypesStore, zonesStore, editHistoryStore } from "$lib/stores";
  import ModalActionPanel from "./ModalActionPanel.svelte";
  import ModalTypeDropdown from "./ModalTypeDropdown.svelte";
  import { focusTrap } from "$lib/utils";
  import { detectLanguage, highlightCode } from "$lib/utils/syntax";
  import { resolveTypeSelection } from "$lib/utils/blockTypes";
  import { getPreview } from "$lib/utils/text";
  import { diffLines, type DiffLine } from "$lib/utils/diff";

  interface Props {
    block: Block | null;
    open?: boolean;
    onClose?: () => void;
    onCompress?: (level: Block["compressionLevel"]) => void;
    onMove?: (zone: Block["zone"]) => void;
    onPin?: (position: Block["pinned"]) => void;
    onRemove?: () => void;
    onRoleChange?: (role: Role, blockType?: string) => void;
    onContentEdit?: (content: string) => void;
    onOpenDiff?: (filterZone?: string, highlightBlockId?: string) => void;
    onRevert?: (blockId: string, content: string) => void;
    snapshots?: Snapshot[];
  }

  let {
    block,
    open = false,
    onClose,
    onCompress,
    onMove,
    onPin,
    onRemove,
    onRoleChange,
    onContentEdit,
    onOpenDiff,
    onRevert,
    snapshots = [],
  }: Props = $props();

  let roleDropdownOpen = $state(false);
  let isEditing = $state(false);
  let isContentExpanded = $state(false);
  let editContent = $state("");
  let textareaRef = $state<HTMLTextAreaElement | null>(null);
  let diffExpanded = $state(false);
  let diffVersionIndex = $state(0);
  let diffSnapshotFilter = $state<string | null>(null); // null = all/edits, string = specific snapshot id
  let pendingRevert = $state<{ blockId: string; content: string } | null>(null);

  // Get zone label for display
  const zoneLabel = $derived.by(() => {
    if (!block) return '';
    const zone = zonesStore.getZoneById(block.zone);
    return zone?.label ?? block.zone;
  });

  // Get display info for current block
  const displayTypeId = $derived(block?.blockType ?? block?.role ?? "user");
  const typeInfo = $derived(blockTypesStore.getTypeById(displayTypeId));
  const displayLabel = $derived(typeInfo?.label ?? displayTypeId);

  // Track if content is long enough to need expand
  const isContentLong = $derived(block ? block.content.length > 400 : false);

  // Snapshot diff: compare block against latest snapshot
  const snapshotDiff = $derived.by(() => {
    if (!block || snapshots.length === 0) return null;
    const latest = snapshots[snapshots.length - 1];
    const snapBlock = latest.blocks.find((b) => b.id === block.id);
    if (!snapBlock) return { status: "new" as const, changes: [] as string[], snapshotName: latest.name };
    const changes: string[] = [];
    if (block.content !== snapBlock.content) changes.push("content");
    if (block.zone !== snapBlock.zone) {
      const from = zonesStore.getZoneById(snapBlock.zone)?.label ?? snapBlock.zone;
      const to = zonesStore.getZoneById(block.zone)?.label ?? block.zone;
      changes.push(`zone: ${from} → ${to}`);
    }
    if (block.compressionLevel !== snapBlock.compressionLevel) {
      changes.push(`compression: ${snapBlock.compressionLevel} → ${block.compressionLevel}`);
    }
    if (block.tokens !== snapBlock.tokens) {
      const delta = block.tokens - snapBlock.tokens;
      changes.push(`tokens: ${delta > 0 ? "+" : ""}${delta}`);
    }
    if (block.pinned !== snapBlock.pinned) changes.push("pin changed");
    if (block.role !== snapBlock.role) changes.push(`role: ${snapBlock.role} → ${block.role}`);
    if (changes.length === 0) return { status: "unchanged" as const, changes: [], snapshotName: latest.name };
    return { status: "modified" as const, changes, snapshotName: latest.name };
  });

  // Syntax highlighting for modal content (works in both view and edit modes)
  const modalDetectedLang = $derived(block ? detectLanguage(block.content, block.role) : null);
  const modalSyntaxHtml = $derived.by(() => {
    if (!block || !modalDetectedLang) return null;
    const text = isEditing ? editContent : (isContentExpanded ? block.content : getPreview(block.content, 400));
    return highlightCode(text, modalDetectedLang);
  });

  // Combined version timeline: edit history + snapshot versions
  interface VersionEntry {
    content: string;
    label: string;
    source: "edit" | "snapshot";
    snapshotId: string | null; // null for edit history entries
    timestamp: number;
  }

  // All versions (unfiltered)
  const allVersions = $derived.by((): VersionEntry[] => {
    if (!block) return [];
    const versions: VersionEntry[] = [];

    // From edit history (content changes only)
    const history = editHistoryStore.getBlockHistory(block.id);
    for (const entry of history) {
      if (entry.type === "content" && entry.before.content != null) {
        versions.push({
          content: entry.before.content,
          label: "Edit",
          source: "edit",
          snapshotId: null,
          timestamp: entry.timestamp,
        });
      }
    }

    // From snapshots
    for (const snap of snapshots) {
      const snapBlock = snap.blocks.find((b) => b.id === block.id);
      if (snapBlock) {
        versions.push({
          content: snapBlock.content,
          label: snap.name,
          source: "snapshot",
          snapshotId: snap.id,
          timestamp: new Date(snap.timestamp).getTime(),
        });
      }
    }

    // Sort newest first, deduplicate adjacent identical content
    versions.sort((a, b) => b.timestamp - a.timestamp);
    const deduped: VersionEntry[] = [];
    let lastContent: string | null = null;
    for (const v of versions) {
      if (v.content !== lastContent) {
        deduped.push(v);
        lastContent = v.content;
      }
    }
    return deduped;
  });

  // Snapshots that have versions for this block (for snapshot dropdown)
  const availableSnapshotIds = $derived.by((): { id: string; name: string }[] => {
    const seen = new Map<string, string>();
    for (const v of allVersions) {
      if (v.snapshotId && !seen.has(v.snapshotId)) {
        seen.set(v.snapshotId, v.label);
      }
    }
    return Array.from(seen, ([id, name]) => ({ id, name }));
  });

  // Filtered timeline: when snapshot is selected, only show that snapshot's versions
  const versionTimeline = $derived.by((): VersionEntry[] => {
    if (!diffSnapshotFilter) return allVersions;
    return allVersions.filter(v => v.snapshotId === diffSnapshotFilter);
  });

  // Current snapshot filter label
  const diffSnapshotLabel = $derived.by(() => {
    if (!diffSnapshotFilter) return "All";
    const snap = availableSnapshotIds.find(s => s.id === diffSnapshotFilter);
    return snap?.name ?? "Unknown";
  });

  const hasVersions = $derived(allVersions.length > 0);
  const hasEditHistory = $derived(block ? editHistoryStore.getBlockHistory(block.id).length > 0 : false);

  // Current diff comparison (uses pending revert content if active)
  const diffVersion = $derived(
    versionTimeline.length > 0 && diffVersionIndex < versionTimeline.length
      ? versionTimeline[diffVersionIndex]
      : null
  );

  // The "current" content for diff — if a revert is pending, show the reverted content on the left
  const effectiveBlockContent = $derived(
    pendingRevert ? pendingRevert.content : (block?.content ?? "")
  );

  const diffResult = $derived.by((): DiffLine[] => {
    if (!block || !diffVersion || !diffExpanded) return [];
    return diffLines(diffVersion.content, effectiveBlockContent);
  });

  // Reset state when modal opens/closes or block changes
  $effect(() => {
    if (!open) {
      // Apply pending revert on close
      if (pendingRevert && onRevert) {
        onRevert(pendingRevert.blockId, pendingRevert.content);
      }
      isEditing = false;
      isContentExpanded = false;
      editContent = "";
      roleDropdownOpen = false;
      diffExpanded = false;
      diffVersionIndex = 0;
      diffSnapshotFilter = null;
      pendingRevert = null;
    }
  });

  function enterEditMode() {
    if (!block) return;
    editContent = block.content;
    isEditing = true;
    isContentExpanded = true;
    // Focus textarea after render
    requestAnimationFrame(() => {
      textareaRef?.focus();
      // Move cursor to end
      if (textareaRef) {
        textareaRef.selectionStart = textareaRef.value.length;
        textareaRef.selectionEnd = textareaRef.value.length;
      }
    });
  }

  function saveEdit() {
    if (!block || editContent === block.content) {
      isEditing = false;
      return;
    }
    onContentEdit?.(editContent);
    isEditing = false;
  }

  function cancelEdit() {
    isEditing = false;
    editContent = "";
  }

  function handleContentDoubleClick() {
    if (!isEditing) {
      enterEditMode();
    }
  }

  function handleEditKeydown(e: KeyboardEvent) {
    // Ctrl/Cmd+Enter to save
    if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      saveEdit();
    }
    // Escape to cancel
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      cancelEdit();
    }
  }

  function handleTypeSelect(typeId: string) {
    if (!block) return;
    const { role, blockType } = resolveTypeSelection(typeId, block.role);
    onRoleChange?.(role, blockType);
    roleDropdownOpen = false;
  }

  const roleColors: Record<string, string> = {
    system: "var(--role-system)",
    user: "var(--role-user)",
    assistant: "var(--role-assistant)",
    tool_use: "var(--role-tool)",
    tool_result: "var(--role-tool)",
    thinking: "var(--role-thinking)",
  };

  function handleBackdropClick(e: MouseEvent) {
    if (e.target === e.currentTarget) {
      if (isEditing) {
        cancelEdit();
      }
      onClose?.();
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape" && !isEditing) onClose?.();
  }

  function formatDate(date: Date): string {
    return date.toLocaleString("en-US", {
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    });
  }

</script>

<svelte:window onkeydown={handleKeydown} />

{#if open && block}
  <div
    class="modal-backdrop"
    role="dialog"
    aria-modal="true"
    onclick={handleBackdropClick}
    onkeydown={handleKeydown}
    tabindex="-1"
  >
    <div
      class="modal"
      class:expanded={isContentExpanded}
      class:diff-expanded={diffExpanded}
      style:--role-color={roleColors[block.role]}
      use:focusTrap={{ enabled: open, onEscape: onClose }}
    >
      <div class="modal-header">
        <div class="modal-title">
          <ModalTypeDropdown
            {displayLabel}
            {displayTypeId}
            open={roleDropdownOpen}
            onToggle={() => roleDropdownOpen = !roleDropdownOpen}
            onSelect={handleTypeSelect}
          />
          {#if block.metadata.toolName}
            <span class="tool-name">{block.metadata.toolName}</span>
          {/if}
          {#if modalDetectedLang}
            <span class="lang-badge">{modalDetectedLang}</span>
          {/if}
        </div>
        <button class="close-btn" onclick={onClose}>×</button>
      </div>

      <div class="modal-meta">
        <div class="meta-item">
          <span class="meta-label">Tokens</span>
          <span class="meta-value">{block.tokens.toLocaleString()}</span>
        </div>
        <div class="meta-item">
          <span class="meta-label">Zone</span>
          <span class="meta-value">{zoneLabel}</span>
        </div>
        <div class="meta-item">
          <span class="meta-label">Compression</span>
          <span class="meta-value">{block.compressionLevel}</span>
        </div>
        <div class="meta-item">
          <span class="meta-label">Created</span>
          <span class="meta-value">{formatDate(block.timestamp)}</span>
        </div>
      </div>

      <div class="modal-content-header">
        <div class="content-header-left">
          {#if isEditing}
            <span class="editing-indicator">Editing</span>
            <span class="edit-hint">Ctrl+Enter to save · Esc to cancel</span>
          {:else if isContentLong}
            <button
              class="expand-btn"
              onclick={() => isContentExpanded = !isContentExpanded}
            >
              {isContentExpanded ? "Collapse" : "Expand"} content
            </button>
          {/if}
        </div>
        <div class="content-header-right">
          {#if isEditing}
            <button class="content-action-btn save-btn" onclick={saveEdit}>Save</button>
            <button class="content-action-btn cancel-btn" onclick={cancelEdit}>Cancel</button>
          {:else}
            <button
              class="content-action-btn edit-btn"
              onclick={enterEditMode}
              title="Edit content (or double-click)"
            >
              ✎ Edit
            </button>
          {/if}
        </div>
      </div>

      <div class="modal-content-area" class:side-by-side={diffExpanded}>
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div
          class="modal-content"
          class:editing={isEditing}
          ondblclick={handleContentDoubleClick}
        >
          {#if isEditing}
            <div class="edit-overlay-container">
              {#if modalSyntaxHtml}
                <!-- Safe invariant: highlightCode() returns Prism-escaped HTML. -->
                <!-- eslint-disable-next-line svelte/no-at-html-tags -->
                <pre class="edit-highlight-layer syntax-highlighted" aria-hidden="true">{@html modalSyntaxHtml}
</pre>
              {:else}
                <pre class="edit-highlight-layer" aria-hidden="true">{editContent}
</pre>
              {/if}
              <textarea
                bind:this={textareaRef}
                bind:value={editContent}
                class="edit-textarea-layer"
                class:has-highlight={!!modalSyntaxHtml}
                spellcheck="true"
                lang="en"
                autocomplete="off"
                onkeydown={handleEditKeydown}
                onscroll={(e) => {
                  const target = e.currentTarget;
                  const pre = target.previousElementSibling as HTMLElement;
                  if (pre) {
                    pre.scrollTop = target.scrollTop;
                    pre.scrollLeft = target.scrollLeft;
                  }
                }}
              ></textarea>
            </div>
          {:else if modalSyntaxHtml}
            <!-- Safe invariant: highlightCode() returns Prism-escaped HTML. -->
            <!-- eslint-disable-next-line svelte/no-at-html-tags -->
            <pre class="syntax-highlighted">{@html modalSyntaxHtml}</pre>
            {#if !isContentExpanded && isContentLong}
              <div class="content-fade"></div>
            {/if}
          {:else}
            <pre>{isContentExpanded ? block.content : getPreview(block.content, 400)}</pre>
            {#if !isContentExpanded && isContentLong}
              <div class="content-fade"></div>
            {/if}
          {/if}
        </div>

        {#if diffExpanded && diffVersion}
          <div class="diff-divider"></div>
          <div class="diff-panel">
            <div class="diff-panel-header">
              <div class="diff-version-info">
                <span class="diff-version-label">{diffVersion.label}</span>
                <span class="diff-source-badge diff-source-{diffVersion.source}">{diffVersion.source}</span>
              </div>
              <div class="diff-nav">
                <!-- Snapshot filter dropdown -->
                <select
                  class="diff-snapshot-select"
                  value={diffSnapshotFilter ?? ""}
                  onchange={(e) => {
                    const val = (e.currentTarget as HTMLSelectElement).value;
                    diffSnapshotFilter = val || null;
                    diffVersionIndex = 0;
                  }}
                >
                  <option value="">All</option>
                  {#each availableSnapshotIds as snap (snap.id)}
                    <option value={snap.id}>{snap.name}</option>
                  {/each}
                </select>
                <button
                  class="diff-nav-btn"
                  disabled={diffVersionIndex <= 0}
                  onclick={() => diffVersionIndex = Math.max(diffVersionIndex - 1, 0)}
                  title="Newer version"
                >◀</button>
                <span class="diff-nav-label">
                  {diffVersionIndex + 1} / {versionTimeline.length}
                </span>
                <button
                  class="diff-nav-btn"
                  disabled={diffVersionIndex >= versionTimeline.length - 1}
                  onclick={() => diffVersionIndex = Math.min(diffVersionIndex + 1, versionTimeline.length - 1)}
                  title="Older version"
                >▶</button>
              </div>
            </div>
            {#if diffSnapshotFilter}
              <div class="diff-snapshot-indicator">
                Viewing: <strong>{diffSnapshotLabel}</strong>
              </div>
            {/if}
            <div class="diff-panel-content">
              {#each diffResult as line}
                <div class="modal-diff-line modal-diff-line-{line.type}">
                  <span class="modal-diff-marker">{line.type === "added" ? "+" : line.type === "removed" ? "-" : " "}</span>
                  <span class="modal-diff-text">{line.content || " "}</span>
                </div>
              {/each}
            </div>
            {#if block}
              <div class="diff-panel-footer">
                {#if pendingRevert}
                  <button
                    class="revert-btn revert-undo"
                    onclick={() => pendingRevert = null}
                  >Undo Revert</button>
                {:else}
                  <button
                    class="revert-btn"
                    onclick={() => {
                      if (block && diffVersion) {
                        pendingRevert = { blockId: block.id, content: diffVersion.content };
                      }
                    }}
                  >Revert to this version</button>
                {/if}
              </div>
            {/if}
          </div>
        {/if}
      </div>

      <ModalActionPanel
        {block}
        {zoneLabel}
        {snapshotDiff}
        {hasEditHistory}
        versionCount={versionTimeline.length}
        {hasVersions}
        {isEditing}
        {diffExpanded}
        onToggleDiff={() => {
          diffExpanded = !diffExpanded;
          diffVersionIndex = 0;
        }}
        {onMove}
        {onCompress}
        {onPin}
        {onOpenDiff}
        {onRemove}
      />
    </div>
  </div>
{/if}

<style>
  .modal-backdrop {
    position: fixed;
    inset: 0;
    background: color-mix(in srgb, var(--bg-base) 85%, transparent);
    backdrop-filter: blur(4px);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
    animation: modal-fade-in 0.15s ease;
  }

  @keyframes modal-fade-in {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  @keyframes modal-slide-in {
    from {
      opacity: 0;
      transform: translateY(8px) scale(0.98);
    }
    to {
      opacity: 1;
      transform: translateY(0) scale(1);
    }
  }

  .modal {
    background: var(--bg-surface);
    border: 1px solid var(--border-default);
    border-radius: 8px;
    width: 90%;
    max-width: 560px;
    max-height: 75vh;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    box-shadow: var(--shadow-lg);
    animation: modal-slide-in 0.2s cubic-bezier(0.34, 1.2, 0.64, 1);
    transition: max-height 0.2s ease, max-width 0.2s ease;
  }

  .modal.expanded {
    max-height: 90vh;
    max-width: 700px;
  }

  .modal-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 14px;
    border-bottom: 1px solid var(--border-subtle);
  }

  .modal-title {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .tool-name {
    font-size: 11px;
    color: var(--text-muted);
  }

  .lang-badge {
    font-family: var(--font-mono);
    font-size: 9px;
    font-weight: 600;
    padding: 3px 6px;
    border-radius: 2px;
    background: color-mix(in srgb, var(--role-user) 15%, transparent);
    color: var(--role-user);
    letter-spacing: 0.3px;
    text-transform: lowercase;
  }

  .close-btn {
    font-size: 18px;
    line-height: 1;
    color: var(--text-muted);
    background: none;
    border: none;
    cursor: pointer;
    padding: 4px;
    border-radius: 4px;
    opacity: 0.6;
    transition: all 0.1s ease;
  }

  .close-btn:hover {
    background: var(--bg-hover);
    color: var(--text-primary);
    opacity: 1;
  }

  .modal-meta {
    display: flex;
    flex-wrap: wrap;
    gap: 16px;
    padding: 10px 14px;
    border-bottom: 1px solid var(--border-subtle);
    background: var(--bg-inset);
  }

  .meta-item {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .meta-label {
    font-size: 9px;
    text-transform: uppercase;
    color: var(--text-faint);
    letter-spacing: 0.3px;
  }

  .meta-value {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--text-primary);
    font-variant-numeric: tabular-nums;
  }

  /* Content header bar */
  .modal-content-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 6px 14px;
    border-bottom: 1px solid var(--border-subtle);
    background: var(--bg-surface);
    min-height: 32px;
  }

  .content-header-left {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .content-header-right {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .editing-indicator {
    font-family: var(--font-mono);
    font-size: 9px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.3px;
    color: var(--accent);
    padding: 2px 6px;
    background: var(--accent-subtle);
    border-radius: 2px;
  }

  .edit-hint {
    font-size: 9px;
    color: var(--text-faint);
  }

  .expand-btn {
    font-family: var(--font-mono);
    font-size: 9px;
    padding: 2px 6px;
    border-radius: 2px;
    border: 1px solid var(--border-default);
    background: var(--bg-surface);
    color: var(--text-muted);
    cursor: pointer;
    transition: all 0.1s ease;
  }

  .expand-btn:hover {
    background: var(--bg-hover);
    color: var(--text-primary);
    border-color: var(--border-strong);
  }

  .content-action-btn {
    font-family: var(--font-mono);
    font-size: 9px;
    padding: 3px 8px;
    border-radius: 3px;
    cursor: pointer;
    transition: all 0.1s ease;
  }

  .edit-btn {
    border: 1px solid var(--border-default);
    background: var(--bg-surface);
    color: var(--text-muted);
  }

  .edit-btn:hover {
    background: var(--bg-hover);
    color: var(--text-primary);
    border-color: var(--border-strong);
  }

  .save-btn {
    border: 1px solid var(--accent);
    background: var(--accent);
    color: var(--bg-surface);
    font-weight: 600;
  }

  .save-btn:hover {
    opacity: 0.85;
  }

  .cancel-btn {
    border: 1px solid var(--border-default);
    background: var(--bg-surface);
    color: var(--text-muted);
  }

  .cancel-btn:hover {
    background: var(--bg-hover);
    color: var(--text-primary);
  }

  /* Content area */
  .modal-content {
    flex: 1;
    overflow: auto;
    padding: 14px;
    background: var(--bg-elevated);
    position: relative;
    cursor: text;
    min-height: 80px;
  }

  .modal-content pre {
    font-family: var(--font-mono);
    font-size: 11px;
    line-height: 1.55;
    color: var(--text-secondary);
    white-space: pre-wrap;
    word-break: break-word;
    margin: 0;
  }

  .content-fade {
    position: absolute;
    bottom: 0;
    left: 0;
    right: 0;
    height: 30px;
    background: linear-gradient(transparent, var(--bg-elevated));
    pointer-events: none;
  }

  .modal-content.editing {
    padding: 0;
  }

  /* Overlay edit pattern: highlighted pre behind transparent textarea */
  .edit-overlay-container {
    position: relative;
    width: 100%;
    min-height: 200px;
  }

  .edit-highlight-layer {
    font-family: var(--font-mono);
    font-size: 11px;
    line-height: 1.55;
    color: var(--text-secondary);
    white-space: pre-wrap;
    word-break: break-word;
    margin: 0;
    padding: 14px;
    pointer-events: none;
    overflow: hidden;
  }

  .edit-textarea-layer {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    min-height: 200px;
    padding: 14px;
    font-family: var(--font-mono);
    font-size: 11px;
    line-height: 1.55;
    color: var(--text-secondary);
    background: transparent;
    border: none;
    outline: none;
    resize: none;
    white-space: pre-wrap;
    word-break: break-word;
    caret-color: var(--text-primary);
  }

  /* When syntax highlighting is active, hide textarea text — the pre shows colors */
  .edit-textarea-layer.has-highlight {
    color: transparent;
  }

  .edit-textarea-layer:focus {
    box-shadow: inset 0 0 0 1px var(--accent);
  }

  /* Diff expanded modal */
  .modal.diff-expanded {
    max-width: 1000px;
    max-height: 90vh;
  }

  /* Side-by-side content area */
  .modal-content-area {
    flex: 1;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }

  .modal-content-area.side-by-side {
    flex-direction: row;
  }

  .modal-content-area.side-by-side .modal-content {
    flex: 1;
    min-width: 0;
  }

  .diff-divider {
    width: 1px;
    background: var(--border-default);
    flex-shrink: 0;
  }

  /* Diff panel */
  .diff-panel {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg-base);
    overflow: hidden;
  }

  .diff-panel-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 6px 10px;
    border-bottom: 1px solid var(--border-subtle);
    background: var(--bg-surface);
    flex-shrink: 0;
  }

  .diff-version-info {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .diff-version-label {
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--text-secondary);
    font-weight: 600;
  }

  .diff-source-badge {
    font-family: var(--font-mono);
    font-size: 8px;
    padding: 1px 4px;
    border-radius: 2px;
    font-weight: 600;
    letter-spacing: 0.2px;
  }

  .diff-source-edit {
    color: var(--semantic-warning);
    background: color-mix(in srgb, var(--semantic-warning) 15%, transparent);
  }

  .diff-source-snapshot {
    color: var(--accent);
    background: var(--accent-subtle);
  }

  .diff-nav {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .diff-nav-btn {
    font-family: var(--font-mono);
    font-size: 10px;
    padding: 2px 6px;
    border: 1px solid var(--border-default);
    border-radius: 2px;
    background: var(--bg-surface);
    color: var(--text-muted);
    cursor: pointer;
    transition: all 0.1s ease;
  }

  .diff-nav-btn:hover:not(:disabled) {
    background: var(--bg-hover);
    color: var(--text-primary);
    border-color: var(--border-strong);
  }

  .diff-nav-btn:disabled {
    opacity: 0.3;
    cursor: not-allowed;
  }

  .diff-nav-label {
    font-family: var(--font-mono);
    font-size: 9px;
    color: var(--text-muted);
    white-space: nowrap;
    min-width: 60px;
    text-align: center;
  }

  .diff-panel-content {
    flex: 1;
    overflow-y: auto;
    padding: 8px 0;
    scrollbar-width: thin;
    scrollbar-color: var(--border-default) transparent;
  }

  .modal-diff-line {
    display: flex;
    font-family: var(--font-mono);
    font-size: 10px;
    line-height: 1.55;
    min-height: 18px;
    padding: 0 8px;
  }

  .modal-diff-line-added {
    background: color-mix(in srgb, var(--semantic-success) 10%, transparent);
  }

  .modal-diff-line-removed {
    background: color-mix(in srgb, var(--semantic-danger) 10%, transparent);
    opacity: 0.7;
  }

  .modal-diff-line-unchanged {
    color: var(--text-muted);
  }

  .modal-diff-marker {
    width: 14px;
    flex-shrink: 0;
    text-align: center;
    font-weight: 700;
    user-select: none;
  }

  .modal-diff-line-added .modal-diff-marker { color: var(--semantic-success); }
  .modal-diff-line-removed .modal-diff-marker { color: var(--semantic-danger); }
  .modal-diff-line-unchanged .modal-diff-marker { color: var(--text-faint); }

  .modal-diff-text {
    flex: 1;
    white-space: pre-wrap;
    word-break: break-word;
  }

  .diff-panel-footer {
    padding: 8px 10px;
    border-top: 1px solid var(--border-subtle);
    background: var(--bg-surface);
    flex-shrink: 0;
  }

  .revert-btn {
    font-family: var(--font-mono);
    font-size: 10px;
    padding: 4px 10px;
    border-radius: 3px;
    border: 1px solid color-mix(in srgb, var(--semantic-warning) 50%, transparent);
    background: transparent;
    color: var(--semantic-warning);
    cursor: pointer;
    transition: all 0.1s ease;
    width: 100%;
  }

  .revert-btn:hover {
    background: var(--semantic-warning);
    border-color: var(--semantic-warning);
    color: var(--bg-surface);
  }

  .revert-btn.revert-undo {
    border-color: color-mix(in srgb, var(--semantic-danger) 50%, transparent);
    color: var(--semantic-danger);
  }

  .revert-btn.revert-undo:hover {
    background: var(--semantic-danger);
    border-color: var(--semantic-danger);
    color: var(--bg-surface);
  }

  /* Snapshot filter dropdown in diff nav */
  .diff-snapshot-select {
    font-family: var(--font-mono);
    font-size: 9px;
    padding: 2px 4px;
    background: var(--bg-base);
    border: 1px solid var(--border-default);
    border-radius: 2px;
    color: var(--text-secondary);
    cursor: pointer;
    color-scheme: dark;
    max-width: 100px;
  }

  .diff-snapshot-select:focus {
    outline: none;
    border-color: var(--accent);
  }

  .diff-snapshot-indicator {
    padding: 3px 10px;
    font-family: var(--font-mono);
    font-size: 9px;
    color: var(--text-muted);
    background: color-mix(in srgb, var(--accent) 8%, var(--bg-surface));
    border-bottom: 1px solid var(--border-subtle);
  }

  .diff-snapshot-indicator strong {
    color: var(--text-primary);
  }
</style>
