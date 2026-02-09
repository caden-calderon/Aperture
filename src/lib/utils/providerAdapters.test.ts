import { describe, expect, it } from "vitest";

import {
  formatProviderCapabilities,
  getProviderAdapter,
  inferManualLaunchProvider,
  listProviderAdapters,
  shouldStartCodexBridgeFromOutput,
} from "./providerAdapters";

describe("provider adapters", () => {
  it("provides stable ordered launch adapters", () => {
    const adapters = listProviderAdapters();
    expect(adapters.map((a) => a.id)).toEqual(["anthropic", "openai", "openai_proxy"]);
  });

  it("maps openai direct mode to bridge transport", () => {
    const adapter = getProviderAdapter("openai");
    expect(adapter.command).toBe("codex");
    expect(adapter.transport).toBe("direct_cli_bridge");
    expect(adapter.startsCodexBridge).toBe(true);
    expect(adapter.capabilities.supportsUsage).toBe(false);
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
});
