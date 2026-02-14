<script lang="ts">
  import type { Block } from "$lib/types";
  import { blockTypesStore, contextStore } from "$lib/stores";
  import { searchStore, type SearchMatch } from "$lib/stores/search.svelte";
  import { detectLanguage, highlightCode, escapeHtml } from "$lib/utils/syntax";
  import { getPreview } from "$lib/utils/text";

  interface Props {
    block: Block;
    selected?: boolean;
    focused?: boolean;
    dragging?: boolean;
    contentExpanded?: boolean;
    selectedIds?: Set<string>;
    onSelect?: (id: string, event: { shiftKey: boolean; ctrlKey: boolean; metaKey: boolean }) => void;
    onDoubleClick?: (id: string) => void;
    onContextMenu?: (id: string, e: MouseEvent) => void;
    onDragStart?: (ids: string[]) => void;
    onDragEnd?: () => void;
  }

  let {
    block,
    selected = false,
    focused = false,
    dragging = false,
    contentExpanded = false,
    selectedIds = new Set<string>(),
    onSelect,
    onDoubleClick,
    onContextMenu,
    onDragStart,
    onDragEnd,
  }: Props = $props();

  // Per-block collapsed state (local, not persisted)
  let isCollapsed = $state(false);

  // Track whether content overflows its max-height (to show/hide fade gradient)
  let preRef = $state<HTMLElement | null>(null);
  let isOverflowing = $state(false);

  $effect(() => {
    // Re-check when content or expand state changes
    const _content = block.content;
    const _fullContent = showFullContent;
    const _collapsed = isCollapsed;
    if (preRef && !_collapsed && !_fullContent) {
      // $effect runs after DOM update, so measurements are accurate
      isOverflowing = preRef.scrollHeight > preRef.clientHeight + 2;
    } else {
      isOverflowing = false;
    }
  });

  // Count how many blocks are being dragged
  const dragCount = $derived(selected && selectedIds.size > 1 ? selectedIds.size : 1);

  // Get display info: use blockType if set, otherwise use role
  const displayTypeId = $derived(block.blockType ?? block.role);
  const typeInfo = $derived(blockTypesStore.getTypeById(displayTypeId));
  const displayColor = $derived(typeInfo?.color ?? "var(--text-muted)");
  const displayLabel = $derived(typeInfo?.shortLabel ?? displayTypeId.slice(0, 4).toUpperCase());
  const stalenessScore = $derived(contextStore.getBlockStaleness(block.id) ?? 0);

  // Whether to show full content (zone-level expand OR block not collapsed)
  const showContent = $derived(!isCollapsed);
  const showFullContent = $derived(contentExpanded && !isCollapsed);

  // Search integration
  const blockMatches = $derived(searchStore.isOpen ? searchStore.getBlockMatches(block.id) : []);
  const isCurrentMatch = $derived(searchStore.isOpen && searchStore.isCurrentMatchBlock(block.id));
  const currentBlockMatch = $derived(searchStore.getCurrentMatchForBlock(block.id));

  // Syntax highlighting (disabled when search is active)
  const detectedLang = $derived(detectLanguage(block.content, block.role));
  const syntaxHtml = $derived.by(() => {
    if (!detectedLang || blockMatches.length > 0) return null;
    const text = showFullContent ? block.content : getPreview(block.content);
    return highlightCode(text, detectedLang);
  });
  let blockEl = $state<HTMLElement | null>(null);

  // Scroll into view when this block is the current match or keyboard-focused
  $effect(() => {
    if ((isCurrentMatch || focused) && blockEl) {
      blockEl.scrollIntoView({ behavior: "smooth", block: "nearest" });
    }
  });

  /**
   * Highlight search matches in content.
   * XSS-safe: escapes HTML entities FIRST, then inserts <mark> tags.
   */
  function highlightContent(content: string, matches: SearchMatch[], currentMatch: SearchMatch | null): string {
    if (matches.length === 0) return escapeHtml(content);

    // Sort matches by position (ascending)
    const sorted = [...matches].sort((a, b) => a.startPos - b.startPos);

    // Build result by walking through original content
    let result = "";
    let lastEnd = 0;

    for (const m of sorted) {
      // Escape text before this match
      result += escapeHtml(content.slice(lastEnd, m.startPos));
      // Insert highlighted match
      const isCurrent = currentMatch && m.startPos === currentMatch.startPos && m.matchIndex === currentMatch.matchIndex;
      const cls = isCurrent ? "search-match current-match" : "search-match";
      result += `<mark class="${cls}">${escapeHtml(content.slice(m.startPos, m.endPos))}</mark>`;
      lastEnd = m.endPos;
    }

    // Escape remaining text
    result += escapeHtml(content.slice(lastEnd));
    return result;
  }

  function handleClick(e: MouseEvent) {
    onSelect?.(block.id, {
      shiftKey: e.shiftKey,
      ctrlKey: e.ctrlKey,
      metaKey: e.metaKey,
    });
  }

  function handleDoubleClick() {
    onDoubleClick?.(block.id);
  }

  function handleContextMenu(e: MouseEvent) {
    onContextMenu?.(block.id, e);
  }

  function handleDragStart(e: DragEvent) {
    const idsToMove = selected && selectedIds.size > 1
      ? [...selectedIds]
      : [block.id];

    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = "move";
      e.dataTransfer.setData("text/plain", JSON.stringify(idsToMove));

      // Create compact drag ghost instead of using full block element
      const ghost = document.createElement('div');
      const count = idsToMove.length;
      ghost.textContent = count > 1 ? `${count} blocks` : displayLabel;
      ghost.style.cssText = `
        padding: 4px 10px;
        background: var(--bg-elevated, #2a2a2a);
        color: var(--text-primary, #fff);
        font-family: var(--font-mono, monospace);
        font-size: 11px;
        font-weight: 600;
        border: 1px solid var(--border-default, #444);
        border-left: 3px solid ${displayColor};
        border-radius: 4px;
        white-space: nowrap;
        position: fixed;
        top: -100px;
        left: -100px;
        z-index: 9999;
      `;
      document.body.appendChild(ghost);
      e.dataTransfer.setDragImage(ghost, 0, 0);
      // Clean up after browser captures the ghost
      requestAnimationFrame(() => document.body.removeChild(ghost));
    }
    onDragStart?.(idsToMove);
  }

  function handleDragEnd() {
    onDragEnd?.();
  }

  function toggleCollapse(e: MouseEvent) {
    e.stopPropagation();
    isCollapsed = !isCollapsed;
  }

  function formatTokens(n: number): string {
    if (n >= 1000) return (n / 1000).toFixed(1) + "k";
    return n.toString();
  }
</script>

<div
  class="block"
  class:selected
  class:focused
  class:dragging
  class:collapsed={isCollapsed}
  class:pinned={block.pinned !== null}
  class:compressed={block.compressionLevel !== "original"}
  class:current-search-match={isCurrentMatch}
  style:--role-color={displayColor}
  data-block-id={block.id}
  bind:this={blockEl}
  role="button"
  tabindex="0"
  draggable="true"
  onclick={handleClick}
  ondblclick={handleDoubleClick}
  oncontextmenu={handleContextMenu}
  ondragstart={handleDragStart}
  ondragend={handleDragEnd}
  onkeydown={(e) => {
    if (e.key === "Enter" || e.key === " ") {
      handleClick(e as unknown as MouseEvent);
    }
  }}
>
  <div class="block-header" class:no-margin={isCollapsed}>
    <span class="role-badge">{displayLabel}</span>
    {#if block.pinned}
      <span class="pin-badge" title="Pinned to {block.pinned}">
        <span class="pin-icon">📌</span>
        <span class="pin-direction">{block.pinned === "top" ? "↑" : "↓"}</span>
      </span>
    {/if}
    {#if block.metadata.toolName}
      <span class="tool-name">{block.metadata.toolName}</span>
    {/if}
    {#if detectedLang}
      <span class="lang-badge">{detectedLang}</span>
    {/if}
    <span class="token-count">{formatTokens(block.tokens)}</span>
    {#if block.compressionLevel !== "original"}
      <span class="compression-inline-badge">{block.compressionLevel}</span>
    {/if}
    <span class="staleness-badge" title={`Staleness score ${stalenessScore.toFixed(1)}`}>
      S:{stalenessScore.toFixed(1)}
    </span>
    <button
      class="collapse-toggle"
      onclick={toggleCollapse}
      title={isCollapsed ? "Expand block" : "Collapse block"}
    >
      {isCollapsed ? "▸" : "▾"}
    </button>
  </div>

  {#if showContent}
    <div class="block-content" class:content-expanded={showFullContent} class:has-overflow={isOverflowing}>
      {#if blockMatches.length > 0}
        <!-- Safe invariant: highlightContent() escapes untrusted text via escapeHtml(). -->
        <!-- eslint-disable-next-line svelte/no-at-html-tags -->
        <pre bind:this={preRef}>{@html highlightContent(
          showFullContent ? block.content : getPreview(block.content),
          blockMatches.filter(m => showFullContent || m.startPos < 180),
          currentBlockMatch
        )}</pre>
      {:else if syntaxHtml}
        <!-- Safe invariant: highlightCode() returns Prism-escaped HTML. -->
        <!-- eslint-disable-next-line svelte/no-at-html-tags -->
        <pre class="syntax-highlighted" bind:this={preRef}>{@html syntaxHtml}</pre>
      {:else}
        <pre bind:this={preRef}>{showFullContent ? block.content : getPreview(block.content)}</pre>
      {/if}
    </div>
  {/if}

  {#if block.compressionLevel !== "original"}
    <span class="compression-badge">{block.compressionLevel}</span>
  {/if}

  {#if dragging && dragCount > 1}
    <span class="drag-count-badge">+{dragCount - 1}</span>
  {/if}
</div>

<style>
  .block {
    position: relative;
    background: var(--bg-elevated);
    border: 1px solid var(--border-subtle);
    border-radius: 4px;
    padding: calc(6px * var(--density-scale, 1)) calc(10px * var(--density-scale, 1));
    cursor: pointer;
    transition: all 0.1s ease;
    animation: block-materialize 0.2s ease-out;
  }

  @keyframes block-materialize {
    0% {
      opacity: 0;
      transform: translateY(4px);
    }
    100% {
      opacity: 1;
      transform: translateY(0);
    }
  }

  .block::before {
    content: "";
    position: absolute;
    left: 0;
    top: 0;
    bottom: 0;
    width: 2px;
    background: var(--role-color);
    border-radius: 4px 0 0 4px;
  }

  .block:hover {
    border-color: var(--border-default);
    background: var(--bg-hover);
  }

  .block.selected {
    border-color: var(--accent);
    background: var(--accent-subtle);
    box-shadow: inset 0 0 0 1px var(--accent);
  }

  .block.focused {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }

  .block.dragging {
    opacity: 0.4;
    box-shadow: none;
  }

  .block.compressed {
    opacity: 0.85;
    border-style: dashed;
  }

  .block.pinned::after {
    content: "";
    position: absolute;
    top: 4px;
    right: 4px;
    width: 4px;
    height: 4px;
    border-radius: 1px;
    background: var(--role-color);
  }

  .block-header {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-bottom: 4px;
  }

  .block-header.no-margin {
    margin-bottom: 0;
  }

  .role-badge {
    font-family: var(--font-mono);
    font-size: calc(9px * var(--density-scale, 1));
    font-weight: 600;
    padding: 2px 5px;
    border-radius: 2px;
    background: color-mix(in srgb, var(--role-color) 25%, transparent);
    color: var(--role-color);
    letter-spacing: 0.2px;
  }

  .pin-badge {
    display: flex;
    align-items: center;
    gap: 2px;
    font-size: 9px;
    padding: 1px 4px;
    border-radius: 2px;
    background: color-mix(in srgb, var(--semantic-warning) 20%, transparent);
    color: var(--semantic-warning);
  }

  .pin-icon {
    font-size: 8px;
  }

  .pin-direction {
    font-weight: 600;
  }

  .tool-name {
    font-size: 10px;
    color: var(--text-muted);
    background: var(--bg-muted);
    padding: 1px 4px;
    border-radius: 2px;
    max-width: 100px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .lang-badge {
    font-family: var(--font-mono);
    font-size: calc(9px * var(--density-scale, 1));
    font-weight: 600;
    padding: 2px 5px;
    border-radius: 2px;
    background: color-mix(in srgb, var(--role-user) 15%, transparent);
    color: var(--role-user);
    letter-spacing: 0.3px;
    text-transform: lowercase;
  }

  .token-count {
    margin-left: auto;
    font-family: var(--font-mono);
    font-size: 9px;
    color: var(--text-faint);
    font-variant-numeric: tabular-nums;
  }

  .staleness-badge {
    font-family: var(--font-mono);
    font-size: 8px;
    color: var(--text-faint);
    font-variant-numeric: tabular-nums;
    border: 1px solid var(--border-subtle);
    border-radius: 3px;
    padding: 1px 4px;
    background: var(--bg-inset);
  }

  .collapse-toggle {
    font-size: 11px;
    color: var(--text-muted);
    background: none;
    border: 1px solid var(--border-subtle);
    cursor: pointer;
    padding: 2px 5px;
    border-radius: 3px;
    line-height: 1;
    transition: all 0.1s ease;
    flex-shrink: 0;
  }

  .collapse-toggle:hover {
    color: var(--text-primary);
    background: var(--bg-hover);
    border-color: var(--border-default);
  }

  .block-content {
    position: relative;
  }

  .block-content pre {
    font-family: var(--font-mono);
    font-size: calc(11px * var(--density-scale, 1));
    line-height: 1.45;
    color: var(--text-primary);
    white-space: pre-wrap;
    word-break: break-word;
    margin: 0;
    max-height: calc(60px * var(--density-scale, 1));
    overflow: hidden;
  }

  .block-content.content-expanded pre {
    max-height: none;
  }

  .block-content.content-expanded::after {
    display: none;
  }

  /* Fade out at bottom — only when content actually overflows */
  .block-content::after {
    content: "";
    position: absolute;
    bottom: 0;
    left: 0;
    right: 0;
    height: 20px;
    background: linear-gradient(transparent, var(--bg-elevated));
    pointer-events: none;
    display: none;
  }

  .block-content.has-overflow::after {
    display: block;
  }

  .block.selected .block-content.has-overflow::after {
    background: linear-gradient(transparent, var(--accent-subtle));
  }

  .block:hover .block-content.has-overflow::after {
    background: linear-gradient(transparent, var(--bg-hover));
  }

  .compression-inline-badge {
    font-family: var(--font-mono);
    font-size: 8px;
    font-weight: 600;
    text-transform: uppercase;
    color: var(--semantic-warning);
    letter-spacing: 0.3px;
    background: color-mix(in srgb, var(--semantic-warning) 15%, transparent);
    padding: 1px 4px;
    border-radius: 2px;
    border: 1px dashed color-mix(in srgb, var(--semantic-warning) 30%, transparent);
  }

  .compression-badge {
    position: absolute;
    top: 6px;
    right: 8px;
    font-family: var(--font-mono);
    font-size: 8px;
    text-transform: uppercase;
    color: var(--text-faint);
    letter-spacing: 0.3px;
    background: var(--bg-muted);
    padding: 1px 3px;
    border-radius: 2px;
  }

  .drag-count-badge {
    position: absolute;
    top: -6px;
    right: -6px;
    font-family: var(--font-mono);
    font-size: 10px;
    font-weight: 600;
    color: var(--bg-surface);
    background: var(--accent);
    padding: 2px 5px;
    border-radius: 10px;
    z-index: 10;
    box-shadow: 0 2px 4px rgba(0,0,0,0.2);
  }

  /* Search highlighting */
  .block.current-search-match {
    border-color: var(--accent);
    box-shadow: 0 0 0 1px var(--accent), 0 0 8px color-mix(in srgb, var(--accent) 30%, transparent);
  }

  .block :global(.search-match) {
    background: color-mix(in srgb, var(--semantic-warning) 35%, transparent);
    color: inherit;
    border-radius: 1px;
    padding: 0 1px;
  }

  .block :global(.current-match) {
    background: color-mix(in srgb, var(--accent) 45%, transparent);
    outline: 1px solid var(--accent);
  }
</style>
