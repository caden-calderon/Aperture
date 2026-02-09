<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { Terminal } from "@xterm/xterm";
  import { FitAddon } from "@xterm/addon-fit";
  import { WebLinksAddon } from "@xterm/addon-web-links";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { terminalStore } from "$lib/stores/terminal.svelte";
  import { connectionStore } from "$lib/stores/connection.svelte";
  import { themeStore } from "$lib/stores/theme.svelte";
  import {
    getProviderAdapter,
    shouldStartCodexBridgeFromOutput,
    type LaunchProviderId,
  } from "$lib/utils/providerAdapters";

  let containerEl = $state<HTMLDivElement | null>(null);
  let terminal: Terminal | null = null;
  let fitAddon: FitAddon | null = null;
  let resizeObserver: ResizeObserver | null = null;
  let resizeTimeout: ReturnType<typeof setTimeout> | null = null;
  let unlistenOutput: UnlistenFn | null = null;
  let unlistenExit: UnlistenFn | null = null;
  let codexBridgeRunning = $state(false);
  let codexBridgeStarting = $state(false);
  let launchInFlight = $state(false);

  async function startCodexSubscriptionBridge(): Promise<void> {
    if (codexBridgeRunning || codexBridgeStarting) return;
    codexBridgeStarting = true;
    try {
      await invoke<boolean>('start_codex_subscription_bridge');
      codexBridgeRunning = true;
    } catch {
      // Ignore outside Tauri / unsupported runtime
    } finally {
      codexBridgeStarting = false;
    }
  }

  async function stopCodexSubscriptionBridge(): Promise<void> {
    if (!codexBridgeRunning && !codexBridgeStarting) return;
    try {
      await invoke<boolean>('stop_codex_subscription_bridge');
      codexBridgeRunning = false;
    } catch {
      // Ignore outside Tauri / unsupported runtime
    } finally {
      codexBridgeStarting = false;
    }
  }

  function maybeStartCodexBridgeFromOutput(text: string): void {
    if (codexBridgeRunning || codexBridgeStarting) return;
    if (shouldStartCodexBridgeFromOutput(text)) {
      // Manual `codex` launch should converge with quick-launch behavior.
      connectionStore.clearSession();
      terminalStore.setSelectedProvider('openai');
      terminalStore.setLaunchStatus('running');
      void startCodexSubscriptionBridge();
    }
  }

  /** Spawn a shell, optionally configured for a specific provider. */
  async function spawnShell(provider?: string, command?: string): Promise<boolean> {
    try {
      let id: string;
      if (provider && provider !== 'none') {
        // Provider-aware spawn: injects proxy env vars
        id = await invoke<string>('spawn_provider_shell', {
          provider,
          command: command ?? null,
        });
      } else {
        id = await invoke<string>('spawn_shell');
      }

      terminalStore.setSessionId(id);
      terminalStore.setExited(false);

      // Register listeners atomically to avoid partial listener leaks on failure.
      let nextUnlistenOutput: UnlistenFn | null = null;
      let nextUnlistenExit: UnlistenFn | null = null;
      try {
        nextUnlistenOutput = await listen<[string, string]>('terminal:output', (event) => {
          const [sid, data] = event.payload;
          if (sid === terminalStore.sessionId && terminal) {
            terminal.write(data);
            maybeStartCodexBridgeFromOutput(data);
          }
        });

        nextUnlistenExit = await listen<string>('terminal:exit', (event) => {
          if (event.payload === terminalStore.sessionId) {
            terminalStore.setExited(true);
            terminalStore.setLaunchStatus('idle');
            terminalStore.setSelectedProvider('none');
            connectionStore.clearSession();
            void stopCodexSubscriptionBridge();
            terminal?.write('\r\n\x1b[90m[Process exited — press Enter to restart]\x1b[0m\r\n');
          }
        });
      } catch (listenerError) {
        nextUnlistenOutput?.();
        nextUnlistenExit?.();
        await invoke('kill_session', { sessionId: id }).catch(() => {});
        terminalStore.setSessionId(null);
        throw listenerError;
      }

      unlistenOutput = nextUnlistenOutput;
      unlistenExit = nextUnlistenExit;
      return true;
    } catch (e) {
      console.error('Failed to spawn shell:', e);
      terminalStore.setLaunchStatus('error');
      terminal?.write(`\r\n\x1b[31mFailed to spawn shell: ${e}\x1b[0m\r\n`);
      return false;
    }
  }

  /**
   * Launch a provider CLI:
   * - anthropic: Claude via proxy
   * - openai: Codex direct (ChatGPT subscription mode)
   * - openai_proxy: Codex via Aperture proxy (API key/scopes required)
   */
  export async function launchProvider(provider: LaunchProviderId) {
    if (launchInFlight) return;
    launchInFlight = true;

    // Provider switch = new client session; clear old live context first.
    connectionStore.clearSession();

    try {
      // Kill existing session first
      cleanup();
      terminal?.clear();

      const adapter = getProviderAdapter(provider);
      terminalStore.setSelectedProvider(provider);
      terminalStore.setLaunchStatus('launching');

      if (adapter.startsCodexBridge) {
        await startCodexSubscriptionBridge();
      } else {
        await stopCodexSubscriptionBridge();
      }

      const ok = await spawnShell(provider, adapter.command);
      terminalStore.setLaunchStatus(ok ? 'running' : 'error');
      if (!ok) {
        terminalStore.setSelectedProvider('none');
      }
    } finally {
      launchInFlight = false;
    }
  }

  function doFit() {
    if (!fitAddon || !terminal || !containerEl) return;
    try {
      fitAddon.fit();
      const sessionId = terminalStore.sessionId;
      if (sessionId && terminal.cols && terminal.rows) {
        invoke('resize_terminal', {
          sessionId,
          cols: terminal.cols,
          rows: terminal.rows,
        }).catch(() => {});
      }
    } catch {
      // Ignore fit errors during rapid resize
    }
  }

  onMount(() => {
    if (!containerEl) return;

    terminal = new Terminal({
      fontFamily: "'JetBrains Mono', 'Fira Code', monospace",
      fontSize: 13,
      lineHeight: 1.3,
      cursorBlink: true,
      cursorStyle: 'block',
      allowProposedApi: true,
      theme: themeStore.getXtermTheme(),
    });

    fitAddon = new FitAddon();
    terminal.loadAddon(fitAddon);
    terminal.loadAddon(new WebLinksAddon());
    terminal.open(containerEl);

    // Initial fit
    requestAnimationFrame(() => doFit());

    // Keyboard input → PTY
    terminal.onData((data) => {
      if (terminalStore.isExited) {
        // Restart on Enter
        if (data === '\r') {
          cleanup();
          terminal?.clear();
          terminalStore.setSelectedProvider('none');
          void spawnShell();
        }
        return;
      }
      const sessionId = terminalStore.sessionId;
      if (sessionId) {
        invoke('send_input', { sessionId, data }).catch(() => {});
      }
    });

    // ResizeObserver with debounce
    resizeObserver = new ResizeObserver(() => {
      if (resizeTimeout) clearTimeout(resizeTimeout);
      resizeTimeout = setTimeout(() => doFit(), 50);
    });
    resizeObserver.observe(containerEl);

    void spawnShell();
  });

  function cleanup() {
    unlistenOutput?.();
    unlistenOutput = null;
    unlistenExit?.();
    unlistenExit = null;

    const sessionId = terminalStore.sessionId;
    if (sessionId) {
      invoke('kill_session', { sessionId }).catch(() => {});
      terminalStore.setSessionId(null);
    }
  }

  onDestroy(() => {
    cleanup();
    void stopCodexSubscriptionBridge();
    if (resizeTimeout) clearTimeout(resizeTimeout);
    resizeObserver?.disconnect();
    terminal?.dispose();
    terminal = null;
    fitAddon = null;
  });

  export function focus() {
    terminal?.focus();
  }

  export function updateTheme() {
    if (terminal) {
      terminal.options.theme = themeStore.getXtermTheme();
    }
  }

  export function clear() {
    terminal?.clear();
  }
</script>

<div class="terminal-container" bind:this={containerEl}></div>

<style>
  .terminal-container {
    width: 100%;
    height: 100%;
    overflow: hidden;
    /* Match xterm theme background so cell-grid gaps are invisible */
    background: var(--bg-base);
  }

  /* Override xterm.js defaults to fill container */
  .terminal-container :global(.xterm) {
    padding: 2px 0 0 4px;
    height: 100%;
  }

  .terminal-container :global(.xterm-screen) {
    width: 100% !important;
    height: 100% !important;
  }

  .terminal-container :global(.xterm-viewport) {
    scrollbar-width: thin;
    scrollbar-color: var(--border-default) transparent;
  }

  .terminal-container :global(.xterm-viewport::-webkit-scrollbar) {
    width: 6px;
  }

  .terminal-container :global(.xterm-viewport::-webkit-scrollbar-track) {
    background: transparent;
  }

  .terminal-container :global(.xterm-viewport::-webkit-scrollbar-thumb) {
    background: var(--border-default);
    border-radius: 3px;
  }
</style>
