import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { AssistantConversation } from "./AssistantConversation";
import * as chat from "../lib/chat";
import * as chatHistory from "../lib/chatHistory";

vi.mock("../lib/chat");
vi.mock("../lib/chatHistory");
vi.mock("../lib/mcpSecurity", () => ({
  respondToConfirm: vi.fn(),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: () => Promise.resolve(() => {}),
}));

// This repo doesn't pull in @testing-library/jest-dom, so assert directly on
// DOM presence (`getByText`/`queryByText` throws-or-null) instead of
// `toBeInTheDocument`.

let nextSession = 0;

beforeEach(() => {
  // Every completed turn now auto-saves, so a mock's call history from an
  // earlier test in this file (`saveSession`, `startChat`, ...) would
  // otherwise bleed into the next test's assertions — clear it first.
  vi.clearAllMocks();
  nextSession = 0;
  vi.mocked(chat.listAgents).mockResolvedValue([
    {
      kind: "claude",
      label: "Claude Code",
      available: true,
      path: "/usr/bin/claude",
      version: null,
      installUrl: "",
      gated: false,
    },
  ]);
  // Each call mints a fresh channel session id — mirrors the real
  // `startChat()`, which is also the disk `Session.id` (see chatHistory.ts).
  vi.mocked(chat.startChat).mockImplementation(async () => `s${++nextSession}`);
  vi.mocked(chatHistory.listSessions).mockResolvedValue([]);
  vi.mocked(chatHistory.loadSession).mockRejectedValue(new Error("not stubbed"));
  vi.mocked(chatHistory.saveSession).mockResolvedValue(undefined);
  vi.mocked(chatHistory.deleteSession).mockResolvedValue(undefined);
});

describe("AssistantConversation", () => {
  it("renders with no context: no chip, agent picker and input present", async () => {
    render(<AssistantConversation />);
    await screen.findByRole("combobox", { name: /agent/i });
    expect(screen.getByPlaceholderText(/ask/i)).toBeTruthy();
    // No context was passed, so no removable context chip.
    expect(screen.queryByLabelText("Remove context")).toBeFalsy();
  });

  it("streams a reply with no context attached", async () => {
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "textDelta", text: "Hello from the global assistant." });
      onEvent({ type: "turnDone" });
    });
    render(<AssistantConversation />);
    fireEvent.change(await screen.findByPlaceholderText(/ask/i), { target: { value: "what's up?" } });
    fireEvent.click(screen.getByRole("button", { name: /send/i }));
    await waitFor(() => expect(screen.getByText(/Hello from the global assistant\./)).toBeTruthy());
    // The prompt sent to the agent carries no context preface since none was attached.
    expect(vi.mocked(chat.sendChat).mock.calls[0][1]).toBe("what's up?");
  });

  it("shows a context chip when context is provided, scoped like the drawer", async () => {
    render(<AssistantConversation context={{ context: "kind", namespace: "payments" }} />);
    expect(await screen.findByText("kind / payments")).toBeTruthy();
  });

  it("never auto-selects a gated agent: Claude wins over an installed-but-gated Codex", async () => {
    vi.mocked(chat.listAgents).mockResolvedValue([
      {
        kind: "codex",
        label: "Codex",
        available: true,
        path: "/usr/bin/codex",
        version: null,
        installUrl: "",
        gated: true,
      },
      {
        kind: "claude",
        label: "Claude Code",
        available: true,
        path: "/usr/bin/claude",
        version: null,
        installUrl: "",
        gated: false,
      },
    ]);
    render(<AssistantConversation />);
    const select = await screen.findByRole("combobox", { name: /agent/i });
    await waitFor(() => expect((select as HTMLSelectElement).value).toBe("claude"));
    fireEvent.change(screen.getByPlaceholderText(/ask/i), { target: { value: "hi" } });
    expect((screen.getByRole("button", { name: /send/i }) as HTMLButtonElement).disabled).toBe(false);
  });

  it("disables Send and shows a coming-soon note when a gated agent is selected", async () => {
    vi.mocked(chat.listAgents).mockResolvedValue([
      {
        kind: "codex",
        label: "Codex",
        available: true,
        path: "/usr/bin/codex",
        version: null,
        installUrl: "",
        gated: true,
      },
      {
        kind: "claude",
        label: "Claude Code",
        available: true,
        path: "/usr/bin/claude",
        version: null,
        installUrl: "",
        gated: false,
      },
    ]);
    render(<AssistantConversation />);
    const select = await screen.findByRole("combobox", { name: /agent/i });
    fireEvent.change(select, { target: { value: "codex" } });
    expect((screen.getByRole("button", { name: /send/i }) as HTMLButtonElement).disabled).toBe(true);
    expect(
      await screen.findByText(/Codex\/Cursor support is coming — use Claude for now\./)
    ).toBeTruthy();
  });

  it("a gated-only agent list keeps Send disabled and shows the coming note, not an install link", async () => {
    vi.mocked(chat.listAgents).mockResolvedValue([
      {
        kind: "codex",
        label: "Codex",
        available: true,
        path: "/usr/bin/codex",
        version: null,
        installUrl: "https://developers.openai.com/codex/cli/",
        gated: true,
      },
    ]);
    render(<AssistantConversation />);
    await screen.findByRole("combobox", { name: /agent/i });
    expect((screen.getByRole("button", { name: /send/i }) as HTMLButtonElement).disabled).toBe(true);
    expect(
      await screen.findByText(/Codex\/Cursor support is coming — use Claude for now\./)
    ).toBeTruthy();
    expect(screen.queryByText("https://developers.openai.com/codex/cli/")).toBeFalsy();
  });
});

describe("AssistantConversation session persistence", () => {
  it("auto-saves exactly once per completed turn, with the expected shape", async () => {
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "textDelta", text: "Scaling now." });
      onEvent({ type: "turnDone" });
    });
    render(<AssistantConversation />);
    fireEvent.change(await screen.findByPlaceholderText(/ask/i), {
      target: { value: "scale the payments deployment to 3 replicas" },
    });
    fireEvent.click(screen.getByRole("button", { name: /send/i }));

    await waitFor(() => expect(chatHistory.saveSession).toHaveBeenCalledTimes(1));
    const saved = vi.mocked(chatHistory.saveSession).mock.calls[0][0];
    expect(saved.id).toBe("s1");
    expect(saved.title).toBe("scale the payments deployment to 3 replicas");
    expect(saved.contexts).toEqual([]);
    expect(saved.skills).toEqual([]);
    expect(saved.cliSessionId).toBeNull();
    expect(saved.messages).toHaveLength(2);
    expect(saved.messages[0]).toEqual({ id: 0, role: "user", text: "scale the payments deployment to 3 replicas" });
    expect(saved.messages[1]).toEqual({ id: 1, role: "assistant", text: "Scaling now." });
  });

  it("auto-save records the attached context under `contexts`", async () => {
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "textDelta", text: "ok" });
      onEvent({ type: "turnDone" });
    });
    render(<AssistantConversation context={{ context: "prod-cluster", namespace: "payments" }} />);
    fireEvent.change(await screen.findByPlaceholderText(/ask/i), { target: { value: "hi" } });
    fireEvent.click(screen.getByRole("button", { name: /send/i }));

    await waitFor(() => expect(chatHistory.saveSession).toHaveBeenCalledTimes(1));
    expect(vi.mocked(chatHistory.saveSession).mock.calls[0][0].contexts).toEqual(["prod-cluster"]);
  });

  it("auto-saves on a stream error turn too, once, with the error appended", async () => {
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "error", message: "boom" });
    });
    render(<AssistantConversation />);
    fireEvent.change(await screen.findByPlaceholderText(/ask/i), { target: { value: "hi" } });
    fireEvent.click(screen.getByRole("button", { name: /send/i }));

    await waitFor(() => expect(chatHistory.saveSession).toHaveBeenCalledTimes(1));
    const saved = vi.mocked(chatHistory.saveSession).mock.calls[0][0];
    expect(saved.messages).toHaveLength(3);
    expect(saved.messages[2]).toEqual({ id: 2, role: "error", text: "boom" });
  });

  it("New chat clears the transcript, and the next send mints a fresh session id", async () => {
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "textDelta", text: "first reply" });
      onEvent({ type: "turnDone" });
    });
    render(<AssistantConversation />);
    fireEvent.change(await screen.findByPlaceholderText(/ask/i), { target: { value: "first question" } });
    fireEvent.click(screen.getByRole("button", { name: /send/i }));
    await waitFor(() => expect(screen.getByText("first reply")).toBeTruthy());
    await waitFor(() => expect(chatHistory.saveSession).toHaveBeenCalledTimes(1));

    fireEvent.click(screen.getByRole("button", { name: /new chat/i }));
    // The transcript is empty again — back to the placeholder — even though
    // the just-saved session still shows up as a chip (New chat clears the
    // *editing* state; it doesn't delete anything from disk).
    expect(await screen.findByText(/ask about this cluster to get started/i)).toBeTruthy();

    fireEvent.change(screen.getByPlaceholderText(/ask/i), { target: { value: "second question" } });
    fireEvent.click(screen.getByRole("button", { name: /send/i }));
    await waitFor(() => expect(chatHistory.saveSession).toHaveBeenCalledTimes(2));

    const ids = vi.mocked(chatHistory.saveSession).mock.calls.map((c) => c[0].id);
    expect(ids).toEqual(["s1", "s2"]);
  });

  it("selecting a session loads it and replays its messages read-only into state", async () => {
    vi.mocked(chatHistory.listSessions).mockResolvedValue([
      { id: "old-1", title: "Old chat", createdAt: 1, updatedAt: 2 },
    ]);
    vi.mocked(chatHistory.loadSession).mockResolvedValue({
      id: "old-1",
      title: "Old chat",
      createdAt: 1,
      updatedAt: 2,
      contexts: [],
      skills: [],
      cliSessionId: null,
      messages: [
        { id: 0, role: "user", text: "what pods are crashing?" },
        { id: 1, role: "assistant", text: "pod-a is crashlooping." },
      ],
    });
    render(<AssistantConversation />);
    fireEvent.click(await screen.findByText("Old chat"));

    expect(await screen.findByText("what pods are crashing?")).toBeTruthy();
    expect(await screen.findByText(/pod-a is crashlooping\./)).toBeTruthy();
    expect(chatHistory.loadSession).toHaveBeenCalledWith("old-1");
  });

  it("delete removes a session from disk and from the visible list", async () => {
    vi.mocked(chatHistory.listSessions).mockResolvedValue([
      { id: "old-1", title: "Old chat", createdAt: 1, updatedAt: 2 },
    ]);
    render(<AssistantConversation />);
    await screen.findByText("Old chat");

    fireEvent.click(screen.getByLabelText("Delete Old chat"));

    await waitFor(() => expect(chatHistory.deleteSession).toHaveBeenCalledWith("old-1"));
    expect(screen.queryByText("Old chat")).toBeFalsy();
  });

  it("renders the session list in the newest-first order listSessions returns", async () => {
    vi.mocked(chatHistory.listSessions).mockResolvedValue([
      { id: "newest", title: "Newest chat", createdAt: 1, updatedAt: 300 },
      { id: "oldest", title: "Oldest chat", createdAt: 1, updatedAt: 10 },
    ]);
    render(<AssistantConversation />);
    await screen.findByText("Newest chat");

    const newest = screen.getByText("Newest chat");
    const oldest = screen.getByText("Oldest chat");
    expect(newest.compareDocumentPosition(oldest) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });
});
