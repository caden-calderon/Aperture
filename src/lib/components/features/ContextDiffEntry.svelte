<script lang="ts">
  import type { Block } from "$lib/types";
  import { blockTypesStore } from "$lib/stores";
  import { getPreview } from "$lib/utils/text";
  import type { DiffLine } from "$lib/utils/diff";

  interface ContextDiffEntryModel {
    status: "added" | "removed" | "modified" | "unchanged";
    current: Block | null;
    snapshot: Block | null;
    changes: string[];
    hasContentChange: boolean;
  }

  interface PendingRevert {
    revertContent: string;
    originalContent: string;
  }

  interface Props {
    entry: ContextDiffEntryModel;
    entryId: string;
    highlighted?: boolean;
    expanded?: boolean;
    inlineDiffLines?: DiffLine[];
    pendingRevert?: PendingRevert;
    onOpenBlock?: (blockId: string) => void;
    onToggleInlineDiff?: (entryId: string) => void;
    onSetPendingRevert?: (entryId: string, payload: PendingRevert | null) => void;
  }

  let {
    entry,
    entryId,
    highlighted = false,
    expanded = false,
    inlineDiffLines = undefined,
    pendingRevert = undefined,
    onOpenBlock,
    onToggleInlineDiff,
    onSetPendingRevert,
  }: Props = $props();

  const block = $derived(entry.current ?? entry.snapshot);
  const preview = $derived.by(() => {
    if (!block) return "";
    return getPreview(block.content.replace(/\n/g, " "), 60);
  });
  const roleLabel = $derived.by(() => {
    if (!block) return "";
    const typeId = block.blockType ?? block.role;
    const typeInfo = blockTypesStore.getTypeById(typeId);
    return typeInfo?.shortLabel ?? block.role.toUpperCase();
  });
  const roleColor = $derived.by(() => {
    if (!block) return "var(--text-muted)";
    const roleColors: Record<string, string> = {
      system: "var(--role-system)",
      user: "var(--role-user)",
      assistant: "var(--role-assistant)",
      tool_use: "var(--role-tool)",
      tool_result: "var(--role-tool)",
      thinking: "var(--role-thinking)",
    };
    return roleColors[block.role] ?? "var(--text-muted)";
  });

  function openEntry() {
    if (!block) return;
    onOpenBlock?.(block.id);
  }

  function handleEntryKeydown(e: KeyboardEvent) {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      openEntry();
    }
  }

  function handleToggleInlineDiff(e: MouseEvent) {
    e.stopPropagation();
    onToggleInlineDiff?.(entryId);
  }

  function setPendingRevert(payload: PendingRevert | null) {
    onSetPendingRevert?.(entryId, payload);
  }

  function getDiffMarker(type: DiffLine["type"]): string {
    if (type === "added") return "+";
    if (type === "removed") return "-";
    return " ";
  }
</script>

{#if block}
  <div
    class="diff-entry"
    class:diff-added={entry.status === "added"}
    class:diff-removed={entry.status === "removed"}
    class:diff-modified={entry.status === "modified"}
    class:highlighted
    role="button"
    tabindex="0"
    onclick={openEntry}
    onkeydown={handleEntryKeydown}
  >
    <div class="diff-entry-header">
      <span class="diff-status-badge">{
        entry.status === "added" ? "+" :
        entry.status === "removed" ? "−" :
        "~"
      }</span>
      <span class="diff-role-badge" style:--role-color={roleColor}>{roleLabel}</span>
      <span class="diff-preview">{preview}</span>
      {#if entry.changes.length > 0}
        <span class="diff-changes">
          {entry.changes.join(", ")}
        </span>
      {/if}
      {#if entry.hasContentChange}
        <button
          class="diff-expand-btn"
          class:expanded
          onclick={handleToggleInlineDiff}
          title="Toggle inline diff"
        >
          {expanded ? "▾ diff" : "▸ diff"}
        </button>
      {/if}
    </div>

    {#if expanded && inlineDiffLines}
      <div class="inline-diff" onclick={(e) => e.stopPropagation()} onkeydown={() => {}} role="presentation">
        {#each inlineDiffLines as line}
          <div
            class="diff-line"
            class:diff-line-added={line.type === "added"}
            class:diff-line-removed={line.type === "removed"}
            class:diff-line-unchanged={line.type === "unchanged"}
          >
            <span class="diff-line-marker">{getDiffMarker(line.type)}</span>
            <span class="diff-line-num">{line.oldLineNum ?? ""}</span>
            <span class="diff-line-num">{line.newLineNum ?? ""}</span>
            <span class="diff-line-content">{line.content || " "}</span>
          </div>
        {/each}
        {#if entry.snapshot && entry.current}
          <div class="inline-diff-footer">
            {#if pendingRevert}
              <button
                class="inline-revert-btn inline-revert-undo"
                onclick={(e) => {
                  e.stopPropagation();
                  setPendingRevert(null);
                }}
              >Undo Revert</button>
            {:else}
              <button
                class="inline-revert-btn"
                onclick={(e) => {
                  e.stopPropagation();
                  if (entry.snapshot && entry.current) {
                    setPendingRevert({
                      revertContent: entry.snapshot.content,
                      originalContent: entry.current.content,
                    });
                  }
                }}
              >Revert to snapshot version</button>
            {/if}
          </div>
        {/if}
      </div>
    {/if}
  </div>
{/if}

<style>
  .diff-entry {
    border: 1px solid var(--border-subtle);
    border-radius: 3px;
    overflow: hidden;
    transition: border-color 0.1s ease, box-shadow 0.1s ease;
    background: var(--bg-inset);
    cursor: pointer;
  }

  .diff-entry:hover {
    border-color: var(--border-default);
    background: var(--bg-surface);
  }

  .diff-entry:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }

  .diff-entry.highlighted {
    border-color: var(--accent);
    box-shadow: 0 0 0 1px var(--accent), 0 0 8px color-mix(in srgb, var(--accent) 25%, transparent);
  }

  .diff-entry.diff-added {
    border-left: 3px solid var(--semantic-success);
    background: color-mix(in srgb, var(--semantic-success) 4%, var(--bg-inset));
  }

  .diff-entry.diff-removed {
    border-left: 3px solid var(--semantic-danger);
    background: color-mix(in srgb, var(--semantic-danger) 4%, var(--bg-inset));
  }

  .diff-entry.diff-modified {
    border-left: 3px solid var(--semantic-warning);
    background: color-mix(in srgb, var(--semantic-warning) 4%, var(--bg-inset));
  }

  .diff-entry-header {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 8px;
  }

  .diff-status-badge {
    font-family: var(--font-mono);
    font-size: 11px;
    font-weight: 700;
    width: 14px;
    text-align: center;
    flex-shrink: 0;
  }

  .diff-added .diff-status-badge { color: var(--semantic-success); }
  .diff-removed .diff-status-badge { color: var(--semantic-danger); }
  .diff-modified .diff-status-badge { color: var(--semantic-warning); }

  .diff-role-badge {
    font-family: var(--font-mono);
    font-size: 8px;
    font-weight: 600;
    padding: 2px 4px;
    border-radius: 2px;
    background: color-mix(in srgb, var(--role-color) 18%, transparent);
    color: var(--role-color);
    letter-spacing: 0.2px;
    flex-shrink: 0;
  }

  .diff-preview {
    font-family: var(--font-mono);
    font-size: 9px;
    color: var(--text-secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
    min-width: 0;
  }

  .diff-changes {
    font-family: var(--font-mono);
    font-size: 9px;
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex-shrink: 0;
    max-width: 180px;
  }

  .diff-expand-btn {
    font-family: var(--font-mono);
    font-size: 9px;
    padding: 1px 5px;
    border: 1px solid var(--border-default);
    border-radius: 2px;
    background: var(--bg-surface);
    color: var(--text-muted);
    cursor: pointer;
    transition: all 0.1s ease;
    flex-shrink: 0;
    white-space: nowrap;
  }

  .diff-expand-btn:hover {
    background: var(--bg-hover);
    color: var(--text-primary);
    border-color: var(--border-strong);
  }

  .diff-expand-btn.expanded {
    background: var(--bg-hover);
    color: var(--text-primary);
  }

  .inline-diff {
    max-height: 200px;
    overflow-y: auto;
    border-top: 1px solid var(--border-subtle);
    background: var(--bg-base);
    scrollbar-width: thin;
    scrollbar-color: var(--border-default) transparent;
  }

  .diff-line {
    display: flex;
    align-items: stretch;
    font-family: var(--font-mono);
    font-size: 10px;
    line-height: 1.5;
    min-height: 18px;
  }

  .diff-line-added {
    background: color-mix(in srgb, var(--semantic-success) 10%, transparent);
  }

  .diff-line-removed {
    background: color-mix(in srgb, var(--semantic-danger) 10%, transparent);
    opacity: 0.7;
  }

  .diff-line-unchanged {
    color: var(--text-muted);
  }

  .diff-line-marker {
    width: 16px;
    text-align: center;
    flex-shrink: 0;
    font-weight: 700;
    user-select: none;
  }

  .diff-line-added .diff-line-marker { color: var(--semantic-success); }
  .diff-line-removed .diff-line-marker { color: var(--semantic-danger); }
  .diff-line-unchanged .diff-line-marker { color: var(--text-faint); }

  .diff-line-num {
    width: 28px;
    text-align: right;
    padding-right: 4px;
    color: var(--text-faint);
    flex-shrink: 0;
    user-select: none;
    font-size: 9px;
  }

  .diff-line-content {
    flex: 1;
    white-space: pre-wrap;
    word-break: break-all;
    padding-left: 4px;
  }

  .inline-diff-footer {
    padding: 4px 8px;
    border-top: 1px solid var(--border-subtle);
    background: var(--bg-surface);
  }

  .inline-revert-btn {
    font-family: var(--font-mono);
    font-size: 9px;
    padding: 3px 8px;
    border-radius: 2px;
    border: 1px solid color-mix(in srgb, var(--semantic-warning) 50%, transparent);
    background: transparent;
    color: var(--semantic-warning);
    cursor: pointer;
    transition: all 0.1s ease;
    width: 100%;
  }

  .inline-revert-btn:hover {
    background: var(--semantic-warning);
    border-color: var(--semantic-warning);
    color: var(--bg-surface);
  }

  .inline-revert-btn.inline-revert-undo {
    border-color: color-mix(in srgb, var(--semantic-danger) 50%, transparent);
    color: var(--semantic-danger);
  }

  .inline-revert-btn.inline-revert-undo:hover {
    background: var(--semantic-danger);
    border-color: var(--semantic-danger);
    color: var(--bg-surface);
  }
</style>
