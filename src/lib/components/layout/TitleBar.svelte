<script lang="ts">
  import { contextStore, zonesStore, connectionStore } from "$lib/stores";

  let showStatus = $state(false);

  function formatNumber(n: number): string {
    return n.toLocaleString();
  }

  // Window controls (Tauri APIs)
  async function minimize() {
    const { getCurrentWindow } = await import('@tauri-apps/api/window');
    await getCurrentWindow().minimize();
  }

  async function toggleMaximize() {
    const { getCurrentWindow } = await import('@tauri-apps/api/window');
    const win = getCurrentWindow();
    if (await win.isMaximized()) {
      await win.unmaximize();
    } else {
      await win.maximize();
    }
  }

  async function close() {
    const { getCurrentWindow } = await import('@tauri-apps/api/window');
    await getCurrentWindow().close();
  }
</script>

<div class="titlebar-wrapper">
  <div class="titlebar" data-tauri-drag-region>
    <div class="titlebar-left">
      <div class="logo">
        <span class="logo-mark">A</span>
      </div>
    </div>

    <div class="titlebar-center" data-tauri-drag-region>
      <button
        class="title-trigger"
        class:open={showStatus}
        onclick={() => { showStatus = !showStatus; }}
        title="Toggle status panel"
      >
        <span class="titlebar-title">Aperture</span>
        <svg class="title-chevron" width="8" height="8" viewBox="0 0 8 8" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
          <path d="M2 3L4 5L6 3" />
        </svg>
      </button>
    </div>

    <div class="titlebar-right">
      <button class="window-btn" onclick={minimize} title="Minimize">
        <svg width="10" height="10" viewBox="0 0 10 10">
          <rect x="0" y="4" width="10" height="2" fill="currentColor"/>
        </svg>
      </button>
      <button class="window-btn" onclick={toggleMaximize} title="Maximize">
        <svg width="10" height="10" viewBox="0 0 10 10">
          <rect x="0" y="0" width="10" height="10" fill="none" stroke="currentColor" stroke-width="1.5"/>
        </svg>
      </button>
      <button class="window-btn window-btn-close" onclick={close} title="Close">
        <svg width="10" height="10" viewBox="0 0 10 10">
          <path d="M1 1L9 9M9 1L1 9" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>
        </svg>
      </button>
    </div>
  </div>

  {#if showStatus}
    <div class="status-dropdown">
      <span class="status-indicator" class:status-active={connectionStore.isActive} class:status-idle={connectionStore.status === 'connected'} title="Proxy {connectionStore.status}">
        <span class="status-dot"></span>
        <span class="status-label">{connectionStore.isActive ? "ACTIVE" : connectionStore.status === "connected" ? "IDLE" : connectionStore.status.toUpperCase()}{connectionStore.isStreaming ? " (streaming)" : ""}</span>
      </span>
      <span class="status-sep"></span>
      <span class="status-stat" title="Total blocks">
        <strong>{contextStore.blocks.length}</strong> blocks
      </span>
      <span class="status-sep"></span>
      <span class="status-stat" title="Active zones">
        <strong>{zonesStore.zonesByDisplayOrder.length}</strong> zones
      </span>
      <span class="status-sep"></span>
      <span class="status-stat" title="Total tokens">
        <strong>{formatNumber(contextStore.tokenBudget.used)}</strong> / {formatNumber(contextStore.tokenLimit)} tokens
      </span>
      {#if connectionStore.totalRequestsCaptured > 0}
        <span class="status-sep"></span>
        <span class="status-stat" title="Requests captured">
          <strong>{connectionStore.totalRequestsCaptured}</strong> captured
        </span>
      {/if}
      {#if connectionStore.activeModel}
        <span class="status-sep"></span>
        <span class="status-stat" title="Active model">
          {connectionStore.activeModel}
        </span>
      {/if}
      {#if connectionStore.pendingHotPatches > 0}
        <span class="status-sep"></span>
        <span class="status-stat status-hot-patch" title="Pending hot patches — will apply on next request">
          <strong>{connectionStore.pendingHotPatches}</strong> patch{connectionStore.pendingHotPatches === 1 ? '' : 'es'}
        </span>
      {/if}
    </div>
  {/if}
</div>

<style>
  .titlebar-wrapper {
    flex-shrink: 0;
    position: relative;
    z-index: 20;
  }

  .titlebar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: 32px;
    background: var(--bg-surface);
    border-bottom: 1px solid var(--border-subtle);
    padding: 0 8px;
    user-select: none;
    -webkit-user-select: none;
  }

  .titlebar-left,
  .titlebar-right {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100px;
  }

  .titlebar-right {
    justify-content: flex-end;
  }

  .titlebar-center {
    flex: 1;
    display: flex;
    justify-content: center;
    align-items: center;
  }

  .title-trigger {
    display: flex;
    align-items: center;
    gap: 4px;
    background: none;
    border: none;
    cursor: pointer;
    padding: 2px 8px;
    border-radius: 4px;
    transition: background 0.1s ease;
  }

  .title-trigger:hover {
    background: var(--bg-hover);
  }

  .titlebar-title {
    font-family: var(--font-display);
    font-size: 12px;
    font-weight: 500;
    color: var(--text-secondary);
    letter-spacing: -0.01em;
  }

  .title-chevron {
    color: var(--text-muted);
    transition: transform 0.15s ease;
    flex-shrink: 0;
  }

  .title-trigger.open .title-chevron {
    transform: rotate(180deg);
  }

  .logo {
    display: flex;
    align-items: center;
  }

  .logo-mark {
    width: 20px;
    height: 20px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--text-primary);
    color: var(--bg-surface);
    font-family: var(--font-display);
    font-size: 11px;
    font-weight: 600;
    border-radius: 3px;
  }

  /* Status dropdown panel — floats over content */
  .status-dropdown {
    position: absolute;
    top: 100%;
    left: 50%;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 4px 16px;
    background: var(--bg-elevated);
    border: 1px solid var(--border-subtle);
    border-top: none;
    border-radius: 0 0 6px 6px;
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--text-muted);
    box-shadow: 0 4px 12px color-mix(in srgb, var(--bg-base) 40%, transparent);
    animation: status-drop 0.12s ease-out;
    white-space: nowrap;
  }

  @keyframes status-drop {
    from {
      opacity: 0;
      transform: translateX(-50%) translateY(-4px);
    }
    to {
      opacity: 1;
      transform: translateX(-50%) translateY(0);
    }
  }

  .status-sep {
    width: 3px;
    height: 3px;
    border-radius: 50%;
    background: var(--text-muted);
    opacity: 0.3;
    flex-shrink: 0;
  }

  .status-stat {
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .status-stat strong {
    color: var(--text-secondary);
    font-weight: 600;
  }

  .status-indicator {
    display: flex;
    align-items: center;
    gap: 5px;
  }

  .status-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--semantic-danger);
    flex-shrink: 0;
  }

  .status-indicator.status-active .status-dot {
    background: var(--semantic-success);
    box-shadow: 0 0 4px color-mix(in srgb, var(--semantic-success) 50%, transparent);
  }

  .status-indicator.status-idle .status-dot {
    background: var(--text-muted);
  }

  .status-label {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.3px;
  }

  .window-btn {
    width: 24px;
    height: 24px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: none;
    border-radius: 4px;
    color: var(--text-muted);
    cursor: pointer;
    transition: all 0.1s ease;
  }

  .window-btn:hover {
    background: var(--bg-hover);
    color: var(--text-primary);
  }

  .window-btn-close:hover {
    background: #e53935;
    color: white;
  }

  .status-hot-patch {
    color: var(--semantic-warning);
  }

  .status-hot-patch strong {
    color: var(--semantic-warning);
  }
</style>
