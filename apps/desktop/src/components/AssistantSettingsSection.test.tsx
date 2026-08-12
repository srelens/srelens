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
  it("lists the four providers as rows with a default-provider radio each", async () => {
    render(<AssistantSettingsSection />);
    expect(await screen.findByText("Anthropic")).toBeTruthy();
    expect(screen.getByText("OpenAI")).toBeTruthy();
    expect(screen.getByText("Google Gemini")).toBeTruthy();
    expect(screen.getByText("OpenAI-compatible")).toBeTruthy();
    // One radio per provider; the stored default is checked.
    expect(screen.getAllByRole("radio")).toHaveLength(4);
    expect(
      (screen.getByRole("radio", { name: /use anthropic as the default/i }) as HTMLInputElement).checked,
    ).toBe(true);
  });

  it("expands only the default provider initially, and one row at a time", async () => {
    render(<AssistantSettingsSection />);
    await screen.findByText("Anthropic");
    // Only the default (Anthropic) row is open → exactly one key input.
    expect(screen.getAllByPlaceholderText(/paste api key/i)).toHaveLength(1);

    // Opening Gemini collapses Anthropic — still exactly one key input.
    fireEvent.click(screen.getByRole("button", { name: /google gemini/i }));
    expect(screen.getAllByPlaceholderText(/paste api key/i)).toHaveLength(1);
    expect(screen.getByRole("button", { name: /google gemini/i }).getAttribute("aria-expanded")).toBe(
      "true",
    );
    expect(screen.getByRole("button", { name: /anthropic/i }).getAttribute("aria-expanded")).toBe(
      "false",
    );
  });

  it("summarizes each provider's state on its row", async () => {
    llm.llmKeyStatus.mockResolvedValue(["anthropic", "gemini"]);
    llm.llmGetSettings.mockResolvedValue({
      defaultProvider: "anthropic",
      models: { gemini: "gemini-2.5-pro" },
      baseUrls: {},
      maxTokens: 4096,
    });
    render(<AssistantSettingsSection />);
    await screen.findByText("Anthropic");
    // Key but no model: the blocking step is named.
    expect(await screen.findByText(/key set — choose a model/)).toBeTruthy();
    // Key and model: both shown.
    expect(screen.getByText(/key set · gemini-2\.5-pro/)).toBeTruthy();
    // No key at all.
    expect(screen.getAllByText("no key").length).toBeGreaterThanOrEqual(1);
  });

  it("saves a key for the expanded provider and refreshes key status", async () => {
    render(<AssistantSettingsSection />);
    await screen.findByText("Anthropic");

    // The default (Anthropic) row is the expanded one.
    fireEvent.change(screen.getByPlaceholderText(/paste api key/i), { target: { value: "sk-ant-123" } });
    fireEvent.click(screen.getByRole("button", { name: /save key/i }));

    await waitFor(() => expect(llm.llmSetKey).toHaveBeenCalledWith("anthropic", "sk-ant-123"));
    // Key status is re-read after saving (once on mount, once after save).
    await waitFor(() => expect(llm.llmKeyStatus).toHaveBeenCalledTimes(2));
  });

  it("fetches models and lets one be chosen for a keyed provider", async () => {
    llm.llmKeyStatus.mockResolvedValue(["anthropic"]);
    llm.llmListModels.mockResolvedValue([{ id: "claude-opus-4-8", displayName: "Claude Opus 4.8" }]);
    render(<AssistantSettingsSection />);
    await screen.findByText("Anthropic");

    fireEvent.click(screen.getByRole("button", { name: /fetch models/i }));
    expect(await screen.findByText("Claude Opus 4.8")).toBeTruthy();
  });

  it("picking a radio changes the default provider saved with settings", async () => {
    render(<AssistantSettingsSection />);
    await screen.findByText("Anthropic");
    fireEvent.click(screen.getByRole("radio", { name: /use google gemini as the default/i }));
    fireEvent.click(screen.getByRole("button", { name: /save settings/i }));
    await waitFor(() =>
      expect(llm.llmSetSettings).toHaveBeenCalledWith(
        expect.objectContaining({ defaultProvider: "gemini" }),
      ),
    );
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
