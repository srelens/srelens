import { describe, it, expect, vi, beforeEach } from "vitest";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const core = vi.hoisted(() => ({
  llmGetSettings: vi.fn(),
  llmSetSettings: vi.fn(),
  llmSetKey: vi.fn(),
  llmClearKey: vi.fn(),
  llmKeyStatus: vi.fn(),
  llmListModels: vi.fn(),
  listAgents: vi.fn(),
}));
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  ...core,
}));

import { AgentPane } from "./AgentPane";

const { llmGetSettings, llmSetSettings, llmSetKey, llmClearKey, llmKeyStatus, llmListModels, listAgents } =
  core;

/** A settings record with a chosen model, so a provider row can read "key set
 * · <model>" rather than the bare "key set" state. */
const SETTINGS = {
  defaultProvider: "anthropic" as const,
  models: { anthropic: "claude-opus" },
  baseUrls: {},
  maxTokens: 4096,
};

describe("AgentPane", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    llmGetSettings.mockResolvedValue(SETTINGS);
    llmSetSettings.mockResolvedValue(undefined);
    llmSetKey.mockResolvedValue(undefined);
    llmClearKey.mockResolvedValue(undefined);
    // A key already on "anthropic" by default, so the row that starts
    // expanded (the default provider) has a clickable Fetch models button.
    llmKeyStatus.mockResolvedValue(["anthropic"]);
    llmListModels.mockResolvedValue([]);
    listAgents.mockResolvedValue([]);
  });

  it("never prints a stored key, only that there is one", async () => {
    llmKeyStatus.mockResolvedValue(["anthropic"]);
    render(<AgentPane />);
    await screen.findByText(/anthropic/i);
    expect(document.body.textContent).not.toMatch(/sk-/);
    expect(screen.getByText(/key set/i)).toBeTruthy();
  });

  it("lists the CLIs srelens can drive, and marks the ones it cannot yet", async () => {
    listAgents.mockResolvedValue([
      { kind: "claude", label: "Claude", available: true, path: "/c", version: "1.2", installUrl: "", gated: false },
      { kind: "codex", label: "Codex", available: false, path: null, version: null, installUrl: "https://x", gated: false },
    ]);
    render(<AgentPane />);
    expect(await screen.findByText("1.2")).toBeTruthy();
    expect(screen.getByText(/not installed/i)).toBeTruthy();
  });

  it("reports a provider failure in the provider's terms, never the cluster's", async () => {
    llmListModels.mockRejectedValue("ApiError: 401 Unauthorized");
    render(<AgentPane />);
    await userEvent.click(await screen.findByRole("button", { name: /fetch models/i }));
    expect(await screen.findByText(/could not fetch models/i)).toBeTruthy();
    // `describeError`'s 401 branch is cluster vocabulary (issue #383). An
    // Anthropic key that was refused has no kubeconfig and no client
    // certificate, so neither word may reach this pane.
    expect(document.body.textContent).not.toMatch(/kubeconfig/i);
    expect(document.body.textContent).not.toMatch(/rejected your credentials/i);
    // The original is still one disclosure away, as everywhere else.
    expect(screen.getByText(/401 Unauthorized/)).toBeTruthy();
  });

  // --- Additional coverage: the three-state rule this plan keeps re-finding
  // violated, plus the two other core calls (`llmClearKey`, `llmSetSettings`)
  // the brief lists as consumed but the three given tests never exercise. ---

  it("waits rather than claiming no provider has a key while the read is in flight", async () => {
    let resolveStatus!: (v: string[]) => void;
    llmKeyStatus.mockReturnValue(new Promise((res) => (resolveStatus = res)));
    render(<AgentPane />);

    // Still in flight: neither "no key" nor "key set" is a fact this pane has
    // yet, so the panel must say it's checking rather than assert an answer.
    expect(await screen.findByText(/checking configured providers/i)).toBeTruthy();
    expect(screen.queryByText("no key")).toBeNull();
    expect(screen.queryByText(/key set/i)).toBeNull();

    // `llmGetSettings` (the OTHER read this panel waits on) is a plain
    // resolved promise from `beforeEach`, so awaiting the exact instance the
    // pane called flushes its `.then()` — and the state update it schedules —
    // without depending on any DOM signal that update produces. This
    // specifically covers the gap where settings are already known and the
    // key status alone is still outstanding: a pane keyed only off the
    // settings read would drop the "checking" notice right here, before it
    // has any actual key-status fact to replace it with.
    await act(async () => {
      await llmGetSettings.mock.results[0]?.value;
    });
    expect(screen.getByText(/checking configured providers/i)).toBeTruthy();
    expect(screen.queryByText("no key")).toBeNull();

    resolveStatus(["anthropic"]);
    expect(await screen.findByText(/key set/i)).toBeTruthy();
  });

  it("waits rather than claiming no agent CLIs are installed while the read is in flight", async () => {
    let resolveAgents!: (v: unknown[]) => void;
    listAgents.mockReturnValue(new Promise((res) => (resolveAgents = res)));
    render(<AgentPane />);

    expect(await screen.findByText(/checking installed agent clis/i)).toBeTruthy();
    expect(screen.queryByText(/not installed/i)).toBeNull();

    resolveAgents([
      { kind: "codex", label: "Codex", available: false, path: null, version: null, installUrl: "", gated: false },
    ]);
    expect(await screen.findByText(/not installed/i)).toBeTruthy();
  });

  it("removes a provider's key through llmClearKey and reflects it has none", async () => {
    llmKeyStatus.mockResolvedValueOnce(["anthropic"]).mockResolvedValueOnce([]);
    render(<AgentPane />);

    await screen.findByText(/key set/i);
    const anthropicRow = screen.getByTestId("provider-row-anthropic");
    await userEvent.click(within(anthropicRow).getByRole("button", { name: "Remove key" }));

    expect(llmClearKey).toHaveBeenCalledWith("anthropic");
    await waitFor(() => expect(within(anthropicRow).getByText("no key")).toBeTruthy());
  });

  it("saves the edited default provider through llmSetSettings", async () => {
    render(<AgentPane />);
    await screen.findByText(/key set/i);

    await userEvent.click(screen.getByRole("radio", { name: /use openai as the default provider/i }));
    await userEvent.click(screen.getByRole("button", { name: "Save settings" }));

    expect(llmSetSettings).toHaveBeenCalledWith(expect.objectContaining({ defaultProvider: "openAi" }));
  });

  it("clears the entered key from its field once saved, rather than leaving it there", async () => {
    render(<AgentPane />);
    await screen.findByText(/key set/i);

    const field = screen.getByLabelText<HTMLInputElement>("Anthropic API key");
    await userEvent.type(field, "sk-freshly-typed-secret");
    await userEvent.click(screen.getByRole("button", { name: "Save key" }));

    expect(llmSetKey).toHaveBeenCalledWith("anthropic", "sk-freshly-typed-secret");
    // The field is cleared afterwards, not left holding what was just sent.
    await waitFor(() => expect(field.value).toBe(""));
    expect(document.body.textContent).not.toMatch(/freshly-typed-secret/);
  });
});
