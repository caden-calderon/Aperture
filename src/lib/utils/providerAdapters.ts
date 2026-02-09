export type LaunchProviderId = 'anthropic' | 'openai' | 'openai_proxy';

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
    label: 'Codex Direct',
    quickLaunchLabel: 'Codex (ChatGPT Direct)',
    command: 'codex',
    transport: 'direct_cli_bridge',
    startsCodexBridge: true,
    startupMarkers: ['OpenAI Codex', 'codex resume '],
    capabilities: {
      supportsUsage: false,
      supportsReasoning: true,
      supportsResumeId: true,
    },
  },
  openai_proxy: {
    id: 'openai_proxy',
    label: 'Codex Proxy',
    quickLaunchLabel: 'Codex (API via Proxy)',
    command: 'codex',
    transport: 'proxy',
    startsCodexBridge: false,
    startupMarkers: [],
    capabilities: {
      supportsUsage: true,
      supportsReasoning: true,
      supportsResumeId: true,
    },
  },
};

const ADAPTER_ORDER: LaunchProviderId[] = ['anthropic', 'openai', 'openai_proxy'];

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
