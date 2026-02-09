import { describe, expect, it } from "vitest";

import {
  getProviderAdapter,
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
  });

  it("only starts codex bridge for known startup markers", () => {
    expect(shouldStartCodexBridgeFromOutput("OpenAI Codex v0.1")).toBe(true);
    expect(shouldStartCodexBridgeFromOutput("Run `codex resume abc` to continue")).toBe(true);
    expect(shouldStartCodexBridgeFromOutput("Claude Code ready")).toBe(false);
  });
});

