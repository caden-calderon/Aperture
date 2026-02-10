import { describe, expect, it } from "vitest";

import {
  contextModeBadgeLabel,
  contextMutationModeForProvider,
  formatProviderCapabilities,
  getProviderAdapter,
  inferManualLaunchProvider,
  listProviderAdapters,
  shouldStartCodexBridgeFromOutput,
} from "./providerAdapters";

describe("provider adapters", () => {
  it("provides stable ordered launch adapters", () => {
    const adapters = listProviderAdapters();
    expect(adapters.map((a) => a.id)).toEqual(["anthropic", "openai"]);
  });

  it("maps openai to proxy transport with codex bridge", () => {
    const adapter = getProviderAdapter("openai");
    expect(adapter.command).toBe("codex");
    expect(adapter.transport).toBe("proxy");
    expect(adapter.startsCodexBridge).toBe(true);
    expect(adapter.capabilities.supportsUsage).toBe(true);
  });

  it("only starts codex bridge for known startup markers", () => {
    expect(shouldStartCodexBridgeFromOutput("OpenAI Codex v0.1")).toBe(true);
    expect(shouldStartCodexBridgeFromOutput("Run `codex resume abc` to continue")).toBe(true);
    expect(shouldStartCodexBridgeFromOutput("Claude Code ready")).toBe(false);
  });

  it("infers provider for manual launch commands", () => {
    expect(inferManualLaunchProvider("claude")).toBe("anthropic");
    expect(inferManualLaunchProvider("claude-code --resume abc")).toBe("anthropic");
    expect(inferManualLaunchProvider("/usr/local/bin/codex resume foo")).toBe("openai");
    expect(inferManualLaunchProvider("echo codex")).toBeNull();
  });

  it("formats capability summary for UI labels", () => {
    expect(
      formatProviderCapabilities({
        supportsUsage: true,
        supportsReasoning: false,
        supportsResumeId: true,
      })
    ).toBe("usage · no-reasoning · resume-id");
  });

  it("maps all providers to proxy mutable context mode", () => {
    expect(contextMutationModeForProvider("openai")).toBe("proxy_mutable");
    expect(contextMutationModeForProvider("anthropic")).toBe("proxy_mutable");
    expect(contextMutationModeForProvider("none")).toBe("proxy_mutable");
  });

  it("formats mode badge labels", () => {
    expect(contextModeBadgeLabel("direct_read_only")).toBe("Direct (Observe Only)");
    expect(contextModeBadgeLabel("proxy_mutable")).toBe("Proxy (Full Control)");
  });
});
