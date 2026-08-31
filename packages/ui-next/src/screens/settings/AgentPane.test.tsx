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
  openExternal: vi.fn(),
}));
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  ...core,
}));

import { AgentPane } from "./AgentPane";

const {
  llmGetSettings,
  llmSetSettings,
  llmSetKey,
  llmClearKey,
  llmKeyStatus,
  llmListModels,
  listAgents,
  openExternal,
} = core;

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
    openExternal.mockResolvedValue(undefined);
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

  // M9: `settingsRead.kind === "error"` and `keyStatusRead.kind === "error"`
  // had no test anywhere — making the `llmKeyStatus` catch a no-op (so a
  // rejection is silently swallowed rather than recorded) passed every test
  // in this file.
  it("says provider settings could not be loaded, rather than drawing the provider rows, when llmGetSettings rejects", async () => {
    llmGetSettings.mockRejectedValue(new Error("no such command: llm_get_settings"));
    render(<AgentPane />);
    expect(await screen.findByText(/provider settings could not be loaded/i)).toBeTruthy();
    expect(await screen.findByText(/llm_get_settings/)).toBeTruthy();
    expect(screen.queryByTestId("provider-row-anthropic")).toBeNull();
  });

  it("says which providers have a key could not be checked, rather than drawing the provider rows, when llmKeyStatus rejects", async () => {
    llmKeyStatus.mockRejectedValue(new Error("no such command: llm_key_status"));
    render(<AgentPane />);
    expect(await screen.findByText(/which providers have a key could not be checked/i)).toBeTruthy();
    expect(await screen.findByText(/llm_key_status/)).toBeTruthy();
    expect(screen.queryByTestId("provider-row-anthropic")).toBeNull();
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

  /**
   * P2 (#392 review round 3): the install link was an `<a target="_blank">`,
   * which is a silent no-op inside the Tauri WebView (#348) — and it is the
   * ONLY control an unavailable CLI offers, so on the desktop the one
   * actionable thing in that row did nothing. `screens/Forwards.tsx` had
   * already learned this and says so in its own comment.
   */
  it("opens an install page through the desktop opener, not a dead target=_blank link", async () => {
    listAgents.mockResolvedValue([
      { kind: "codex", label: "Codex", available: false, path: null, version: null, installUrl: "https://x", gated: false },
    ]);
    render(<AgentPane />);
    const control = await screen.findByRole("button", { name: /install Codex/i });
    // A button, not a link: an anchor here cannot open anything on the
    // primary surface.
    expect(control.tagName).toBe("BUTTON");
    await userEvent.click(control);
    expect(openExternal).toHaveBeenCalledWith("https://x");
  });

  it("says so when the install page could not be opened, rather than looking like it worked", async () => {
    listAgents.mockResolvedValue([
      { kind: "codex", label: "Codex", available: false, path: null, version: null, installUrl: "https://x", gated: false },
    ]);
    openExternal.mockRejectedValue(new Error("no opener available"));
    render(<AgentPane />);
    await userEvent.click(await screen.findByRole("button", { name: /install Codex/i }));
    expect(await screen.findByText(/could not open codex's install page/i)).toBeTruthy();
    expect(screen.getByText(/no opener available/)).toBeTruthy();
  });

  /**
   * P2 (#392 review round 4). The controls stay editable while
   * `llmSetSettings` is in flight, and its completion set `saved` to true
   * unconditionally — overwriting the `setSaved(false)` that an intervening
   * edit had performed. The reader was told their newer draft was persisted
   * when what landed was the older snapshot, which invites navigating away
   * and losing it.
   */
  it("does not report a save as done when the draft moved on while it was in flight", async () => {
    let finishSave!: () => void;
    llmSetSettings.mockImplementation(() => new Promise<void>((resolve) => { finishSave = () => resolve(); }));
    llmKeyStatus.mockResolvedValue(["anthropic"]);
    render(<AgentPane />);

    await userEvent.click(await screen.findByRole("button", { name: /save settings/i }));
    // The reader picks a different default provider while the request is out.
    const radios = screen.getAllByRole("radio");
    const other = radios.find((r) => !(r as HTMLInputElement).checked)!;
    await userEvent.click(other);

    await act(async () => {
      finishSave();
    });

    // The older snapshot landed; what is on screen is not what was saved.
    expect(screen.queryByText(/^saved\.$/i)).toBeNull();
  });

  it("reports a save as done when the draft is still the one that went to disk", async () => {
    llmKeyStatus.mockResolvedValue(["anthropic"]);
    render(<AgentPane />);
    await userEvent.click(await screen.findByRole("button", { name: /save settings/i }));
    // Not a latch on the other side either: an untouched draft still confirms.
    expect(await screen.findByText(/^saved\.$/i)).toBeTruthy();
  });

  /**
   * P2 (#392 review round 5). `busy` was one overwritable string, so a second
   * key operation replaced the first's token and whichever request finished
   * first cleared it while the other was still running — both providers'
   * buttons came back live mid-flight and a double submit was one click away.
   */
  it("holds every provider's key actions while one key operation is in flight", async () => {
    let finishClear!: () => void;
    llmClearKey.mockImplementation(() => new Promise<void>((resolve) => { finishClear = () => resolve(); }));
    // Two providers with keys, so both have a Remove to be wrongly offered.
    // `openAi`, not `openai` — `PROVIDERS` (core/llm.ts) is camelCase, and a
    // fixture with the wrong kind gives that row no key, so it renders no
    // Remove button and the test cannot see the defect it is about.
    llmKeyStatus.mockResolvedValue(["anthropic", "openAi"]);
    render(<AgentPane />);

    // The default provider's row is open already; start removing its key.
    await userEvent.click(await screen.findByRole("button", { name: /remove key/i }));

    // Switch to the other provider — `expanded` holds one at a time, so this
    // is the reviewer's exact scenario.
    // "OpenAI" and "OpenAI-compatible" both match a loose pattern; this is
    // the plain one, whose row says it holds a key.
    const openai = screen
      .getAllByRole("button")
      .find((b) => /^OpenAI(?!-)/.test(b.textContent ?? ""))!;
    await userEvent.click(openai);

    // Its key actions must not be live: both write the same store, and the
    // first operation has not finished.
    expect((screen.getByRole("button", { name: /remove key/i }) as HTMLButtonElement).disabled).toBe(true);

    await act(async () => {
      finishClear();
    });
  });
});
