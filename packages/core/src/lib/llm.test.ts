import { describe, it, expect, vi, beforeEach } from "vitest";

vi.mock("../transport/transport", () => ({
  invokeCommand: vi.fn(),
}));

import { invokeCommand } from "../transport/transport";
import {
  PROVIDERS,
  providerSlug,
  llmSetKey,
  llmListModels,
  llmSetSettings,
  type LlmSettings,
} from "./llm";

beforeEach(() => {
  vi.mocked(invokeCommand).mockReset().mockResolvedValue(undefined);
});

describe("llm lib", () => {
  it("maps each provider kind to its stable backend slug", () => {
    expect(providerSlug("anthropic")).toBe("anthropic");
    expect(providerSlug("openAi")).toBe("openai");
    expect(providerSlug("gemini")).toBe("gemini");
    expect(providerSlug("openAiCompatible")).toBe("openai-compatible");
  });

  it("lists the four providers, only the compatible one needing a base URL", () => {
    expect(PROVIDERS.map((p) => p.kind)).toEqual(["anthropic", "openAi", "gemini", "openAiCompatible"]);
    expect(PROVIDERS.filter((p) => p.needsBaseUrl).map((p) => p.kind)).toEqual(["openAiCompatible"]);
  });

  it("forwards commands with the expected argument shape", async () => {
    await llmSetKey("openAi", "sk-test");
    expect(invokeCommand).toHaveBeenCalledWith("llm_set_key", { provider: "openAi", key: "sk-test" });

    await llmListModels("gemini");
    expect(invokeCommand).toHaveBeenCalledWith("llm_list_models", { provider: "gemini" });

    const settings: LlmSettings = {
      defaultProvider: "anthropic",
      models: { anthropic: "claude-opus-4-8" },
      baseUrls: {},
      maxTokens: 4096,
    };
    await llmSetSettings(settings);
    expect(invokeCommand).toHaveBeenCalledWith("llm_set_settings", { settings });
  });
});
