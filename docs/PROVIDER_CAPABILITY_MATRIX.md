# Provider Capability Matrix

Current launch/parser capability baseline for Phase 1.5 modularity.

| Provider | Launch Mode(s) | Transport | Usage Metrics | Reasoning Visibility | Resume ID |
|---|---|---|---|---|---|
| Claude (`anthropic`) | Quick-launch, manual `claude` / `claude-code` | Proxy | Yes | Yes | No |
| Codex Direct (`openai`) | Quick-launch, manual `codex` | Direct CLI bridge | No (bridge path) | Yes | Yes |
| Codex Proxy (`openai_proxy`) | Quick-launch | Proxy | Yes | Yes | Yes |
| Gemini CLI (`gemini_cli`) | Planned | Adapter placeholder | Planned | Planned | Planned |
| OpenCode (`opencode`) | Planned | Adapter placeholder | Planned | Planned | Planned |
| KiloCode (`kilocode`) | Planned | Adapter placeholder | Planned | Planned | Planned |

## Notes
- Frontend adapter contract: `src/lib/utils/providerAdapters.ts`
- Backend parser adapter contract: `src-tauri/src/proxy/provider_adapter.rs`
- Provider-specific logic must stay inside adapter/parser boundaries.
