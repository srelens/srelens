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

  it("saves exactly once when a turn streams an error followed by turnDone (the real backend's shape)", async () => {
    // The backend always emits a terminal `turnDone` after any `error` on a
    // live channel (crash-recovery in `finish_turn`, and the bad-image-
    // attachment path both do this) — saving on `error` too would double-save.
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "error", message: "boom" });
      onEvent({ type: "turnDone" });
    });
    render(<AssistantConversation />);
    fireEvent.change(await screen.findByPlaceholderText(/ask/i), { target: { value: "hi" } });
    fireEvent.click(screen.getByRole("button", { name: /send/i }));

    await waitFor(() => expect(screen.getByText("boom")).toBeTruthy());
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

  it("preserves createdAt across a second save of the same session, while updatedAt advances", async () => {
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "textDelta", text: "reply" });
      onEvent({ type: "turnDone" });
    });
    render(<AssistantConversation />);

    fireEvent.change(await screen.findByPlaceholderText(/ask/i), { target: { value: "first turn" } });
    fireEvent.click(screen.getByRole("button", { name: /send/i }));
    await waitFor(() => expect(chatHistory.saveSession).toHaveBeenCalledTimes(1));
    const firstSaved = vi.mocked(chatHistory.saveSession).mock.calls[0][0];

    fireEvent.change(screen.getByPlaceholderText(/ask/i), { target: { value: "second turn" } });
    fireEvent.click(screen.getByRole("button", { name: /send/i }));
    await waitFor(() => expect(chatHistory.saveSession).toHaveBeenCalledTimes(2));
    const secondSaved = vi.mocked(chatHistory.saveSession).mock.calls[1][0];

    expect(secondSaved.id).toBe(firstSaved.id);
    expect(secondSaved.createdAt).toBe(firstSaved.createdAt);
    expect(secondSaved.updatedAt).toBeGreaterThanOrEqual(firstSaved.updatedAt);
  });

  it("sends further turns of a reopened session back to the same disk id", async () => {
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
      messages: [{ id: 0, role: "user", text: "what pods are crashing?" }],
    });
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "textDelta", text: "still looking" });
      onEvent({ type: "turnDone" });
    });
    render(<AssistantConversation />);
    fireEvent.click(await screen.findByText("Old chat"));
    await screen.findByText("what pods are crashing?");

    fireEvent.change(screen.getByPlaceholderText(/ask/i), { target: { value: "any update?" } });
    fireEvent.click(screen.getByRole("button", { name: /send/i }));

    await waitFor(() => expect(chatHistory.saveSession).toHaveBeenCalledTimes(1));
    expect(vi.mocked(chatHistory.saveSession).mock.calls[0][0].id).toBe("old-1");
    // Reopening never re-mints a channel session — `startChat` (which only
    // fires when `sessionRef.current` is empty) must not have been called.
    expect(chat.startChat).not.toHaveBeenCalled();
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

describe("AssistantConversation image attachments", () => {
  // 3 zero bytes -> base64 "AAAA" with no padding, so the data URI jsdom's
  // real (in-memory, no filesystem) FileReader produces is a stable literal:
  // `data:image/png;base64,AAAA`.
  function pngFile(name = "a.png"): File {
    return new File([Uint8Array.from([0, 0, 0])], name, { type: "image/png" });
  }

  it("attaching an image via the attach control shows a thumbnail chip", async () => {
    render(<AssistantConversation />);
    const attach = await screen.findByLabelText(/attach image/i);
    fireEvent.change(attach, { target: { files: [pngFile()] } });

    const thumb = (await screen.findByAltText(/pending image 1/i)) as HTMLImageElement;
    expect(thumb.src).toBe("data:image/png;base64,AAAA");
  });

  it("pasting an image into the composer shows a thumbnail chip", async () => {
    render(<AssistantConversation />);
    const textbox = await screen.findByPlaceholderText(/ask/i);
    fireEvent.paste(textbox, { clipboardData: { files: [pngFile("clip.png")] } });

    expect(await screen.findByAltText(/pending image 1/i)).toBeTruthy();
  });

  it("removing a pending image chip clears it, and it isn't sent", async () => {
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "turnDone" });
    });
    render(<AssistantConversation />);
    fireEvent.change(await screen.findByLabelText(/attach image/i), { target: { files: [pngFile()] } });
    await screen.findByAltText(/pending image 1/i);

    fireEvent.click(screen.getByLabelText(/remove image 1/i));
    expect(screen.queryByAltText(/pending image 1/i)).toBeFalsy();

    fireEvent.change(screen.getByPlaceholderText(/ask/i), { target: { value: "hi" } });
    fireEvent.click(screen.getByRole("button", { name: /^send$/i }));

    await waitFor(() => expect(chat.sendChat).toHaveBeenCalled());
    expect(vi.mocked(chat.sendChat).mock.calls[0][4]).toEqual([]);
    expect(screen.queryByAltText(/attached image/i)).toBeFalsy();
  });

  it("sending strips the data URI prefix, passing raw base64 as sendChat's 5th arg", async () => {
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "turnDone" });
    });
    render(<AssistantConversation />);
    fireEvent.change(await screen.findByLabelText(/attach image/i), { target: { files: [pngFile()] } });
    await screen.findByAltText(/pending image 1/i);

    fireEvent.change(screen.getByPlaceholderText(/ask/i), { target: { value: "check this" } });
    fireEvent.click(screen.getByRole("button", { name: /^send$/i }));

    await waitFor(() => expect(chat.sendChat).toHaveBeenCalled());
    // Hand-written literal: data:image/png;base64,AAAA -> "AAAA".
    expect(vi.mocked(chat.sendChat).mock.calls[0][4]).toEqual(["AAAA"]);
  });

  it("renders the sent image inline in the user bubble, and clears the pending chip", async () => {
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "turnDone" });
    });
    render(<AssistantConversation />);
    fireEvent.change(await screen.findByLabelText(/attach image/i), { target: { files: [pngFile()] } });
    await screen.findByAltText(/pending image 1/i);

    fireEvent.change(screen.getByPlaceholderText(/ask/i), { target: { value: "look at this" } });
    fireEvent.click(screen.getByRole("button", { name: /^send$/i }));

    const img = (await screen.findByAltText(/attached image 1/i)) as HTMLImageElement;
    expect(img.src).toBe("data:image/png;base64,AAAA");
    expect(screen.queryByAltText(/pending image/i)).toBeFalsy();
  });

  it("auto-save persists the attached images on the stored user message", async () => {
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "turnDone" });
    });
    render(<AssistantConversation />);
    fireEvent.change(await screen.findByLabelText(/attach image/i), { target: { files: [pngFile()] } });
    await screen.findByAltText(/pending image 1/i);

    fireEvent.change(screen.getByPlaceholderText(/ask/i), { target: { value: "check this" } });
    fireEvent.click(screen.getByRole("button", { name: /^send$/i }));

    await waitFor(() => expect(chatHistory.saveSession).toHaveBeenCalledTimes(1));
    const saved = vi.mocked(chatHistory.saveSession).mock.calls[0][0];
    expect(saved.messages[0]).toEqual({
      id: 0,
      role: "user",
      text: "check this",
      images: ["data:image/png;base64,AAAA"],
    });
  });

  it("a reloaded session with images renders them inline", async () => {
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
      messages: [{ id: 0, role: "user", text: "check this", images: ["data:image/png;base64,AAAA"] }],
    });
    render(<AssistantConversation />);
    fireEvent.click(await screen.findByText("Old chat"));

    const img = (await screen.findByAltText(/attached image 1/i)) as HTMLImageElement;
    expect(img.src).toBe("data:image/png;base64,AAAA");
  });
});

describe("AssistantConversation multi-context (global tab)", () => {
  it("drawer path (no availableContexts): no Contexts control, and Send isn't gated on any context", async () => {
    render(<AssistantConversation context={{ context: "prod-cluster" }} />);
    await screen.findByRole("combobox", { name: /agent/i });
    expect(screen.queryByRole("button", { name: /contexts \(/i })).toBeFalsy();
    fireEvent.change(screen.getByPlaceholderText(/ask/i), { target: { value: "hi" } });
    expect((screen.getByRole("button", { name: /^send$/i }) as HTMLButtonElement).disabled).toBe(false);
  });

  it("global tab shows a Contexts control and disables Send until at least one context is selected", async () => {
    render(<AssistantConversation availableContexts={["a", "b", "c"]} />);
    await screen.findByRole("combobox", { name: /agent/i });
    expect(screen.getByRole("button", { name: /contexts \(0\)/i })).toBeTruthy();

    fireEvent.change(screen.getByPlaceholderText(/ask/i), { target: { value: "hi" } });
    expect((screen.getByRole("button", { name: /^send$/i }) as HTMLButtonElement).disabled).toBe(true);

    fireEvent.click(screen.getByRole("button", { name: /contexts \(0\)/i }));
    fireEvent.click(await screen.findByText("a"));
    expect((screen.getByRole("button", { name: /^send$/i }) as HTMLButtonElement).disabled).toBe(false);
  });

  it("selecting two contexts shows two removable chips and prefaces the outgoing prompt enumerating both", async () => {
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "turnDone" });
    });
    render(<AssistantConversation availableContexts={["a", "b", "c"]} />);
    fireEvent.click(await screen.findByRole("button", { name: /contexts \(0\)/i }));
    fireEvent.click(await screen.findByText("a"));
    fireEvent.click(screen.getByText("b"));

    expect(screen.getByLabelText("Remove a")).toBeTruthy();
    expect(screen.getByLabelText("Remove b")).toBeTruthy();

    fireEvent.change(screen.getByPlaceholderText(/ask/i), { target: { value: "what's up?" } });
    fireEvent.click(screen.getByRole("button", { name: /^send$/i }));

    await waitFor(() => expect(chat.sendChat).toHaveBeenCalled());
    // Hand-written literal, not built from the component's own preface function.
    expect(vi.mocked(chat.sendChat).mock.calls[0][1]).toBe(
      "You may work across these clusters: `a`, `b`. Pass the appropriate context to each tool call.\n\nwhat's up?",
    );
  });

  it("removing a chip updates both the chip list and the preface (down to the single-context wording)", async () => {
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "turnDone" });
    });
    render(<AssistantConversation availableContexts={["a", "b", "c"]} />);
    fireEvent.click(await screen.findByRole("button", { name: /contexts \(0\)/i }));
    fireEvent.click(await screen.findByText("a"));
    fireEvent.click(screen.getByText("b"));

    fireEvent.click(screen.getByLabelText("Remove a"));
    expect(screen.queryByLabelText("Remove a")).toBeFalsy();
    expect(screen.getByLabelText("Remove b")).toBeTruthy();

    fireEvent.change(screen.getByPlaceholderText(/ask/i), { target: { value: "status?" } });
    fireEvent.click(screen.getByRole("button", { name: /^send$/i }));

    await waitFor(() => expect(chat.sendChat).toHaveBeenCalled());
    expect(vi.mocked(chat.sendChat).mock.calls[0][1]).toBe(
      "Work in the cluster `b` (the default context). Pass its context to each tool call.\n\nstatus?",
    );
  });

  it("persists selectedContexts under the session's contexts field in multi-context mode", async () => {
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "turnDone" });
    });
    render(<AssistantConversation availableContexts={["a", "b"]} />);
    fireEvent.click(await screen.findByRole("button", { name: /contexts \(0\)/i }));
    fireEvent.click(await screen.findByText("a"));
    fireEvent.change(screen.getByPlaceholderText(/ask/i), { target: { value: "hi" } });
    fireEvent.click(screen.getByRole("button", { name: /^send$/i }));

    await waitFor(() => expect(chatHistory.saveSession).toHaveBeenCalledTimes(1));
    expect(vi.mocked(chatHistory.saveSession).mock.calls[0][0].contexts).toEqual(["a"]);
  });

  it("a reloaded session restores its chips from the session's `contexts`", async () => {
    vi.mocked(chatHistory.listSessions).mockResolvedValue([
      { id: "old-1", title: "Old chat", createdAt: 1, updatedAt: 2 },
    ]);
    vi.mocked(chatHistory.loadSession).mockResolvedValue({
      id: "old-1",
      title: "Old chat",
      createdAt: 1,
      updatedAt: 2,
      contexts: ["a"],
      skills: [],
      cliSessionId: null,
      messages: [{ id: 0, role: "user", text: "hi" }],
    });
    render(<AssistantConversation availableContexts={["a", "b", "c"]} />);
    fireEvent.click(await screen.findByText("Old chat"));

    await screen.findByText("hi");
    expect(screen.getByLabelText("Remove a")).toBeTruthy();
    expect(screen.getByRole("button", { name: /contexts \(1\)/i })).toBeTruthy();
  });
});
