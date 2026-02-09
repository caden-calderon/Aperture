export type LaunchProviderId = 'anthropic' | 'openai' | 'openai_proxy';

export interface ProviderAdapter {
  id: LaunchProviderId;
  label: string;
  quickLaunchLabel: string;
  command: string;
  transport: 'proxy' | 'direct_cli_bridge';
  startsCodexBridge: boolean;
  startupMarkers: readonly string[];
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
  },
  openai: {
    id: 'openai',
    label: 'Codex Direct',
    quickLaunchLabel: 'Codex (ChatGPT Direct)',
    command: 'codex',
    transport: 'direct_cli_bridge',
    startsCodexBridge: true,
    startupMarkers: ['OpenAI Codex', 'codex resume '],
  },
  openai_proxy: {
    id: 'openai_proxy',
    label: 'Codex Proxy',
    quickLaunchLabel: 'Codex (API via Proxy)',
    command: 'codex',
    transport: 'proxy',
    startsCodexBridge: false,
    startupMarkers: [],
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

