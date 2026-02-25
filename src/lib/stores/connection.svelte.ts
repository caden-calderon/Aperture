/**
 * Connection Store
 * Manages proxy connection state and live event subscription.
 *
 * The proxy runs as a standalone process (aperture-proxy) on port 5400.
 * "Connected" means the proxy is reachable. Activity tracking shows
 * whether a client (Claude Code, Codex) is actively sending requests.
 *
 * Events arrive via SSE from GET /_aperture/events.
 * Terminal commands stay as Tauri IPC (PTY is process-local).
 */

import { invokeProxy, isProxyHealthy, getProxyUrl, getEventsUrl } from "../api";
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

export interface ProviderUsageSnapshot {
  requestId: string;
  inputTokens: number | null;
  outputTokens: number | null;
  totalTokens: number | null;
  observedAt: number;
}

// ============================================================================
// State
// ============================================================================

let status = $state<ConnectionStatus>("disconnected");
let proxyAddress = $state(getProxyUrl());
let activeProvider = $state<ProxyProvider>("unknown");
let activeModel = $state<string | null>(null);
let streaming = $state<StreamingState | null>(null);
let lastError = $state<string | null>(null);
let totalRequestsCaptured = $state(0);
let pendingHotPatches = $state(0);
let lastProviderUsage = $state<ProviderUsageSnapshot | null>(null);
let lastActivityTime = $state<number>(0);
let hasCapturedTraffic = $state(false);

/** Seconds of inactivity before transitioning from "active" → idle and clearing blocks. */
const IDLE_TIMEOUT_MS = 15_000;

// SSE event source and cleanup handles
let eventSource: EventSource | null = null;
let healthCheckInterval: ReturnType<typeof setInterval> | undefined;
let idleCheckInterval: ReturnType<typeof setInterval> | undefined;

// Callback for when the engine has finished processing (context_updated event)
let onContextUpdatedCb: (() => void) | null = null;
// Callback for session reset (when client disconnects and a new session starts)
let onSessionResetCb: (() => void) | null = null;

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

    // Clear blocks when going idle — client likely exited
    if (onSessionResetCb) {
      onSessionResetCb();
    }
  }
}

// ============================================================================
// Connection lifecycle
// ============================================================================

async function connect(): Promise<void> {
  if (eventSource || idleCheckInterval || healthCheckInterval) {
    disconnect();
  }

  status = "connecting";
  lastError = null;
  proxyAddress = getProxyUrl();

  try {
    // Check if proxy is running via health endpoint
    const healthy = await isProxyHealthy();
    if (!healthy) {
      status = "error";
      lastError = "Proxy is not running";
      return;
    }

    // Subscribe to SSE events from the proxy
    subscribeToEvents();

    // Start idle check polling (checks every 3s if we should go idle)
    startIdleCheck();
    startHealthCheck();
    await refreshHotPatchCount();

    // Proxy is up, but only show connected/idle after traffic is observed.
    status = "disconnected";
  } catch (e) {
    status = "error";
    lastError = e instanceof Error ? e.message : String(e);
  }
}

function disconnect(): void {
  // Close SSE connection
  if (eventSource) {
    eventSource.close();
    eventSource = null;
  }

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
  lastProviderUsage = null;
  lastActivityTime = 0;
  hasCapturedTraffic = false;
  pendingHotPatches = 0;
  status = "disconnected";
  invokeProxy("clear_hot_patches").catch(() => {
    // Ignore when proxy is not reachable
  });
  if (onSessionResetCb) {
    onSessionResetCb();
  }
}

function subscribeToEvents(): void {
  const url = getEventsUrl();
  eventSource = new EventSource(url);

  // Default (unnamed) events — main channel
  eventSource.onmessage = (event) => {
    handleEvent(event.data);
  };

  // Named "stream-progress" events — high frequency
  eventSource.addEventListener("stream-progress", (event) => {
    handleStreamProgress((event as MessageEvent).data);
  });

  eventSource.onerror = () => {
    // EventSource auto-reconnects, but mark error state temporarily
    if (status !== "error") {
      lastError = "SSE connection interrupted (reconnecting...)";
    }
  };
}

function handleEvent(data: string): void {
  let payload: Record<string, unknown>;
  try {
    payload = JSON.parse(data);
  } catch {
    return;
  }

  const eventType = payload.type as string;

  switch (eventType) {
    case "request_captured":
      totalRequestsCaptured++;
      activeProvider = (payload.provider as ProxyProvider) ?? "unknown";
      markActive();
      refreshHotPatchCount();
      break;

    case "blocks_captured": {
      const capturedData = payload as unknown as BlocksCapturedPayload;
      activeModel = capturedData.model;
      activeProvider = (capturedData.provider as ProxyProvider) ?? "unknown";
      if (capturedData.input_tokens !== null || capturedData.output_tokens !== null) {
        const inputTokens = capturedData.input_tokens;
        const outputTokens = capturedData.output_tokens;
        lastProviderUsage = {
          requestId: capturedData.request_id,
          inputTokens,
          outputTokens,
          totalTokens: (inputTokens ?? 0) + (outputTokens ?? 0),
          observedAt: Date.now(),
        };
      }
      markActive();
      break;
    }

    case "context_updated": {
      markActive();
      if (onContextUpdatedCb) onContextUpdatedCb();
      break;
    }

    case "response_complete":
      if (typeof payload.tokens_used === "number") {
        const requestId = typeof payload.request_id === "string" ? payload.request_id : "unknown";
        const previous =
          lastProviderUsage && lastProviderUsage.requestId === requestId
            ? lastProviderUsage
            : null;
        lastProviderUsage = {
          requestId,
          inputTokens: previous?.inputTokens ?? null,
          outputTokens: previous?.outputTokens ?? null,
          totalTokens: payload.tokens_used as number,
          observedAt: Date.now(),
        };
      }
      streaming = null;
      markActive();
      break;

    case "proxy_error":
      lastError = (payload.message as string) ?? "Unknown proxy error";
      break;
  }
}

function handleStreamProgress(data: string): void {
  let payload: Record<string, unknown>;
  try {
    payload = JSON.parse(data);
  } catch {
    return;
  }

  if (payload.type === "response_streaming") {
    streaming = {
      requestId: payload.request_id as string,
      bytesReceived: payload.bytes_received as number,
      provider: activeProvider,
    };
    markActive();
  }
}

async function refreshHotPatchCount(): Promise<void> {
  try {
    pendingHotPatches = await invokeProxy<number>("pending_hot_patch_count");
  } catch {
    // Proxy not reachable
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
    const healthy = await isProxyHealthy();
    if (!healthy) {
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
  get lastProviderUsage() {
    return lastProviderUsage;
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
    onSessionResetCb = cb;
  },
};
