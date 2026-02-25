/**
 * HTTP transport layer for communicating with the standalone Aperture proxy.
 *
 * Replaces Tauri's `invoke()` for engine/hot-patch commands.
 * Terminal commands still use Tauri IPC (PTY is process-local).
 */

const PROXY_URL = "http://127.0.0.1:5400";

/**
 * Call an IPC command on the proxy via HTTP.
 *
 * Equivalent to Tauri's `invoke(command, args)` but over HTTP.
 */
export async function invokeProxy<T = unknown>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  const res = await fetch(`${PROXY_URL}/_aperture/ipc/${command}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(args ?? {}),
  });

  if (!res.ok) {
    const body = await res.text();
    throw new Error(`IPC ${command} failed (${res.status}): ${body}`);
  }

  const text = await res.text();
  if (!text) return null as T;
  return JSON.parse(text) as T;
}

/**
 * Check if the proxy is healthy.
 */
export async function isProxyHealthy(): Promise<boolean> {
  try {
    const res = await fetch(`${PROXY_URL}/_aperture/health`, {
      signal: AbortSignal.timeout(1000),
    });
    return res.ok;
  } catch {
    return false;
  }
}

/**
 * Get the proxy URL.
 */
export function getProxyUrl(): string {
  return PROXY_URL;
}

/**
 * SSE event source URL for real-time events.
 */
export function getEventsUrl(): string {
  return `${PROXY_URL}/_aperture/events`;
}
