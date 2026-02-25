import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("$lib/api", () => ({
  invokeProxy: vi.fn().mockResolvedValue(null),
}));

import { contextStore } from "./context.svelte";

describe("compression settings store", () => {
  beforeEach(() => {
    contextStore.init();
  });

  it("starts with phase-4 sidekick defaults", () => {
    expect(contextStore.compressionSettings.backend).toBe("auto");
    expect(contextStore.compressionSettings.model).toBeNull();
    expect(contextStore.compressionSettings.timeoutMs).toBe(12000);
    expect(contextStore.compressionSettings.maxTokens).toBe(512);
    expect(contextStore.compressionSettings.openrouterApiKeyEnv).toBe("OPENROUTER_API_KEY");
  });

  it("updateCompressionSettings merges and clamps values", async () => {
    await contextStore.updateCompressionSettings({
      backend: "open_router",
      timeoutMs: 1,
      maxTokens: 90_000,
      model: "  google/gemini-2.0-flash-001  ",
      openrouterBaseUrl: "  https://openrouter.ai/api/v1  ",
    });

    const settings = contextStore.compressionSettings;
    expect(settings.backend).toBe("open_router");
    expect(settings.timeoutMs).toBe(500);
    expect(settings.maxTokens).toBe(8192);
    expect(settings.model).toBe("google/gemini-2.0-flash-001");
    expect(settings.openrouterBaseUrl).toBe("https://openrouter.ai/api/v1");
  });

  it("setCompressionSettings accepts disabled backend", async () => {
    await contextStore.setCompressionSettings({
      backend: "disabled",
      model: null,
      timeoutMs: 5000,
      maxTokens: 320,
      openrouterBaseUrl: null,
      openrouterApiKeyEnv: null,
    });

    const settings = contextStore.compressionSettings;
    expect(settings.backend).toBe("disabled");
    expect(settings.model).toBeNull();
    expect(settings.timeoutMs).toBe(5000);
    expect(settings.maxTokens).toBe(320);
  });
});
