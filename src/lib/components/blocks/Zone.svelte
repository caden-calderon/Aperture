<script lang="ts">
  import type { Block, Zone as ZoneType } from "$lib/types";
  import { contextStore, zonesStore } from "$lib/stores";
  import { slide } from "svelte/transition";
  import { computeThreadPositions } from "$lib/utils/threading";
  import ContextBlock from "./ContextBlock.svelte";
  import Sparkline from "./Sparkline.svelte";

  interface Props {
    zone: ZoneType;
    blocks: Block[];
    collapsed?: boolean;
    selectedIds?: Set<string>;
    focusedBlockId?: string | null;
    draggingBlockIds?: string[];
    height?: number;
    expanded?: boolean;
    contentExpanded?: boolean;
    isResizing?: boolean;
    onToggleCollapse?: () => void;
    onToggleExpanded?: () => void;
    onToggleContentExpanded?: () => void;
    onBlockSelect?: (id: string, event: { shiftKey: boolean; ctrlKey: boolean; metaKey: boolean }) => void;
    onBlockDoubleClick?: (id: string) => void;
    onBlockContextMenu?: (id: string, e: MouseEvent) => void;
    onBlockDragStart?: (ids: string[]) => void;
    onBlockDragEnd?: () => void;
    onDrop?: (zone: ZoneType, blockIds: string[]) => void;
    onCreateBlock?: (zone: ZoneType, typeId: string) => void;
    onReorder?: (zone: ZoneType, blockIds: string[], insertIndex: number) => void;
    onResizeStart?: (e: MouseEvent, measuredHeight?: number) => void;
    transitionDuration?: number;
  }

  let {
    zone,
    blocks,
    collapsed = false,
    selectedIds = new Set<string>(),
    focusedBlockId = null,
    draggingBlockIds = [],
    height,
    expanded = false,
    contentExpanded = false,
    isResizing = false,
    onToggleCollapse,
    onToggleExpanded,
    onToggleContentExpanded,
    onBlockSelect,
    onBlockDoubleClick,
    onBlockContextMenu,
    onBlockDragStart,
    onBlockDragEnd,
    onDrop,
    onCreateBlock,
    onReorder,
    onResizeStart,
    transitionDuration = 150,
  }: Props = $props();

  let isDragOver = $state(false);
  let isTypeDropOver = $state(false);
  let dropInsertIndex = $state<number | null>(null);
  let zoneContentRef = $state<HTMLElement | null>(null);
  let dragEnterCount = $state(0); // Track nested drag enter/leave events

  // Get zone config from store (supports custom zones)
  const config = $derived.by(() => {
    const zoneInfo = zonesStore.getZoneById(zone);
    if (zoneInfo) {
      return {
        label: zoneInfo.label,
        color: zoneInfo.color,
        description: zoneInfo.isBuiltIn
          ? zone === "primacy"
            ? "High priority, always included"
            : zone === "recency"
              ? "Recent context, high position relevance"
              : "Standard context, compressed as needed"
          : "Custom zone",
      };
    }
    // Fallback for unknown zones
    return {
      label: zone,
      color: "var(--text-muted)",
      description: "Unknown zone",
    };
  });
  let totalTokens = $derived(blocks.reduce((sum, b) => sum + b.tokens, 0));
  let selectedInZone = $derived(blocks.filter((b) => selectedIds.has(b.id)));
  let tokenHistory = $derived(zonesStore.getTokenHistory(zone));

  // Thread lines are computed from role transitions plus turn continuity,
  // so unrelated blocks in the same zone don't get visually connected.
  const threadPositions = $derived.by(() => computeThreadPositions(blocks));

  function formatTokens(n: number): string {
    if (n >= 1000) return (n / 1000).toFixed(1) + "k";
    return n.toString();
  }

  function handleDragEnter(e: DragEvent) {
    e.preventDefault();
    dragEnterCount++;
  }

  function handleDragOver(e: DragEvent) {
    e.preventDefault();
    if (!e.dataTransfer) return;

    // Check if it's a block type being dragged from sidebar
    const hasBlockType = e.dataTransfer.types.includes('application/x-block-type');

    if (hasBlockType) {
      e.dataTransfer.dropEffect = "copy";
      isTypeDropOver = true;
      isDragOver = false;
      dropInsertIndex = null;
    } else {
      e.dataTransfer.dropEffect = "move";
      isDragOver = true;
      isTypeDropOver = false;

      // Calculate insertion index based on mouse position
      if (zoneContentRef && blocks.length > 0) {
        const blockElements = zoneContentRef.querySelectorAll('[data-block-id]');
        let insertIdx = blocks.length; // Default to end

        for (let i = 0; i < blockElements.length; i++) {
          const rect = blockElements[i].getBoundingClientRect();
          const midY = rect.top + rect.height / 2;
          if (e.clientY < midY) {
            insertIdx = i;
            break;
          }
        }

        // Clamp to valid range (respecting pinned blocks)
        const { min, max } = contextStore.getValidDropRange(zone);
        insertIdx = Math.max(min, Math.min(insertIdx, max));

        // Only update if changed to reduce re-renders
        if (dropInsertIndex !== insertIdx) {
          dropInsertIndex = insertIdx;
        }
      } else if (dropInsertIndex !== 0) {
        dropInsertIndex = 0;
      }
    }
  }

  function handleDragLeave(e: DragEvent) {
    e.preventDefault();
    dragEnterCount--;
    // Only reset when truly leaving the zone (counter reaches 0)
    if (dragEnterCount <= 0) {
      dragEnterCount = 0;
      isDragOver = false;
      isTypeDropOver = false;
      dropInsertIndex = null;
    }
  }

  function handleDrop(e: DragEvent) {
    e.preventDefault();
    const currentInsertIndex = dropInsertIndex;
    isDragOver = false;
    isTypeDropOver = false;
    dropInsertIndex = null;
    dragEnterCount = 0;

    // Check if it's a block type drop (create new block)
    const typeData = e.dataTransfer?.getData("application/x-block-type");
    if (typeData && onCreateBlock) {
      try {
        const parsed = JSON.parse(typeData);
        if (parsed.action === 'create' && parsed.typeId) {
          onCreateBlock(zone, parsed.typeId);
          return;
        }
      } catch {
        // Not a valid block type data
      }
    }

    // Otherwise, it's a block move
    const data = e.dataTransfer?.getData("text/plain");
    if (data) {
      try {
        // Try parsing as JSON array
        const blockIds: string[] = JSON.parse(data);
        if (Array.isArray(blockIds)) {
          // Check if blocks are from this zone (reorder) or different zone (move)
          const allFromThisZone = blockIds.every((id) =>
            blocks.some((b) => b.id === id)
          );

          if (allFromThisZone && onReorder && currentInsertIndex !== null) {
            onReorder(zone, blockIds, currentInsertIndex);
          } else if (onDrop) {
            onDrop(zone, blockIds);
          }
        } else if (onDrop) {
          // Fallback for single ID
          onDrop(zone, [data]);
        }
      } catch {
        // Fallback for plain string ID
        if (onDrop) {
          onDrop(zone, [data]);
        }
      }
    }
  }
</script>

<section
  class="zone"
  class:collapsed
  class:resizing={isResizing}
  class:drag-over={(isDragOver && draggingBlockIds.length > 0) || isTypeDropOver}
  style:--zone-color={config.color}
  aria-label="{config.label} zone"
  ondragenter={handleDragEnter}
  ondragover={handleDragOver}
  ondragleave={handleDragLeave}
  ondrop={handleDrop}
>
  <div class="zone-header-row">
    <button
      class="zone-header"
      onclick={onToggleCollapse}
      aria-expanded={!collapsed}
      title={config.description}
    >
      <div class="zone-info">
        <span class="zone-indicator"></span>
        <span class="zone-label">{config.label}</span>
        <span class="zone-stats">
          {blocks.length} blocks · {formatTokens(totalTokens)} tokens
          <Sparkline data={tokenHistory} color={config.color} label="{config.label} tokens" />
          {#if selectedInZone.length > 0}
            · <span class="selected-stat">{selectedInZone.length} selected</span>
          {/if}
        </span>
      </div>
      <span class="collapse-icon">{collapsed ? "+" : "−"}</span>
    </button>
    {#if !collapsed}
      <button
        class="expand-toggle"
        class:active={contentExpanded}
        onclick={onToggleContentExpanded}
        title={contentExpanded ? "Truncate block content" : "Show full block content"}
      >
        ☰
      </button>
      <button
        class="expand-toggle"
        class:active={expanded}
        onclick={onToggleExpanded}
        title={expanded ? "Collapse to scrollable" : "Expand to show all blocks"}
      >
        {expanded ? "⊟" : "⊞"}
      </button>
    {/if}
  </div>

  {#if !collapsed}
    <div
      class="zone-content"
      class:zone-expanded={expanded}
      bind:this={zoneContentRef}
      style:max-height={expanded ? "none" : `${height ?? 200}px`}
      style:overflow-y={expanded ? "visible" : "auto"}
    >
      {#if blocks.length === 0}
        <div class="zone-empty">Drop blocks here</div>
      {:else}
        {#each blocks as block, index (block.id)}
          {#if dropInsertIndex === index && isDragOver}
            <div class="drop-line"></div>
          {/if}
          <div
            class="thread-wrapper"
            class:thread-first={threadPositions.get(block.id) === 'first'}
            class:thread-middle={threadPositions.get(block.id) === 'middle'}
            class:thread-last={threadPositions.get(block.id) === 'last'}
            transition:slide={{ duration: transitionDuration }}
          >
            <ContextBlock
              {block}
              selected={selectedIds.has(block.id)}
              focused={focusedBlockId === block.id}
              dragging={draggingBlockIds.includes(block.id)}
              {contentExpanded}
              {selectedIds}
              onSelect={onBlockSelect}
              onDoubleClick={onBlockDoubleClick}
              onContextMenu={onBlockContextMenu}
              onDragStart={onBlockDragStart}
              onDragEnd={onBlockDragEnd}
            />
          </div>
        {/each}
        {#if dropInsertIndex === blocks.length && isDragOver}
          <div class="drop-line"></div>
        {/if}
      {/if}
    </div>
    <!-- Resize handle at bottom of zone -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="zone-resize-handle"
      class:active={isResizing}
      onmousedown={(e) => { e.preventDefault(); e.stopPropagation(); onResizeStart?.(e, zoneContentRef?.scrollHeight); }}
    >
      <!-- Grip pill: morphs from line → zone-colored pill with chevron on hover -->
      <button
        class="zone-grip-pill"
        title="Collapse zone"
        onmousedown={(e) => e.stopPropagation()}
        onclick={onToggleCollapse}
      >
        <svg class="grip-chevron" width="10" height="10" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round">
          <polyline points="4 6 8 10 12 6" />
        </svg>
      </button>
    </div>
  {/if}

  {#if isTypeDropOver}
    <div class="drop-indicator drop-indicator-create">
      <span>+ Create block in {config.label}</span>
    </div>
  {:else if isDragOver && draggingBlockIds.length > 0}
    <div class="drop-indicator">
      <span>Drop {draggingBlockIds.length > 1 ? `${draggingBlockIds.length} blocks` : ''} to {config.label}</span>
    </div>
  {/if}
</section>

<style>
  .zone {
    position: relative;
    background: var(--bg-surface);
    border: 1px solid var(--border-subtle);
    border-radius: 6px;
    transition: border-color 0.12s ease, box-shadow 0.12s ease;
    overflow: hidden;
    flex-shrink: 0;
  }

  .zone::before {
    content: "";
    position: absolute;
    left: 0;
    top: 0;
    bottom: 0;
    width: 2px;
    background: var(--zone-color);
  }

  .zone.resizing {
    border-color: var(--zone-color);
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--zone-color) 30%, transparent);
  }

  .zone.drag-over {
    border-color: var(--zone-color);
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--zone-color) 20%, transparent);
  }

  .zone-header-row {
    display: flex;
    align-items: center;
  }

  .zone-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex: 1;
    padding: 8px 12px 8px 14px;
    background: transparent;
    border: none;
    cursor: pointer;
    text-align: left;
    color: var(--text-primary);
    transition: background 0.1s ease;
  }

  .zone-header:hover {
    background: var(--bg-hover);
  }

  .expand-toggle {
    padding: 4px 8px;
    margin-right: 8px;
    font-size: 12px;
    background: transparent;
    border: 1px solid var(--border-subtle);
    border-radius: 3px;
    color: var(--text-muted);
    cursor: pointer;
    transition: all 0.1s ease;
  }

  .expand-toggle:hover {
    background: var(--bg-hover);
    border-color: var(--border-default);
    color: var(--text-primary);
  }

  .expand-toggle.active {
    background: var(--accent-subtle);
    border-color: var(--accent);
    color: var(--accent);
  }

  .zone-info {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .zone-indicator {
    width: 6px;
    height: 6px;
    border-radius: 1px;
    background: var(--zone-color);
  }

  .zone-label {
    font-family: var(--font-display);
    font-size: 12px;
    font-weight: 600;
    color: var(--text-primary);
    letter-spacing: -0.01em;
  }

  .zone-stats {
    font-size: 11px;
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }

  .selected-stat {
    color: var(--zone-color);
    font-weight: 500;
  }

  .collapse-icon {
    font-size: 12px;
    color: var(--text-muted);
    width: 16px;
    text-align: center;
    opacity: 0.6;
  }

  .zone-header:hover .collapse-icon {
    opacity: 1;
  }

  .zone-content {
    padding: calc(4px * var(--density-scale, 1)) calc(10px * var(--density-scale, 1)) calc(14px * var(--density-scale, 1)) 12px;
    border-top: 1px solid var(--border-subtle);
    display: flex;
    flex-direction: column;
    gap: calc(4px * var(--density-scale, 1));
    animation: zone-expand 0.15s ease-out;
    /* max-height and overflow-y controlled via inline styles */
    scrollbar-width: thin;
    scrollbar-color: var(--border-default) transparent;
  }

  .zone-content.zone-expanded {
    padding-bottom: calc(20px * var(--density-scale, 1));
  }

  .zone-content::-webkit-scrollbar {
    width: 6px;
  }

  .zone-content::-webkit-scrollbar-track {
    background: transparent;
  }

  .zone-content::-webkit-scrollbar-thumb {
    background: var(--border-default);
    border-radius: 3px;
  }

  .zone-content::-webkit-scrollbar-thumb:hover {
    background: var(--text-muted);
  }

  /* Zone resize handle */
  .zone-resize-handle {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 16px;
    cursor: row-resize;
    background: var(--bg-inset);
    border-top: 1px solid var(--border-subtle);
    transition: all 0.1s ease;
    user-select: none;
    position: relative;
  }

  .zone-resize-handle:hover {
    background: color-mix(in srgb, var(--zone-color) 15%, var(--bg-inset));
  }

  .zone-resize-handle.active {
    background: color-mix(in srgb, var(--zone-color) 25%, var(--bg-inset));
  }

  /* Grip pill — morphs from subtle line to zone-colored pill with chevron */
  .zone-grip-pill {
    width: 40px;
    height: 3px;
    background: var(--border-default);
    border: none;
    border-radius: 2px;
    cursor: pointer;
    transition: all 0.15s ease;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    overflow: hidden;
    color: var(--bg-surface);
  }

  .grip-chevron {
    opacity: 0;
    transition: opacity 0.12s ease;
    flex-shrink: 0;
  }

  .zone-resize-handle:hover .zone-grip-pill {
    width: 48px;
    height: 16px;
    background: var(--zone-color);
    border-radius: 8px;
  }

  .zone-resize-handle:hover .grip-chevron {
    opacity: 1;
  }

  .zone-resize-handle.active .zone-grip-pill {
    width: 80px;
    height: 4px;
    background: var(--zone-color);
    border-radius: 2px;
  }

  .zone-resize-handle.active .grip-chevron {
    opacity: 0;
  }

  @keyframes zone-expand {
    0% {
      opacity: 0;
      transform: translateY(-4px);
    }
    100% {
      opacity: 1;
      transform: translateY(0);
    }
  }

  /* Thread wrapper: vertical line + stems connecting related blocks */
  .thread-wrapper {
    position: relative;
  }

  /* Indent subsequent blocks in a thread */
  .thread-wrapper.thread-middle,
  .thread-wrapper.thread-last {
    padding-left: 14px;
  }

  /* Vertical thread line — continuous 2px line on left side */
  .thread-wrapper.thread-first::before,
  .thread-wrapper.thread-middle::before,
  .thread-wrapper.thread-last::before {
    content: "";
    position: absolute;
    left: -6px;
    width: 2px;
    background: var(--border-default);
    border-radius: 1px;
    z-index: 1;
  }

  .thread-wrapper.thread-first::before {
    top: 50%;
    bottom: calc(-1 * (4px * var(--density-scale, 1)) - 1px);
  }

  .thread-wrapper.thread-middle::before {
    top: -1px;
    bottom: calc(-1 * (4px * var(--density-scale, 1)) - 1px);
  }

  .thread-wrapper.thread-last::before {
    top: -1px;
    bottom: 50%;
  }

  /* Horizontal stems — short legs from vertical line to each block */
  .thread-wrapper.thread-first::after,
  .thread-wrapper.thread-middle::after,
  .thread-wrapper.thread-last::after {
    content: "";
    position: absolute;
    left: -6px;
    top: 50%;
    height: 2px;
    background: var(--border-default);
    border-radius: 1px;
    z-index: 1;
    transform: translateY(-1px);
  }

  /* First block: short stem (line to block edge) */
  .thread-wrapper.thread-first::after {
    width: 6px;
  }

  /* Indented blocks: longer stem (line to indented block edge) */
  .thread-wrapper.thread-middle::after,
  .thread-wrapper.thread-last::after {
    width: 20px;
  }

  .zone-empty {
    padding: var(--space-md);
    text-align: center;
    border: 1px dashed var(--border-default);
    border-radius: 4px;
    color: var(--text-faint);
    font-size: 11px;
    margin: 4px 0;
  }

  .drop-line {
    height: 2px;
    background: var(--zone-color);
    border-radius: 1px;
    margin: 2px 0;
    box-shadow: 0 0 4px var(--zone-color);
    animation: pulse-line 0.8s ease-in-out infinite;
  }

  @keyframes pulse-line {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.6; }
  }

  .drop-indicator {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    background: color-mix(in srgb, var(--zone-color) 10%, var(--bg-base) 80%);
    border-radius: 6px;
    pointer-events: none;
    z-index: 10;
    backdrop-filter: blur(2px);
  }

  .drop-indicator span {
    font-size: 11px;
    font-weight: 500;
    color: var(--bg-surface);
    padding: 6px 12px;
    background: var(--zone-color);
    border-radius: 4px;
  }

  .drop-indicator-create {
    background: color-mix(in srgb, var(--semantic-success) 10%, var(--bg-base) 80%);
  }

  .drop-indicator-create span {
    background: var(--semantic-success);
  }
</style>
