<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { TokenBudgetBar, Zone, Modal, Toast, CommandPalette, ThemeToggle, DensityControl, TitleBar, ThemeCustomizer, BlockTypeManager, ZoneManager, SearchBar, TerminalPanel, ContextMenu, ZoneMinimap, ContextDiff } from "$lib/components";
  import { contextStore, selectionStore, uiStore, themeStore, blockTypesStore, zonesStore, searchStore, terminalStore, editHistoryStore, connectionStore } from "$lib/stores";
  import { createResizable, createBlockHandlers, createModalHandlers, createKeyboardHandlers, createCommandHandlers } from "$lib/composables";
  import { convertBlock, type RustBlock } from "$lib/utils/blockConvert";
  import type { Zone as ZoneType } from "$lib/types";

  interface EngineSessionInfo {
    id: string;
    provider: string;
    model: string;
    source: string;
    thread_identity?: string | null;
    started_at: string;
    block_count: number;
    exchange_count: number;
    total_tokens: number;
    token_budget: number;
  }

  // -- Composables --

  const resizable = createResizable({ uiStore, zonesStore, terminalStore });
  const blockHandlers = createBlockHandlers({ selectionStore, uiStore, contextStore, zonesStore, blockTypesStore });
  const modalHandlers = createModalHandlers({ uiStore, contextStore, blockTypesStore });

  // TerminalPanel component ref (shared between keyboard + command handlers)
  let terminalPanelComponentRef = $state<ReturnType<typeof TerminalPanel> | null>(null);

  const keyboardHandlers = createKeyboardHandlers(
    { selectionStore, uiStore, contextStore, searchStore, terminalStore, zonesStore },
    { get terminalPanelComponentRef() { return terminalPanelComponentRef; }, get terminalPanelRef() { return resizable.terminalPanelRef; } }
  );

  // Diff view state (stays in page -- used by template inline handlers)
  let diffViewOpen = $state(false);
  let diffFilterZone = $state<string | null>(null);
  let diffHighlightBlockId = $state<string | null>(null);

  const commandHandlers = createCommandHandlers(
    { uiStore, selectionStore, contextStore, searchStore, terminalStore, zonesStore },
    { get terminalPanelComponentRef() { return terminalPanelComponentRef; } },
    {
      openDiffView: () => {
        diffFilterZone = null;
        diffHighlightBlockId = null;
        diffViewOpen = true;
      },
    }
  );

  // Snapshot CRUD state (stays in page -- used by sidebar template)
  let editingSnapshotId = $state<string | null>(null);
  let editSnapshotName = $state("");
  let deleteConfirmId = $state<string | null>(null);

  // Zones container ref (for minimap scroll targeting)
  let zonesRef = $state<HTMLElement | null>(null);
  let engineSessions = $state<EngineSessionInfo[]>([]);
  let activeEngineSessionId = $state<string | null>(null);
  let switchingSession = $state(false);

  function sessionLabel(session: EngineSessionInfo): string {
    return `${session.provider}/${session.model} · ${session.block_count} blocks`;
  }

  async function refreshEngineState(): Promise<void> {
    const [rustBlocks, budget, sessions, active] = await Promise.all([
      invoke<RustBlock[]>("engine_get_blocks"),
      invoke<{ limit_tokens: number }>("engine_get_budget_status"),
      invoke<EngineSessionInfo[]>("engine_get_sessions"),
      invoke<EngineSessionInfo | null>("engine_get_active_session"),
    ]);

    activeEngineSessionId = active?.id ?? null;
    contextStore.setActiveEngineSession(activeEngineSessionId, active?.thread_identity ?? null);
    contextStore.setEngineBlocks(rustBlocks.map(convertBlock));
    selectionStore.pruneStaleSelection();
    contextStore.setEngineBudget(budget.limit_tokens);
    engineSessions = sessions.sort((a, b) => b.started_at.localeCompare(a.started_at));
  }

  async function handleSessionSwitch(e: Event): Promise<void> {
    const target = e.currentTarget as HTMLSelectElement | null;
    const nextSessionId = target?.value ?? "";
    if (!nextSessionId || nextSessionId === activeEngineSessionId || switchingSession) {
      return;
    }

    switchingSession = true;
    try {
      const switched = await invoke<boolean>("engine_switch_session", { sessionId: nextSessionId });
      if (!switched) {
        uiStore.showToast("Could not switch session", "warning");
        return;
      }
      selectionStore.deselect();
      await refreshEngineState();
      const session = engineSessions.find((item) => item.id === nextSessionId);
      uiStore.showToast(
        `Switched to ${session ? `${session.provider}/${session.model}` : nextSessionId}`,
        "info"
      );
    } catch {
      uiStore.showToast("Session switching unavailable", "warning");
    } finally {
      switchingSession = false;
    }
  }

  // Initialize stores on mount
  onMount(() => {
    themeStore.init();
    uiStore.initDensity();
    uiStore.initSidebarWidth();
    blockTypesStore.init();
    zonesStore.init();
    uiStore.initContextPanel();
    uiStore.initMinimap();
    terminalStore.init();
    editHistoryStore.init();
    contextStore.init();

    // Connect to proxy and subscribe to engine-processed events
    connectionStore.onContextUpdated(async () => {
      try {
        await refreshEngineState();
      } catch {
        // Not in Tauri or engine unavailable — ignore
      }
    });
    connectionStore.onSessionReset(() => {
      contextStore.clearBlocks();
      contextStore.setActiveEngineSession(null, null);
      selectionStore.deselect();
      engineSessions = [];
      activeEngineSessionId = null;
    });
    connectionStore.connect();
    void refreshEngineState().catch(() => {
      // Not in Tauri or engine unavailable — ignore
    });

    // Flush debounced localStorage writes + cleanup terminal on window close
    const handleBeforeUnload = () => {
      contextStore.flushPendingWrites();
      zonesStore.flushPendingWrites();
      editHistoryStore.flushPendingWrites();

      const sid = terminalStore.sessionId;
      if (sid) {
        import("@tauri-apps/api/core").then(({ invoke }) => {
          invoke('kill_session', { sessionId: sid }).catch(() => {});
        });
      }
    };
    window.addEventListener('beforeunload', handleBeforeUnload);

    return () => {
      window.removeEventListener('beforeunload', handleBeforeUnload);
      connectionStore.disconnect();
    };
  });

  // Record token history for sparklines whenever blocks change
  $effect(() => {
    const byZone = contextStore.blocksByZone;
    if (contextStore.blocks.length === 0) return;

    const tokensByZone: Record<string, number> = {};
    for (const [zoneId, blocks] of Object.entries(byZone)) {
      tokensByZone[zoneId] = blocks.reduce((sum, b) => sum + b.tokens, 0);
    }
    zonesStore.recordTokenSnapshot(tokensByZone);
  });

  // Reactively update terminal theme when theme changes
  $effect(() => {
    void themeStore.currentPresetId;
    void themeStore.customColors;
    terminalPanelComponentRef?.updateTheme();
  });

  // Keep context mutation guardrails aligned with terminal transport mode.
  $effect(() => {
    contextStore.setContextMutationMode(terminalStore.contextMutationMode);
  });

  function formatNumber(n: number): string {
    return n.toLocaleString();
  }
</script>

<svelte:window
  on:keydown={keyboardHandlers.handleKeydown}
  on:mousemove={(e) => { resizable.handleResizeMove(e); resizable.handleZoneResizeMove(e); resizable.handleTermResizeMove(e); }}
  on:mouseup={() => { resizable.handleResizeEnd(); resizable.handleZoneResizeEnd(); resizable.handleTermResizeEnd(); }}
/>

<div class="app">
  <!-- Custom Title Bar (replaces OS title bar) -->
  <TitleBar />

  <!-- Header -->
  <header class="header">
    <div class="header-left">
      <TokenBudgetBar budget={contextStore.tokenBudget} limit={contextStore.tokenLimit} />
    </div>

    <div class="header-right">
      {#if engineSessions.length > 0}
        <label class="session-switcher">
          <span class="session-label">Session</span>
          <select
            class="session-select"
            value={activeEngineSessionId ?? ""}
            onchange={handleSessionSwitch}
            disabled={switchingSession || engineSessions.length < 2}
          >
            {#each engineSessions as session (session.id)}
              <option value={session.id}>{sessionLabel(session)}</option>
            {/each}
          </select>
        </label>
      {/if}
      <span
        class="mode-badge"
        class:mode-badge-direct={terminalStore.contextMutationMode.startsWith("direct")}
        class:mode-badge-readonly={terminalStore.contextMutationMode === "direct_read_only"}
        title={terminalStore.contextMutationMode === "direct_read_only"
          ? "Codex launched independently — observe only. Relaunch from terminal for full control."
          : "Proxy mode — full context control (edit, compress, reorder blocks)."}
      >
        {terminalStore.contextModeLabel}
      </span>
      <ThemeToggle />
      <button class="btn" onclick={() => contextStore.loadDemoData()}>Demo</button>
      <button class="btn btn-primary" onclick={() => uiStore.toggleCommandPalette()}>
        <span>⌘K</span>
      </button>
    </div>
  </header>

  <!-- Main -->
  <main class="main">
    <!-- Sidebar -->
    <aside
      class="sidebar"
      class:sidebar-collapsed={uiStore.sidebarCollapsed}
      class:sidebar-animating={!resizable.isResizingSidebar}
      bind:this={resizable.sidebarRef}
      style:width="{uiStore.sidebarWidth}px"
    >
      {#if uiStore.sidebarCollapsed}
        <!-- Collapsed icon bar -->
        <div class="sidebar-icons">
          <button class="sidebar-icon" title="Snapshots" onclick={() => uiStore.toggleSidebar()}>S</button>
          <button class="sidebar-icon" title="Block Types" onclick={() => uiStore.toggleSidebar()}>B</button>
          <button class="sidebar-icon" title="Zones" onclick={() => uiStore.toggleSidebar()}>Z</button>
          <button class="sidebar-icon" title="Theme" onclick={() => uiStore.toggleSidebar()}>T</button>
          <button class="sidebar-icon" title="Display" onclick={() => uiStore.toggleSidebar()}>D</button>
        </div>
      {:else}
        <section class="sidebar-section">
          <h3 class="sidebar-heading">Snapshots</h3>

          <!-- State badge -->
          <div class="state-badge" class:on-snapshot={contextStore.activeSnapshotId}>
            <span class="state-label">{contextStore.activeStateLabel}</span>
            {#if contextStore.activeSnapshotId}
              <button
                class="state-back-btn"
                onclick={() => {
                  contextStore.switchToWorkingState();
                  uiStore.showToast("Switched to Working State", "info");
                }}
              >Back to Working</button>
            {/if}
          </div>

          {#if contextStore.snapshots.length === 0}
            <p class="sidebar-empty">No snapshots</p>
          {:else}
            <div class="snapshot-list">
              {#each contextStore.snapshots as snapshot (snapshot.id)}
                <!-- svelte-ignore a11y_no_static_element_interactions -->
                <div
                  class="snapshot-item"
                  class:snapshot-active={contextStore.activeSnapshotId === snapshot.id}
                  ondblclick={() => {
                    contextStore.switchToSnapshot(snapshot.id);
                    uiStore.showToast(`Switched to: ${snapshot.name}`, "info");
                  }}
                >
                  <div class="snapshot-info">
                    {#if editingSnapshotId === snapshot.id}
                      <input
                        class="snapshot-rename-input"
                        bind:value={editSnapshotName}
                        onblur={() => {
                          if (editSnapshotName.trim()) {
                            contextStore.renameSnapshot(snapshot.id, editSnapshotName.trim());
                          }
                          editingSnapshotId = null;
                        }}
                        onkeydown={(e) => {
                          if (e.key === 'Enter') {
                            if (editSnapshotName.trim()) {
                              contextStore.renameSnapshot(snapshot.id, editSnapshotName.trim());
                            }
                            editingSnapshotId = null;
                          }
                          if (e.key === 'Escape') {
                            editingSnapshotId = null;
                          }
                        }}
                      />
                    {:else}
                      <span class="snapshot-name">{snapshot.name}</span>
                    {/if}
                    <span class="snapshot-meta">{formatNumber(snapshot.totalTokens)} tokens</span>
                  </div>
                  <div class="snapshot-actions">
                    <button
                      class="snapshot-action-btn"
                      onclick={() => {
                        contextStore.switchToSnapshot(snapshot.id);
                        uiStore.showToast(`Switched to: ${snapshot.name}`, "info");
                      }}
                      title="Switch to"
                    >↗</button>
                    <button
                      class="snapshot-action-btn"
                      onclick={() => {
                        editingSnapshotId = snapshot.id;
                        editSnapshotName = snapshot.name;
                        requestAnimationFrame(() => {
                          const input = document.querySelector('.snapshot-rename-input') as HTMLInputElement;
                          input?.focus();
                          input?.select();
                        });
                      }}
                      title="Rename"
                    >✎</button>
                    {#if deleteConfirmId === snapshot.id}
                      <button
                        class="snapshot-action-btn snapshot-confirm-delete"
                        onclick={() => {
                          contextStore.deleteSnapshot(snapshot.id);
                          deleteConfirmId = null;
                          uiStore.showToast(`Deleted: ${snapshot.name}`, "success");
                        }}
                        onblur={() => deleteConfirmId = null}
                      >Confirm?</button>
                    {:else}
                      <button
                        class="snapshot-action-btn snapshot-delete-btn"
                        onclick={() => deleteConfirmId = snapshot.id}
                        title="Delete"
                      >×</button>
                    {/if}
                  </div>
                </div>
              {/each}
            </div>
          {/if}
          <button
            class="btn btn-full"
            onclick={() => {
              const snap = contextStore.saveSnapshot(`Snapshot ${contextStore.snapshots.length + 1}`);
              uiStore.showToast(`Saved: ${snap.name}`, "success");
            }}
          >
            Save Snapshot
          </button>
        </section>

        <!-- Block Types Manager -->
        <BlockTypeManager />

        <!-- Zone Manager -->
        <ZoneManager />

        <section class="sidebar-section">
          <h3 class="sidebar-heading">Display</h3>
          <DensityControl />
        </section>

        <!-- Theme Customizer -->
        <ThemeCustomizer />

        <section class="sidebar-section sidebar-grow">
          <h3 class="sidebar-heading">Shortcuts</h3>
          <div class="shortcuts">
            <div class="shortcut"><kbd>J/K</kbd> Navigate</div>
            <div class="shortcut"><kbd>↑/↓</kbd> Navigate</div>
            <div class="shortcut"><kbd>Enter</kbd> Open block</div>
            <div class="shortcut"><kbd>A</kbd> Select all</div>
            <div class="shortcut"><kbd>Esc</kbd> Deselect</div>
            <div class="shortcut"><kbd>Del</kbd> Remove</div>
            <div class="shortcut"><kbd>S</kbd> Snapshot</div>
            <div class="shortcut"><kbd>⌘K</kbd> Commands</div>
            <div class="shortcut"><kbd>⌘[</kbd> Sidebar</div>
            <div class="shortcut"><kbd>⌘F</kbd> Search</div>
            <div class="shortcut"><kbd>⌃T</kbd> Terminal</div>
          </div>
        </section>
      {/if}
    </aside>

    <!-- Sidebar Resize Handle -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="sidebar-resize-handle"
      class:active={resizable.isResizingSidebar}
      onmousedown={resizable.handleResizeStart}
    >
      <button
        class="sidebar-toggle-btn"
        title={uiStore.sidebarCollapsed ? "Expand sidebar (Ctrl+[)" : "Collapse sidebar (Ctrl+[)"}
        onmousedown={(e) => e.stopPropagation()}
        onclick={() => uiStore.toggleSidebar()}
      >{uiStore.sidebarCollapsed ? "›" : "‹"}</button>
    </div>

    <!-- Content -->
    <div class="content" class:content-right={terminalStore.isVisible && terminalStore.position === 'right'} bind:this={resizable.contentRef}>
      {#if uiStore.contextPanelCollapsed}
        <!-- Context panel collapsed bar -->
        <button class="context-collapsed" onclick={() => uiStore.toggleContextPanel()} title="Expand context panel">
          <svg class="context-collapsed-icon" width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <rect x="2" y="2" width="12" height="12" rx="1" />
            <line x1="5" y1="6" x2="11" y2="6" />
            <line x1="5" y1="10" x2="9" y2="10" />
          </svg>
          <span class="context-collapsed-label">Context</span>
          <span class="context-collapsed-count">{contextStore.blocks.length}</span>
        </button>
      {:else}
        <div class="content-main">
          <!-- Toolbar -->
          <div class="toolbar">
            <div class="toolbar-info">
              <strong>{selectionStore.count}</strong> selected
              <span class="toolbar-sep">·</span>
              <strong>{formatNumber(selectionStore.selectedTokens)}</strong> tokens
              {#if uiStore.minimapVisible && contextStore.blocks.length > 0}
                <ZoneMinimap
                  zonesContainer={zonesRef}
                  onDrop={blockHandlers.handleZoneDrop}
                />
              {:else if contextStore.blocks.length > 0}
                <button
                  class="btn btn-sm minimap-show-btn"
                  title="Show minimap"
                  onclick={() => uiStore.toggleMinimap()}
                >
                  <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round">
                    <rect x="1" y="2" width="3" height="6" rx="0.5" />
                    <rect x="5" y="1" width="4" height="8" rx="0.5" />
                  </svg>
                </button>
              {/if}
            </div>
            <div class="toolbar-actions">
              <button class="btn btn-sm" onclick={() => uiStore.toggleContextPanel()} title="Collapse context panel">
                <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round">
                  <line x1="3" y1="8" x2="13" y2="8" />
                </svg>
              </button>
              <button class="btn btn-sm" onclick={() => searchStore.toggle()} title="Search (Ctrl+F)">
                <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><circle cx="6.5" cy="6.5" r="5"/><line x1="10.5" y1="10.5" x2="15" y2="15"/></svg>
              </button>
              <button class="btn btn-sm" onclick={() => selectionStore.selectAll()}>Select All</button>
              <button class="btn btn-sm" onclick={() => selectionStore.deselect()}>Clear</button>
              <button
                class="btn btn-sm btn-danger"
                disabled={!selectionStore.hasSelection}
                onclick={async () => {
                  const result = await contextStore.removeBlocks([...selectionStore.selectedIds]);
                  if (result.applied) {
                    selectionStore.deselect();
                    uiStore.showToast(`Removed ${result.affected} block(s)`, "success");
                  } else {
                    uiStore.showToast(result.reason ?? "Action blocked by policy", "warning");
                  }
                }}
              >
                Remove
              </button>
            </div>
          </div>

          <!-- Search Bar -->
          <SearchBar />

          <!-- Zones -->
          <div class="zones" class:resizing={resizable.resizingZoneId !== null} bind:this={zonesRef}>
            {#if contextStore.blocks.length === 0}
              <div class="empty-state">
                <div class="empty-icon">
                  <svg width="48" height="48" viewBox="0 0 48 48" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                    <rect x="6" y="6" width="36" height="36" rx="4" />
                    <line x1="6" y1="18" x2="42" y2="18" />
                    <line x1="6" y1="30" x2="42" y2="30" />
                    <line x1="18" y1="6" x2="18" y2="42" />
                  </svg>
                </div>
                <h2 class="empty-title">No context blocks</h2>
                <p class="empty-description">
                  Connect to the proxy to capture live conversation data,
                  or load demo blocks to explore the interface.
                </p>
                <div class="empty-actions">
                  <button class="btn" onclick={() => contextStore.loadDemoData()}>
                    Load Demo Data
                  </button>
                  <button class="btn" onclick={() => uiStore.toggleCommandPalette()}>
                    Open Commands <kbd>⌘K</kbd>
                  </button>
                </div>
                <div class="empty-hints">
                  <div class="empty-hint">
                    <kbd>J</kbd>/<kbd>K</kbd> Navigate blocks
                  </div>
                  <div class="empty-hint">
                    <kbd>Enter</kbd> Open block details
                  </div>
                  <div class="empty-hint">
                    <kbd>⌃T</kbd> Toggle terminal
                  </div>
                </div>
              </div>
            {:else}
              {#each zonesStore.zonesByDisplayOrder as zoneConfig (zoneConfig.id)}
                <Zone
                  zone={zoneConfig.id as ZoneType}
                  blocks={contextStore.blocksByZone[zoneConfig.id] ?? []}
                  collapsed={uiStore.isZoneCollapsed(zoneConfig.id)}
                  selectedIds={selectionStore.selectedIds}
                  focusedBlockId={selectionStore.focusedId}
                  draggingBlockIds={uiStore.draggingBlockIds}
                  height={zonesStore.getZoneHeight(zoneConfig.id)}
                  expanded={zonesStore.isZoneExpanded(zoneConfig.id)}
                  contentExpanded={zonesStore.isContentExpanded(zoneConfig.id)}
                  isResizing={resizable.resizingZoneId === zoneConfig.id}
                  onToggleCollapse={() => blockHandlers.handleToggleZoneCollapse(zoneConfig.id as ZoneType)}
                  onToggleExpanded={() => { if (!resizable.resizingZoneId) zonesStore.toggleZoneExpanded(zoneConfig.id); }}
                  onToggleContentExpanded={() => zonesStore.toggleContentExpanded(zoneConfig.id)}
                  onBlockSelect={blockHandlers.handleBlockSelect}
                  onBlockDoubleClick={blockHandlers.handleBlockDoubleClick}
                  onBlockContextMenu={blockHandlers.handleBlockContextMenu}
                  onBlockDragStart={blockHandlers.handleBlockDragStart}
                  onBlockDragEnd={blockHandlers.handleBlockDragEnd}
                  onDrop={blockHandlers.handleZoneDrop}
                  onCreateBlock={blockHandlers.handleCreateBlock}
                  onReorder={blockHandlers.handleZoneReorder}
                  onResizeStart={(e, h) => resizable.handleZoneResizeStart(e, zoneConfig.id, h)}
                  transitionDuration={uiStore.transitionDuration}
                />
              {/each}
            {/if}

          </div>
        </div>
      {/if}

      <!-- Terminal Split Handle + Panel -->
      {#if terminalStore.isVisible}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div
          class="terminal-split-handle"
          class:terminal-split-vertical={terminalStore.position === 'right'}
          class:active={resizable.isResizingTerminal}
          onmousedown={resizable.handleTermResizeStart}
        >
          <button
            class="terminal-toggle-btn"
            class:terminal-toggle-vertical={terminalStore.position === 'right'}
            title={terminalStore.isCollapsed ? "Expand terminal" : "Collapse terminal"}
            onmousedown={(e) => e.stopPropagation()}
            onclick={() => terminalStore.toggleCollapsed()}
          >
            {#if terminalStore.position === 'bottom'}
              {terminalStore.isCollapsed ? "▴" : "▾"}
            {:else}
              {terminalStore.isCollapsed ? "‹" : "›"}
            {/if}
          </button>
        </div>

        <div
          class="terminal-panel-wrapper"
          class:terminal-panel-right={terminalStore.position === 'right'}
          class:terminal-fill={uiStore.contextPanelCollapsed}
          bind:this={resizable.terminalPanelRef}
          style={uiStore.contextPanelCollapsed
            ? ''
            : (terminalStore.position === 'bottom'
              ? `height: ${terminalStore.terminalHeight}px`
              : `width: ${terminalStore.terminalWidth}px`)}
        >
          <TerminalPanel bind:this={terminalPanelComponentRef} />
        </div>
      {/if}
    </div>
  </main>
</div>

<!-- Modal -->
<Modal
  block={uiStore.modalBlockId ? contextStore.getBlock(uiStore.modalBlockId) ?? null : null}
  open={uiStore.hasModal}
  onClose={modalHandlers.handleModalClose}
  onCompress={modalHandlers.handleModalCompress}
  onMove={modalHandlers.handleModalMove}
  onPin={modalHandlers.handleModalPin}
  onRemove={modalHandlers.handleModalRemove}
  onRoleChange={modalHandlers.handleModalRoleChange}
  onContentEdit={modalHandlers.handleModalContentEdit}
  onOpenDiff={(filterZone, highlightBlockId) => {
    diffFilterZone = filterZone ?? null;
    diffHighlightBlockId = highlightBlockId ?? null;
    uiStore.closeModal();
    diffViewOpen = true;
  }}
  onRevert={async (blockId, content) => {
    const result = await contextStore.updateBlockContent(blockId, content);
    if (result.applied) {
      uiStore.showToast("Reverted to previous version", "success");
    } else {
      uiStore.showToast(result.reason ?? "Action blocked by policy", "warning");
    }
  }}
  snapshots={contextStore.snapshots}
/>

<!-- Toasts -->
<Toast toasts={uiStore.toasts} onDismiss={(id) => uiStore.dismissToast(id)} />

<!-- Command Palette -->
<CommandPalette
  open={uiStore.commandPaletteOpen}
  onClose={() => uiStore.toggleCommandPalette()}
  onCommand={commandHandlers.handleCommand}
/>

<!-- Context Diff View -->
<ContextDiff
  open={diffViewOpen}
  onClose={() => { diffViewOpen = false; diffFilterZone = null; diffHighlightBlockId = null; }}
  filterZone={diffFilterZone}
  highlightBlockId={diffHighlightBlockId}
  onOpenBlock={(blockId: string) => { diffViewOpen = false; diffFilterZone = null; diffHighlightBlockId = null; uiStore.openModal(blockId); }}
  onRevert={async (blockId: string, content: string) => {
    const result = await contextStore.updateBlockContent(blockId, content);
    if (result.applied) {
      uiStore.showToast("Reverted to snapshot version", "success");
    } else {
      uiStore.showToast(result.reason ?? "Action blocked by policy", "warning");
    }
  }}
/>

<!-- Context Menu (right-click) -->
<ContextMenu
  block={blockHandlers.contextMenuBlock ? contextStore.getBlock(blockHandlers.contextMenuBlock) ?? null : null}
  x={blockHandlers.contextMenuX}
  y={blockHandlers.contextMenuY}
  visible={blockHandlers.contextMenuVisible}
  onClose={blockHandlers.closeContextMenu}
  onPin={blockHandlers.handleContextMenuPin}
  onMove={blockHandlers.handleContextMenuMove}
  onCompress={blockHandlers.handleContextMenuCompress}
  onCopy={blockHandlers.handleContextMenuCopy}
  onRemove={blockHandlers.handleContextMenuRemove}
  onOpen={blockHandlers.handleContextMenuOpen}
/>

<style>
  .app {
    height: 100vh;
    display: flex;
    flex-direction: column;
    background: var(--bg-base);
    position: relative;
  }

  /* Header — Below title bar */
  .header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: calc(var(--space-sm) * var(--density-scale, 1)) var(--space-md);
    background: var(--bg-surface);
    border-bottom: 1px solid var(--border-subtle);
    gap: var(--space-md);
    z-index: 10;
  }

  .header-left {
    flex: 1;
    max-width: 500px;
  }

  .header-right {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    flex-shrink: 0;
  }

  .session-switcher {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }

  .session-label {
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.4px;
  }

  .session-select {
    min-width: 210px;
    max-width: 290px;
    padding: 4px 8px;
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--text-secondary);
    background: var(--bg-surface);
    border: 1px solid var(--border-default);
    border-radius: 4px;
  }

  .session-select:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .mode-badge {
    font-family: var(--font-mono);
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.25px;
    color: var(--semantic-success);
    border: 1px solid color-mix(in srgb, var(--semantic-success) 45%, transparent);
    background: color-mix(in srgb, var(--semantic-success) 10%, transparent);
    border-radius: 999px;
    padding: 3px 8px;
    white-space: nowrap;
  }

  .mode-badge.mode-badge-direct {
    color: var(--semantic-info);
    border-color: color-mix(in srgb, var(--semantic-info) 55%, transparent);
    background: color-mix(in srgb, var(--semantic-info) 14%, transparent);
  }

  .mode-badge.mode-badge-readonly {
    color: var(--semantic-warning);
    border-color: color-mix(in srgb, var(--semantic-warning) 55%, transparent);
    background: color-mix(in srgb, var(--semantic-warning) 14%, transparent);
  }

  /* Buttons */
  .btn {
    font-family: var(--font-mono);
    font-size: 11px;
    padding: 5px 10px;
    border: 1px solid var(--border-default);
    border-radius: 4px;
    background: var(--bg-surface);
    color: var(--text-secondary);
    cursor: pointer;
    transition: all 0.12s ease;
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .btn:hover:not(:disabled) {
    background: var(--bg-hover);
    color: var(--text-primary);
    border-color: var(--border-strong);
  }

  .btn:active:not(:disabled) {
    transform: scale(0.98);
  }

  .btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .btn-sm {
    font-size: 10px;
    padding: 3px 7px;
  }

  .btn-primary {
    background: var(--text-primary);
    color: var(--bg-surface);
    border-color: var(--text-primary);
    font-weight: 500;
  }

  .btn-primary:hover:not(:disabled) {
    opacity: 0.85;
    color: var(--bg-surface);
  }

  .btn-danger {
    color: var(--semantic-danger);
    border-color: color-mix(in srgb, var(--semantic-danger) 50%, transparent);
  }

  .btn-danger:hover:not(:disabled) {
    background: var(--semantic-danger);
    color: var(--bg-surface);
    border-color: var(--semantic-danger);
  }

  .btn-full {
    width: 100%;
    justify-content: center;
  }

  /* Main layout */
  .main {
    flex: 1;
    display: flex;
    overflow: hidden;
  }

  /* Sidebar */
  .sidebar {
    background: var(--bg-surface);
    border-right: 1px solid var(--border-subtle);
    display: flex;
    flex-direction: column;
    flex-shrink: 0;
    overflow-y: auto;
    overflow-x: hidden;
    scrollbar-gutter: stable;
  }

  /* Sidebar collapsed state */
  .sidebar.sidebar-collapsed {
    overflow: hidden;
  }

  .sidebar.sidebar-animating {
    transition: width 0.2s ease;
  }

  .sidebar-icons {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    padding: 8px 0;
    width: 100%;
  }

  .sidebar-icon {
    width: 28px;
    height: 28px;
    display: flex;
    align-items: center;
    justify-content: center;
    font-family: var(--font-mono);
    font-size: 10px;
    font-weight: 600;
    color: var(--text-muted);
    background: transparent;
    border: 1px solid transparent;
    border-radius: 4px;
    cursor: pointer;
    transition: all 0.1s ease;
  }

  .sidebar-icon:hover {
    background: var(--bg-hover);
    color: var(--text-primary);
    border-color: var(--border-subtle);
  }

  /* Sidebar Resize Handle */
  .sidebar-resize-handle {
    width: 4px;
    cursor: col-resize;
    background: transparent;
    transition: background 0.15s ease;
    flex-shrink: 0;
    position: relative;
  }

  .sidebar-resize-handle:hover,
  .sidebar-resize-handle.active {
    background: var(--accent);
  }

  .sidebar-toggle-btn {
    position: absolute;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    width: 16px;
    height: 32px;
    display: flex;
    align-items: center;
    justify-content: center;
    font-family: var(--font-mono);
    font-size: 14px;
    font-weight: 600;
    color: var(--text-muted);
    background: var(--bg-surface);
    border: 1px solid var(--border-subtle);
    border-radius: 3px;
    cursor: pointer;
    opacity: 0;
    transition: opacity 0.15s ease, color 0.1s ease;
    z-index: 5;
    padding: 0;
    line-height: 1;
  }

  .sidebar-resize-handle:hover .sidebar-toggle-btn {
    opacity: 1;
  }

  .sidebar-toggle-btn:hover {
    color: var(--text-primary);
    border-color: var(--accent);
  }

  .sidebar-section {
    padding: var(--space-md) var(--space-sm);
    border-bottom: 1px solid var(--border-subtle);
  }

  .sidebar-section:last-child {
    border-bottom: none;
  }

  .sidebar-grow {
    flex: 1;
  }

  .sidebar-heading {
    font-family: var(--font-display);
    font-size: 9px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.6px;
    color: var(--text-muted);
    margin-bottom: var(--space-sm);
    padding-left: 2px;
  }

  .sidebar-empty {
    font-size: 11px;
    color: var(--text-faint);
    font-style: italic;
    margin-bottom: var(--space-sm);
  }

  /* State badge */
  .state-badge {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
    padding: 4px 8px;
    margin-bottom: var(--space-sm);
    border-radius: 3px;
    background: var(--bg-inset);
    border: 1px solid var(--border-subtle);
  }

  .state-badge.on-snapshot {
    background: color-mix(in srgb, var(--accent) 10%, var(--bg-inset));
    border-color: color-mix(in srgb, var(--accent) 30%, var(--border-subtle));
  }

  .state-label {
    font-family: var(--font-mono);
    font-size: 9px;
    font-weight: 600;
    color: var(--text-secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .state-back-btn {
    font-family: var(--font-mono);
    font-size: 8px;
    padding: 2px 5px;
    border: 1px solid var(--border-default);
    border-radius: 2px;
    background: var(--bg-surface);
    color: var(--text-muted);
    cursor: pointer;
    transition: all 0.1s ease;
    flex-shrink: 0;
    white-space: nowrap;
  }

  .state-back-btn:hover {
    background: var(--bg-hover);
    color: var(--text-primary);
    border-color: var(--border-strong);
  }

  /* Snapshots */
  .snapshot-list {
    display: flex;
    flex-direction: column;
    gap: 3px;
    margin-bottom: var(--space-sm);
    max-height: 160px;
    overflow-y: auto;
  }

  .snapshot-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 4px 8px;
    background: var(--bg-elevated);
    border: 1px solid var(--border-subtle);
    border-radius: 3px;
    transition: all 0.1s ease;
    text-align: left;
    width: 100%;
    gap: 4px;
  }

  .snapshot-item:hover {
    background: var(--bg-hover);
    border-color: var(--border-default);
  }

  .snapshot-item.snapshot-active {
    border-left: 3px solid var(--accent);
    background: color-mix(in srgb, var(--accent) 8%, var(--bg-elevated));
  }

  .snapshot-info {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
    flex: 1;
  }

  .snapshot-name {
    font-size: 10px;
    color: var(--text-secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .snapshot-meta {
    font-size: 9px;
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }

  .snapshot-rename-input {
    font-family: var(--font-mono);
    font-size: 10px;
    padding: 1px 4px;
    background: var(--bg-base);
    border: 1px solid var(--accent);
    border-radius: 2px;
    color: var(--text-primary);
    outline: none;
    width: 100%;
  }

  .snapshot-actions {
    display: flex;
    align-items: center;
    gap: 2px;
    flex-shrink: 0;
    opacity: 0;
    transition: opacity 0.1s ease;
  }

  .snapshot-item:hover .snapshot-actions {
    opacity: 1;
  }

  .snapshot-item.snapshot-active .snapshot-actions {
    opacity: 1;
  }

  .snapshot-action-btn {
    font-family: var(--font-mono);
    font-size: 13px;
    padding: 2px 5px;
    background: transparent;
    border: 1px solid transparent;
    border-radius: 2px;
    color: var(--text-muted);
    cursor: pointer;
    transition: all 0.1s ease;
    line-height: 1;
  }

  .snapshot-action-btn:hover {
    background: var(--bg-hover);
    color: var(--text-primary);
    border-color: var(--border-default);
  }

  .snapshot-delete-btn:hover {
    color: var(--semantic-danger);
    border-color: color-mix(in srgb, var(--semantic-danger) 40%, transparent);
  }

  .snapshot-confirm-delete {
    font-size: 8px;
    padding: 1px 4px;
    color: var(--semantic-danger);
    border-color: var(--semantic-danger);
    background: color-mix(in srgb, var(--semantic-danger) 10%, transparent);
  }

  /* Shortcuts */
  .shortcuts {
    display: flex;
    flex-direction: column;
    gap: 5px;
    font-size: 10px;
    color: var(--text-muted);
  }

  .shortcut {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  kbd {
    font-family: var(--font-mono);
    font-size: 9px;
    padding: 2px 4px;
    background: var(--bg-inset);
    border: 1px solid var(--border-subtle);
    border-radius: 2px;
    color: var(--text-secondary);
    min-width: 20px;
    text-align: center;
  }

  /* Content area */
  .content {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    background: var(--bg-base);
  }

  .content.content-right {
    flex-direction: row;
  }

  .content-main {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    min-height: 0;
    min-width: 0;
  }

  /* Context panel collapsed bar */
  .context-collapsed {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    background: var(--bg-surface);
    border: none;
    border-bottom: 1px solid var(--border-subtle);
    cursor: pointer;
    transition: background 0.1s ease;
    padding: 6px 12px;
    flex-shrink: 0;
    width: 100%;
  }

  /* When terminal is on right, context collapsed bar should be vertical */
  .content.content-right .context-collapsed {
    writing-mode: vertical-rl;
    text-orientation: mixed;
    width: auto;
    height: 100%;
    padding: 12px 6px;
    border-bottom: none;
    border-right: 1px solid var(--border-subtle);
    flex-direction: column;
  }

  .context-collapsed:hover {
    background: var(--bg-hover);
  }

  .context-collapsed:hover .context-collapsed-icon {
    color: var(--text-primary);
  }

  .context-collapsed-icon {
    color: var(--text-muted);
    flex-shrink: 0;
    transition: color 0.1s ease;
  }

  .context-collapsed-label {
    font-family: var(--font-mono);
    font-size: 9px;
    font-weight: 600;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  .context-collapsed-count {
    font-family: var(--font-mono);
    font-size: 9px;
    color: var(--text-faint);
    font-variant-numeric: tabular-nums;
  }

  /* Toolbar */
  .toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 6px var(--space-md);
    background: var(--bg-surface);
    border-bottom: 1px solid var(--border-subtle);
  }

  .toolbar-info {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
    flex: 1;
    min-width: 0;
  }

  .toolbar-info strong {
    color: var(--text-primary);
    font-weight: 600;
  }

  .toolbar-sep {
    margin: 0 4px;
    opacity: 0.4;
  }

  .minimap-show-btn {
    opacity: 0.4;
    transition: opacity 0.1s ease;
  }

  .minimap-show-btn:hover {
    opacity: 1;
  }

  .toolbar-actions {
    display: flex;
    gap: 4px;
  }

  /* Zones container */
  .zones {
    flex: 1;
    overflow-y: auto;
    overflow-x: hidden;
    padding: var(--space-md);
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
    position: relative;
    scrollbar-width: thin;
    scrollbar-color: var(--border-default) transparent;
  }

  .zones::-webkit-scrollbar {
    width: 8px;
  }

  .zones::-webkit-scrollbar-track {
    background: transparent;
  }

  .zones::-webkit-scrollbar-thumb {
    background: var(--border-default);
    border-radius: 4px;
  }

  .zones::-webkit-scrollbar-thumb:hover {
    background: var(--text-muted);
  }

  .zones.resizing {
    user-select: none;
    cursor: row-resize;
  }

  /* Empty state */
  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    flex: 1;
    padding: var(--space-xl, 32px);
    text-align: center;
    gap: 12px;
    min-height: 300px;
  }

  .empty-icon {
    color: var(--text-faint);
    opacity: 0.5;
    margin-bottom: 4px;
  }

  .empty-title {
    font-family: var(--font-display);
    font-size: 16px;
    font-weight: 600;
    color: var(--text-secondary);
    margin: 0;
  }

  .empty-description {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--text-muted);
    max-width: 340px;
    line-height: 1.5;
    margin: 0;
  }

  .empty-actions {
    display: flex;
    gap: 8px;
    margin-top: 8px;
  }

  .empty-hints {
    display: flex;
    gap: 16px;
    margin-top: 12px;
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--text-faint);
  }

  .empty-hint {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  /* Terminal split handle */
  .terminal-split-handle {
    height: 4px;
    cursor: row-resize;
    background: transparent;
    transition: background 0.15s ease;
    flex-shrink: 0;
    position: relative;
  }

  .terminal-split-handle.terminal-split-vertical {
    height: auto;
    width: 4px;
    cursor: col-resize;
  }

  .terminal-split-handle:hover,
  .terminal-split-handle.active {
    background: var(--accent);
  }

  /* Terminal toggle button on split handle — matches sidebar toggle size */
  .terminal-toggle-btn {
    position: absolute;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    width: 32px;
    height: 16px;
    display: flex;
    align-items: center;
    justify-content: center;
    font-family: var(--font-mono);
    font-size: 14px;
    font-weight: 600;
    color: var(--text-muted);
    background: var(--bg-surface);
    border: 1px solid var(--border-subtle);
    border-radius: 3px;
    cursor: pointer;
    opacity: 0;
    transition: opacity 0.15s ease, color 0.1s ease;
    z-index: 5;
    padding: 0;
    line-height: 1;
  }

  .terminal-toggle-btn.terminal-toggle-vertical {
    width: 16px;
    height: 32px;
  }

  .terminal-split-handle:hover .terminal-toggle-btn {
    opacity: 1;
  }

  .terminal-toggle-btn:hover {
    color: var(--text-primary);
    border-color: var(--accent);
  }

  /* Terminal panel wrapper */
  .terminal-panel-wrapper {
    flex-shrink: 0;
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }

  .terminal-panel-wrapper.terminal-panel-right {
    height: 100%;
  }

  .terminal-panel-wrapper.terminal-fill {
    flex: 1;
    min-height: 0;
    min-width: 0;
  }
</style>
