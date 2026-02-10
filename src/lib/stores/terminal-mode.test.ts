import { beforeEach, describe, expect, it } from "vitest";

import { terminalStore } from "./terminal.svelte";

describe("terminal context mode state", () => {
  beforeEach(() => {
    terminalStore.setSelectedProvider("none");
    terminalStore.setLaunchStatus("idle");
  });

  it("reports proxy mutable mode for codex provider", () => {
    terminalStore.setSelectedProvider("openai");

    expect(terminalStore.contextMutationMode).toBe("proxy_mutable");
    expect(terminalStore.contextModeLabel).toBe("Proxy (Mutable)");
  });

  it("reports proxy mutable mode for all providers and shell", () => {
    terminalStore.setSelectedProvider("anthropic");
    expect(terminalStore.contextMutationMode).toBe("proxy_mutable");
    expect(terminalStore.contextModeLabel).toBe("Proxy (Mutable)");

    terminalStore.setSelectedProvider("openai");
    expect(terminalStore.contextMutationMode).toBe("proxy_mutable");
    expect(terminalStore.contextModeLabel).toBe("Proxy (Mutable)");

    terminalStore.setSelectedProvider("none");
    expect(terminalStore.contextMutationMode).toBe("proxy_mutable");
    expect(terminalStore.contextModeLabel).toBe("Proxy (Mutable)");
  });
});
