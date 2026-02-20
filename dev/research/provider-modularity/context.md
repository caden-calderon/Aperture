# Provider Modularity Context - 2026-02-09

## Why This Exists
Recent Phase 1 completion included additional provider integration work (especially Codex direct bridging) that now needs a clean modularity pass before broader Phase 2 expansion.

## Current Snapshot
1. Phase 1 proxy core is complete.
2. Claude flow is stable through current app launch path.
3. Codex now works through a direct bridge approach (subscription-compatible path) instead of relying only on API proxy mode.
4. Status bar behavior and launch flow were recently fixed/improved.

## Key Observations
1. Codex subscription mode does not behave like API-key proxy mode and should be treated as a first-class direct CLI transport.
2. Claude and Codex differ in surfaced metadata/output shape; normalization must happen in provider parser boundaries.
3. User expects manual terminal launch (`claude`/`codex`) and quick-launch buttons to converge to the same event bridge behavior.

## What to Expose Later (Feature Backlog)
Shared possibilities across Claude/Codex and future providers:
1. model/session metadata extraction
2. launch/connect/disconnect reason codes
3. usage metrics where available
4. resumable session identifiers
5. provider capability flags (reasoning visibility, tool mode, stream granularity)

## Constraints
1. Keep provider-specific logic out of core context store/engine.
2. Keep stream path hot: avoid blocking work, avoid heavy parsing on forwarding path.
3. Preserve explicit error mapping (no panic-prone handling).
4. Maintain event bridge parity between quick-launch and manual terminal command launches.
