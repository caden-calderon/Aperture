export type LaunchProviderId = 'anthropic' | 'openai';
export type ContextMutationMode = 'proxy_mutable' | 'direct_read_only';

export const DIRECT_MODE_EDIT_BLOCK_REASON =
  'Codex was launched independently (not through Aperture). Relaunch from the terminal panel for full context control.';

export interface ProviderCapabilities {
  supportsUsage: boolean;
  supportsReasoning: boolean;
  supportsResumeId: boolean;
}

export interface ProviderAdapter {
  id: LaunchProviderId;
  label: string;
  quickLaunchLabel: string;
  command: string;
  transport: 'proxy' | 'direct_cli_bridge';
  startsCodexBridge: boolean;
  startupMarkers: readonly string[];
  capabilities: ProviderCapabilities;
}

const PROVIDER_ADAPTERS: Record<LaunchProviderId, ProviderAdapter> = {
  anthropic: {
    id: 'anthropic',
    label: 'Claude',
    quickLaunchLabel: 'Claude Code',
    command: 'claude',
    transport: 'proxy',
    startsCodexBridge: false,
    startupMarkers: [],
    capabilities: {
      supportsUsage: true,
      supportsReasoning: true,
      supportsResumeId: false,
    },
  },
  openai: {
    id: 'openai',
    label: 'Codex',
    quickLaunchLabel: 'Codex',
    command: 'codex',
    transport: 'proxy',
    startsCodexBridge: true,
    startupMarkers: ['OpenAI Codex', 'codex resume '],
    capabilities: {
      supportsUsage: true,
      supportsReasoning: true,
      supportsResumeId: true,
    },
  },
};

const ADAPTER_ORDER: LaunchProviderId[] = ['anthropic', 'openai'];

export function listProviderAdapters(): ProviderAdapter[] {
  return ADAPTER_ORDER.map((id) => PROVIDER_ADAPTERS[id]);
}

export function getProviderAdapter(id: LaunchProviderId): ProviderAdapter {
  return PROVIDER_ADAPTERS[id];
}

export function shouldStartCodexBridgeFromOutput(text: string): boolean {
  const adapter = PROVIDER_ADAPTERS.openai;
  return adapter.startupMarkers.some((marker) => text.includes(marker));
}

/**
 * Best-effort inference for provider launched manually in the shell.
 * Matches direct command invocations like:
 * - `claude`
 * - `claude-code --resume`
 * - `/usr/local/bin/codex resume ...`
 */
export function inferManualLaunchProvider(commandLine: string): LaunchProviderId | null {
  const firstToken = commandLine.trim().split(/\s+/)[0];
  if (!firstToken) return null;

  const binary = firstToken.split('/').pop()?.toLowerCase() ?? firstToken.toLowerCase();
  if (binary === 'claude' || binary === 'claude-code') return 'anthropic';
  if (binary === 'codex') return 'openai';
  return null;
}

export function formatProviderCapabilities(capabilities: ProviderCapabilities): string {
  const usage = capabilities.supportsUsage ? 'usage' : 'no-usage';
  const reasoning = capabilities.supportsReasoning ? 'reasoning' : 'no-reasoning';
  const resume = capabilities.supportsResumeId ? 'resume-id' : 'no-resume-id';
  return `${usage} · ${reasoning} · ${resume}`;
}

export function contextMutationModeForProvider(
  provider: LaunchProviderId | 'none'
): ContextMutationMode {
  if (provider === 'none') return 'proxy_mutable';
  return getProviderAdapter(provider).transport === 'proxy'
    ? 'proxy_mutable'
    : 'direct_read_only';
}

export function contextModeBadgeLabel(mode: ContextMutationMode): string {
  if (mode === 'direct_read_only') return 'Direct (Observe Only)';
  return 'Proxy (Full Control)';
}
