/**
 * Connection Store
 * Manages proxy connection state and live event subscription.
 *
 * The proxy is embedded in the Tauri app and always running.
 * "Connected" means the proxy is ready. Activity tracking shows
 * whether a client (Claude Code, Codex) is actively sending requests.
 */

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { BlocksCapturedPayload } from "../utils/blockConvert";

// ============================================================================
// Types
// ============================================================================

export type ConnectionStatus = "disconnected" | "connecting" | "connected" | "active" | "error";

export type ProxyProvider = "anthropic" | "openai" | "unknown";

export interface StreamingState {
  requestId: string;
  bytesReceived: number;
  provider: ProxyProvider;
}

// ============================================================================
// State
// ============================================================================

let status = $state<ConnectionStatus>("disconnected");
let proxyAddress = $state("http://127.0.0.1:5400");
let activeProvider = $state<ProxyProvider>("unknown");
let activeModel = $state<string | null>(null);
let streaming = $state<StreamingState | null>(null);
let lastError = $state<string | null>(null);
let totalRequestsCaptured = $state(0);
let pendingHotPatches = $state(0);
let lastActivityTime = $state<number>(0);
let hasCapturedTraffic = $state(false);

/** Seconds of inactivity before transitioning from "active" → "connected" (idle). */
const IDLE_TIMEOUT_MS = 15_000;

// Event listener handles for cleanup
let unlistenFns: UnlistenFn[] = [];
let healthCheckInterval: ReturnType<typeof setInterval> | undefined;
let idleCheckInterval: ReturnType<typeof setInterval> | undefined;

// Callback for when the engine has finished processing (context_updated event)
let onContextUpdatedCb: (() => void) | null = null;
// Callback for session reset (when client disconnects and a new session starts)
let onSessionReset: (() => void) | null = null;

// ============================================================================
// Activity tracking
// ============================================================================

function markActive(): void {
  hasCapturedTraffic = true;
  lastActivityTime = Date.now();
  if (status !== "active") {
    status = "active";
  }
}

function checkIdle(): void {
  if (status !== "active") return;
  if (streaming !== null) return; // Still streaming, not idle

  const elapsed = Date.now() - lastActivityTime;
  if (elapsed >= IDLE_TIMEOUT_MS) {
    status = hasCapturedTraffic ? "connected" : "disconnected";
    activeModel = null;
    activeProvider = "unknown";
  }
}

// ============================================================================
// Connection lifecycle
// ============================================================================

async function connect(): Promise<void> {
  if (unlistenFns.length > 0 || idleCheckInterval || healthCheckInterval) {
    disconnect();
  }

  status = "connecting";
  lastError = null;

  try {
    // Get proxy address from backend
    proxyAddress = await invoke<string>("get_proxy_address");

    // Check if proxy is running
    const running = await invoke<boolean>("is_proxy_running");
    if (!running) {
      status = "error";
      lastError = "Proxy is not running";
      return;
    }

    // Subscribe to Tauri events
    await subscribeToEvents();

    // Start idle check polling (checks every 3s if we should go idle)
    startIdleCheck();
    startHealthCheck();
    await refreshHotPatchCount();

    // Proxy process is up, but we only show connected/idle after traffic is observed.
    status = "disconnected";
  } catch (e) {
    status = "error";
    lastError = e instanceof Error ? e.message : String(e);
  }
}

function disconnect(): void {
  // Clean up event listeners
  for (const unlisten of unlistenFns) {
    unlisten();
  }
  unlistenFns = [];

  // Stop checks
  if (healthCheckInterval) {
    clearInterval(healthCheckInterval);
    healthCheckInterval = undefined;
  }
  if (idleCheckInterval) {
    clearInterval(idleCheckInterval);
    idleCheckInterval = undefined;
  }

  clearSession();
}

/** Clear all session state (blocks, counters, model info). */
function clearSession(): void {
  totalRequestsCaptured = 0;
  activeModel = null;
  activeProvider = "unknown";
  streaming = null;
  lastActivityTime = 0;
  hasCapturedTraffic = false;
  pendingHotPatches = 0;
  status = "disconnected";
  invoke("clear_hot_patches").catch(() => {
    // Ignore when not running in Tauri
  });
  if (onSessionReset) {
    onSessionReset();
  }
}

async function subscribeToEvents(): Promise<void> {
  // Main event channel
  const unlistenMain = await listen<Record<string, unknown>>("aperture:events", (event) => {
    const payload = event.payload;
    const eventType = payload.type as string;

    switch (eventType) {
      case "request_captured":
        totalRequestsCaptured++;
        activeProvider = (payload.provider as ProxyProvider) ?? "unknown";
        markActive();
        // Refresh hot patch count — patches may have been applied
        refreshHotPatchCount();
        break;

      case "blocks_captured": {
        const data = payload as unknown as BlocksCapturedPayload;
        activeModel = data.model;
        activeProvider = (data.provider as ProxyProvider) ?? "unknown";
        markActive();
        break;
      }

      case "context_updated": {
        markActive();
        if (onContextUpdatedCb) onContextUpdatedCb();
        break;
      }

      case "response_complete":
        streaming = null;
        markActive();
        break;

      case "proxy_error":
        lastError = (payload.message as string) ?? "Unknown proxy error";
        break;
    }
  });
  unlistenFns.push(unlistenMain);

  // Streaming progress channel (high frequency, separate)
  const unlistenStream = await listen<Record<string, unknown>>(
    "aperture:stream-progress",
    (event) => {
      const payload = event.payload;
      if (payload.type === "response_streaming") {
        streaming = {
          requestId: payload.request_id as string,
          bytesReceived: payload.bytes_received as number,
          provider: activeProvider,
        };
        markActive();
      }
    }
  );
  unlistenFns.push(unlistenStream);
}

async function refreshHotPatchCount(): Promise<void> {
  try {
    pendingHotPatches = await invoke<number>("pending_hot_patch_count");
  } catch {
    // Not running in Tauri
  }
}

function startIdleCheck(): void {
  if (idleCheckInterval) {
    clearInterval(idleCheckInterval);
  }
  idleCheckInterval = setInterval(checkIdle, 3000);
}

async function checkHealth(): Promise<void> {
  try {
    const running = await invoke<boolean>("is_proxy_running");
    if (!running) {
      status = "error";
      lastError = "Proxy is not running";
      streaming = null;
      return;
    }

    if (status === "error" && lastError === "Proxy is not running") {
      lastError = null;
      status = hasCapturedTraffic ? "connected" : "disconnected";
    }
  } catch (e) {
    status = "error";
    lastError = e instanceof Error ? e.message : String(e);
  }
}

function startHealthCheck(): void {
  if (healthCheckInterval) {
    clearInterval(healthCheckInterval);
  }
  healthCheckInterval = setInterval(() => {
    void checkHealth();
  }, 5000);
}

// ============================================================================
// Export Store Interface
// ============================================================================

export const connectionStore = {
  get status() {
    return status;
  },
  get proxyAddress() {
    return proxyAddress;
  },
  get activeProvider() {
    return activeProvider;
  },
  get activeModel() {
    return activeModel;
  },
  get streaming() {
    return streaming;
  },
  get lastError() {
    return lastError;
  },
  get totalRequestsCaptured() {
    return totalRequestsCaptured;
  },
  get isConnected() {
    return status === "connected" || status === "active";
  },
  get isActive() {
    return status === "active";
  },
  get isStreaming() {
    return streaming !== null;
  },
  get pendingHotPatches() {
    return pendingHotPatches;
  },

  connect,
  disconnect,
  clearSession,
  refreshHotPatchCount,

  /** Register a callback for when the engine has finished processing blocks. */
  onContextUpdated(cb: () => void): void {
    onContextUpdatedCb = cb;
  },

  /** Register a callback for when a session is cleared/reset. */
  onSessionReset(cb: () => void): void {
    onSessionReset = cb;
  },
};
