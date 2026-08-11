import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";

vi.mock("../lib/notify", () => ({ notify: { success: vi.fn(), error: vi.fn(), info: vi.fn() } }));

const { llm, chat } = vi.hoisted(() => ({
  llm: {
    llmGetSettings: vi.fn(),
    llmSetSettings: vi.fn(),
    llmSetKey: vi.fn(),
    llmClearKey: vi.fn(),
    llmKeyStatus: vi.fn(),
    llmListModels: vi.fn(),
  },
  chat: { listAgents: vi.fn() },
}));
vi.mock("../lib/llm", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../lib/llm")>();
  return { ...actual, ...llm };
});
vi.mock("../lib/chat", () => chat);

import { AssistantSettingsSection } from "./AssistantSettingsSection";

beforeEach(() => {
  llm.llmGetSettings.mockReset().mockResolvedValue({
    defaultProvider: "anthropic",
    models: {},
    baseUrls: {},
    maxTokens: 4096,
  });
  llm.llmSetSettings.mockReset().mockResolvedValue(undefined);
  llm.llmSetKey.mockReset().mockResolvedValue(undefined);
  llm.llmClearKey.mockReset().mockResolvedValue(undefined);
  llm.llmKeyStatus.mockReset().mockResolvedValue([]);
  llm.llmListModels.mockReset().mockResolvedValue([]);
  chat.listAgents.mockReset().mockResolvedValue([
    { kind: "srelens", label: "srelens", available: false, path: null, version: null, installUrl: "", gated: false },
    { kind: "claude", label: "Claude Code", available: true, path: "/usr/bin/claude", version: null, installUrl: "x", gated: false },
    { kind: "codex", label: "Codex", available: false, path: null, version: null, installUrl: "https://codex", gated: false },
  ]);
});

describe("AssistantSettingsSection", () => {
  it("lists the four providers and the default-provider selector", async () => {
    render(<AssistantSettingsSection />);
    // Each label appears both as a card heading and a default-provider option.
    expect((await screen.findAllByText("Anthropic")).length).toBeGreaterThanOrEqual(2);
    expect(screen.getAllByText("OpenAI").length).toBeGreaterThanOrEqual(2);
    expect(screen.getAllByText("Google Gemini").length).toBeGreaterThanOrEqual(2);
    expect(screen.getAllByText("OpenAI-compatible").length).toBeGreaterThanOrEqual(2);
    // The default-provider <select>.
    expect(screen.getByRole("combobox")).toBeTruthy();
  });

  it("saves a key for a provider and refreshes key status", async () => {
    render(<AssistantSettingsSection />);
    await screen.findAllByText("Anthropic");

    // The first API-key input is Anthropic's.
    const keyInputs = screen.getAllByPlaceholderText(/paste api key/i);
    fireEvent.change(keyInputs[0], { target: { value: "sk-ant-123" } });
    fireEvent.click(screen.getAllByRole("button", { name: /save key/i })[0]);

    await waitFor(() => expect(llm.llmSetKey).toHaveBeenCalledWith("anthropic", "sk-ant-123"));
    // Key status is re-read after saving (once on mount, once after save).
    await waitFor(() => expect(llm.llmKeyStatus).toHaveBeenCalledTimes(2));
  });

  it("fetches models and lets one be chosen for a keyed provider", async () => {
    llm.llmKeyStatus.mockResolvedValue(["anthropic"]);
    llm.llmListModels.mockResolvedValue([{ id: "claude-opus-4-8", displayName: "Claude Opus 4.8" }]);
    render(<AssistantSettingsSection />);
    await screen.findAllByText("Anthropic");

    fireEvent.click(screen.getAllByRole("button", { name: /fetch models/i })[0]);
    expect(await screen.findByText("Claude Opus 4.8")).toBeTruthy();
  });

  it("shows the vendor CLIs with their install status", async () => {
    render(<AssistantSettingsSection />);
    // Claude is installed; Codex is not (with an install link). The native
    // agent is filtered out of this CLI list.
    expect(await screen.findByText("installed")).toBeTruthy();
    expect(screen.getByText(/how to install/i)).toBeTruthy();
    expect(screen.getByText("Coding agent CLIs")).toBeTruthy();
  });
});
