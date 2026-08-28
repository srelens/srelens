import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor, within, act } from "@testing-library/react";
import { createRef } from "react";
import {
  AssistantConversation,
  stripDataUri,
  type AssistantConversationHandle,
} from "./AssistantConversation";
import * as chat from "@srelens/core";
import * as chatHistory from "@srelens/core";
import type { StoredMessage } from "@srelens/core";
import * as prompts from "@srelens/core";
import * as skills from "@srelens/core";

vi.mock("@srelens/core/lib/chat");
vi.mock("@srelens/core/lib/chatHistory");
vi.mock("@srelens/core/lib/prompts");
vi.mock("@srelens/core/lib/skills");
vi.mock("@srelens/core/lib/mcpSecurity", () => ({
  respondToConfirm: vi.fn(),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: () => Promise.resolve(() => {}),
}));

// This repo doesn't pull in @testing-library/jest-dom, so assert directly on
// DOM presence (`getByText`/`queryByText` throws-or-null) instead of
// `toBeInTheDocument`.

let nextSession = 0;

/** Opens the compact `HistoryPopover` (Task 19) — New Chat and the saved
 * sessions only render once it's open, so any test that used to find them
 * inline now opens it first. */
async function openHistory() {
  fireEvent.click(await screen.findByRole("button", { name: /^history$/i }));
}

beforeEach(() => {
  // Every completed turn now auto-saves, so a mock's call history from an
  // earlier test in this file (`saveSession`, `startChat`, ...) would
  // otherwise bleed into the next test's assertions — clear it first.
  vi.clearAllMocks();
  // The composer persists the last-used agent to localStorage; clear it so one
  // test's selection doesn't become the next test's default.
  localStorage.clear();
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
  vi.mocked(prompts.listPrompts).mockResolvedValue([]);
  vi.mocked(skills.listSkills).mockResolvedValue([]);
  vi.mocked(skills.loadSkill).mockRejectedValue(new Error("not stubbed"));
});

describe("stripDataUri", () => {
  it("leaves a bare base64 string (no `data:` prefix, no comma) unchanged", () => {
    expect(stripDataUri("AAAA")).toBe("AAAA");
  });
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
      return null;
    });
    render(<AssistantConversation />);
    fireEvent.change(await screen.findByPlaceholderText(/ask/i), { target: { value: "what's up?" } });
    fireEvent.click(screen.getByRole("button", { name: /send/i }));
    await waitFor(() => expect(screen.getByText(/Hello from the global assistant\./)).toBeTruthy());
    // The prompt sent to the agent carries no context preface since none was attached.
    expect(vi.mocked(chat.sendChat).mock.calls[0][1]).toBe("what's up?");
    // Only Claude was in the mocked agent list, so it's the one auto-selected
    // and threaded through as sendChat's agentKind argument.
    expect(vi.mocked(chat.sendChat).mock.calls[0][5]).toBe("claude");
  });

  it("threads the selected agent's kind through to sendChat, not just its path", async () => {
    vi.mocked(chat.listAgents).mockResolvedValue([
      {
        kind: "codex",
        label: "Codex",
        available: true,
        path: "/usr/bin/codex",
        version: null,
        installUrl: "",
        gated: false,
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
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "turnDone" });
      return null;
    });
    render(<AssistantConversation />);
    const trigger = await screen.findByRole("combobox", { name: /agent/i });
    fireEvent.click(trigger);
    fireEvent.click(await screen.findByRole("option", { name: /codex/i }));
    fireEvent.change(screen.getByPlaceholderText(/ask/i), { target: { value: "hi" } });
    fireEvent.click(screen.getByRole("button", { name: /send/i }));
    await waitFor(() => expect(chat.sendChat).toHaveBeenCalled());
    expect(vi.mocked(chat.sendChat).mock.calls[0][2]).toBe("/usr/bin/codex");
    expect(vi.mocked(chat.sendChat).mock.calls[0][5]).toBe("codex");
  });

  it("Cursor is selectable like Claude and Codex", async () => {
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
      {
        kind: "cursor",
        label: "Cursor",
        available: true,
        path: "/usr/bin/cursor-agent",
        version: null,
        installUrl: "",
        gated: false,
      },
    ]);
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "turnDone" });
      return null;
    });
    render(<AssistantConversation />);
    const trigger = await screen.findByRole("combobox", { name: /agent/i });
    fireEvent.click(trigger);
    const cursorOption = await screen.findByRole("option", { name: /cursor/i });
    expect((cursorOption as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(cursorOption);
    fireEvent.change(screen.getByPlaceholderText(/ask/i), { target: { value: "hi" } });
    fireEvent.click(screen.getByRole("button", { name: /send/i }));
    await waitFor(() => expect(chat.sendChat).toHaveBeenCalled());
    expect(vi.mocked(chat.sendChat).mock.calls[0][2]).toBe("/usr/bin/cursor-agent");
    expect(vi.mocked(chat.sendChat).mock.calls[0][5]).toBe("cursor");
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
    const trigger = await screen.findByRole("combobox", { name: /agent/i });
    // The trigger shows the selected agent's label — Claude, never the gated Codex.
    await waitFor(() => expect(trigger.textContent).toMatch(/claude/i));
    fireEvent.change(screen.getByPlaceholderText(/ask/i), { target: { value: "hi" } });
    expect((screen.getByRole("button", { name: /send/i }) as HTMLButtonElement).disabled).toBe(false);
  });

  it("renders a gated agent as a disabled option so it can't be picked", async () => {
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
    const trigger = await screen.findByRole("combobox", { name: /agent/i });
    fireEvent.click(trigger);
    const codexOption = await screen.findByRole("option", { name: /codex/i });
    expect((codexOption as HTMLButtonElement).disabled).toBe(true);
    // Claude stays the selection and Send works.
    fireEvent.change(screen.getByPlaceholderText(/ask/i), { target: { value: "hi" } });
    expect((screen.getByRole("button", { name: /send/i }) as HTMLButtonElement).disabled).toBe(false);
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
      await screen.findByText(/Codex support is coming — use Claude for now\./)
    ).toBeTruthy();
    expect(screen.queryByText("https://developers.openai.com/codex/cli/")).toBeFalsy();
  });
});

describe("AssistantConversation session persistence", () => {
  it("auto-saves exactly once per completed turn, with the expected shape", async () => {
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "textDelta", text: "Scaling now." });
      onEvent({ type: "turnDone" });
      return null;
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

  it("resumes the CLI's own session on follow-up turns and persists the id", async () => {
    // First turn: the backend captures the CLI's session id from the stream
    // and returns it from sendChat.
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "textDelta", text: "hi" });
      onEvent({ type: "turnDone" });
      return "cli-abc";
    });
    render(<AssistantConversation />);
    fireEvent.change(await screen.findByPlaceholderText(/ask/i), { target: { value: "first" } });
    fireEvent.click(screen.getByRole("button", { name: /send/i }));
    // A first turn resumes nothing (9th arg), and once the id lands a
    // follow-up save records it with the transcript.
    await waitFor(() => expect(chat.sendChat).toHaveBeenCalledTimes(1));
    expect(vi.mocked(chat.sendChat).mock.calls[0][7]).toBeNull();
    await waitFor(() => {
      const calls = vi.mocked(chatHistory.saveSession).mock.calls;
      expect(calls[calls.length - 1][0].cliSessionId).toBe("cli-abc");
    });

    // Second turn: the captured id comes back as `resume`.
    fireEvent.change(screen.getByPlaceholderText(/ask/i), { target: { value: "second" } });
    fireEvent.click(screen.getByRole("button", { name: /send/i }));
    await waitFor(() => expect(chat.sendChat).toHaveBeenCalledTimes(2));
    expect(vi.mocked(chat.sendChat).mock.calls[1][7]).toBe("cli-abc");
  });

  it("drops the stored CLI id after a turn on a different agent, so switching back starts fresh", async () => {
    // Turn 1 (Claude): captures an id. Turn 2 (Codex): returns null — but the
    // conversation grew by turns the Claude session never saw, so resuming
    // "cli-abc" later would answer from stale context. Turn 3 (back on
    // Claude) must therefore resume nothing.
    vi.mocked(chat.listAgents).mockResolvedValue([
      { kind: "claude", label: "Claude Code", available: true, path: "/usr/bin/claude", version: null, installUrl: "", gated: false },
      { kind: "codex", label: "Codex", available: true, path: "/usr/bin/codex", version: null, installUrl: "", gated: false },
    ]);
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "turnDone" });
      return "cli-abc";
    });
    render(<AssistantConversation />);
    fireEvent.change(await screen.findByPlaceholderText(/ask/i), { target: { value: "first" } });
    fireEvent.click(screen.getByRole("button", { name: /send/i }));
    await waitFor(() => expect(chat.sendChat).toHaveBeenCalledTimes(1));
    // Let the turn fully settle (Send replaces Stop) before touching the picker.
    await screen.findByRole("button", { name: /^send$/i });

    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "turnDone" });
      return null;
    });
    fireEvent.click(screen.getByRole("combobox", { name: /agent/i }));
    fireEvent.click(await screen.findByRole("option", { name: /codex/i }));
    fireEvent.change(screen.getByPlaceholderText(/ask/i), { target: { value: "second" } });
    fireEvent.click(screen.getByRole("button", { name: /^send$/i }));
    await waitFor(() => expect(chat.sendChat).toHaveBeenCalledTimes(2));
    await screen.findByRole("button", { name: /^send$/i });

    fireEvent.click(screen.getByRole("combobox", { name: /agent/i }));
    fireEvent.click(await screen.findByRole("option", { name: /claude/i }));
    fireEvent.change(screen.getByPlaceholderText(/ask/i), { target: { value: "third" } });
    fireEvent.click(screen.getByRole("button", { name: /^send$/i }));
    await waitFor(() => expect(chat.sendChat).toHaveBeenCalledTimes(3));
    expect(vi.mocked(chat.sendChat).mock.calls[2][7]).toBeNull();
  });

  it("clears the stored id when a resume crashes (null return), so the next turn starts fresh", async () => {
    // Turn 1 captures an id; turn 2 resumes it but the backend reports a
    // crashed resume by returning null (the CLI lost the session). Turn 3
    // must NOT retry the dead id — that would fail identically forever.
    vi.mocked(chat.sendChat)
      .mockImplementationOnce(async (_s, _p, _a, onEvent) => {
        onEvent({ type: "turnDone" });
        return "cli-abc";
      })
      .mockImplementation(async (_s, _p, _a, onEvent) => {
        onEvent({ type: "error", message: "No conversation found" });
        onEvent({ type: "turnDone" });
        return null;
      });
    render(<AssistantConversation />);
    fireEvent.change(await screen.findByPlaceholderText(/ask/i), { target: { value: "first" } });
    fireEvent.click(screen.getByRole("button", { name: /send/i }));
    await waitFor(() => expect(chat.sendChat).toHaveBeenCalledTimes(1));

    fireEvent.change(screen.getByPlaceholderText(/ask/i), { target: { value: "second" } });
    fireEvent.click(screen.getByRole("button", { name: /^send$/i }));
    await waitFor(() => expect(chat.sendChat).toHaveBeenCalledTimes(2));
    expect(vi.mocked(chat.sendChat).mock.calls[1][7]).toBe("cli-abc");

    fireEvent.change(screen.getByPlaceholderText(/ask/i), { target: { value: "third" } });
    fireEvent.click(screen.getByRole("button", { name: /^send$/i }));
    await waitFor(() => expect(chat.sendChat).toHaveBeenCalledTimes(3));
    expect(vi.mocked(chat.sendChat).mock.calls[2][7]).toBeNull();
  });

  it("codex thoughts render without a duration label; delta-streaming agents keep it", async () => {
    // Codex reasoning arrives as an already-completed summary item, so
    // wall-clock timing across its events would be fiction (a long burst
    // would read "· 1s"). Agents that stream real deltas keep the label.
    vi.mocked(chat.listAgents).mockResolvedValue([
      { kind: "claude", label: "Claude Code", available: true, path: "/usr/bin/claude", version: null, installUrl: "", gated: false },
      { kind: "codex", label: "Codex", available: true, path: "/usr/bin/codex", version: null, installUrl: "", gated: false },
    ]);
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "thinking", text: "**Weighing options**\n" });
      onEvent({ type: "textDelta", text: "answer" });
      onEvent({ type: "turnDone" });
      return null;
    });
    render(<AssistantConversation />);

    // Claude-kind turn (default pick): the timer runs → "· 1s" appears.
    fireEvent.change(await screen.findByPlaceholderText(/ask/i), { target: { value: "first" } });
    fireEvent.click(screen.getByRole("button", { name: /send/i }));
    await screen.findByText("Thoughts");
    expect(screen.getByText(/·\s*1s/)).toBeTruthy();
    await screen.findByRole("button", { name: /^send$/i });

    // Codex turn: same events, no duration label on its Thoughts row.
    fireEvent.click(screen.getByRole("combobox", { name: /agent/i }));
    fireEvent.click(await screen.findByRole("option", { name: /codex/i }));
    fireEvent.change(screen.getByPlaceholderText(/ask/i), { target: { value: "second" } });
    fireEvent.click(screen.getByRole("button", { name: /^send$/i }));
    await waitFor(() => expect(screen.getAllByText("Thoughts")).toHaveLength(2));
    expect(screen.getAllByText(/·\s*1s/)).toHaveLength(1);
  });

  it("thoughts round-trip through the saved session and reopen with their row intact", async () => {
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "thinking", text: "**Weighing options**\n" });
      onEvent({ type: "textDelta", text: "answer" });
      onEvent({ type: "turnDone" });
      return null;
    });
    const ref = createRef<AssistantConversationHandle>();
    render(<AssistantConversation ref={ref} />);
    fireEvent.change(await screen.findByPlaceholderText(/ask/i), { target: { value: "think hard" } });
    fireEvent.click(screen.getByRole("button", { name: /send/i }));

    // The auto-save carries the reasoning, not just text and tool calls.
    await waitFor(() => expect(chatHistory.saveSession).toHaveBeenCalled());
    const calls = vi.mocked(chatHistory.saveSession).mock.calls;
    const savedMsg = (calls[calls.length - 1][0].messages as StoredMessage[]).find((m) => m.role === "assistant")!;
    expect(savedMsg.thoughts).toBe("**Weighing options**\n");
    expect(savedMsg.thoughtSecs).toBe(1);

    // Reopening a stored session restores the collapsible Thoughts row.
    vi.mocked(chatHistory.loadSession).mockResolvedValue({
      id: "old",
      title: "old",
      createdAt: 1,
      updatedAt: 2,
      contexts: [],
      skills: [],
      cliSessionId: null,
      messages: [
        { id: 0, role: "user", text: "earlier question" },
        { id: 1, role: "assistant", text: "earlier answer", thoughts: "**Recalling context**\n", thoughtSecs: 4 },
      ],
    });
    await act(async () => void ref.current!.selectSession("old"));
    expect(await screen.findByText("earlier answer")).toBeTruthy();
    expect(screen.getByText("Thoughts")).toBeTruthy();
    expect(screen.getByText(/·\s*4s/)).toBeTruthy();
  });

  it("auto-save records the attached context under `contexts`", async () => {
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "textDelta", text: "ok" });
      onEvent({ type: "turnDone" });
      return null;
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
      return null;
    });
    render(<AssistantConversation />);
    fireEvent.change(await screen.findByPlaceholderText(/ask/i), { target: { value: "hi" } });
    fireEvent.click(screen.getByRole("button", { name: /send/i }));

    await waitFor(() => expect(screen.getByText("boom")).toBeTruthy());
    await waitFor(() => expect(chatHistory.saveSession).toHaveBeenCalledTimes(1));
    const saved = vi.mocked(chatHistory.saveSession).mock.calls[0][0];
    expect(saved.messages).toHaveLength(3);
    // The error slots in BEFORE the untouched assistant placeholder (which
    // stays last as the stream target — see the advisory-error test below).
    expect(saved.messages[1]).toEqual({ id: 2, role: "error", text: "boom" });
  });

  it("an advisory error mid-turn doesn't swallow the reply that streams after it", async () => {
    // The real backend's unsupported-attachment shape: an `error` first, then
    // the turn proceeds text-only. The error must not orphan the assistant
    // placeholder (textDelta targets the LAST message) or the whole reply
    // would be silently discarded while the provider call still ran.
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "error", message: "image attachments aren't supported by the srelens agent yet" });
      onEvent({ type: "textDelta", text: "here is the text-only answer" });
      onEvent({ type: "turnDone" });
      return null;
    });
    render(<AssistantConversation />);
    fireEvent.change(await screen.findByPlaceholderText(/ask/i), { target: { value: "hi" } });
    fireEvent.click(screen.getByRole("button", { name: /send/i }));

    await waitFor(() => expect(screen.getByText("here is the text-only answer")).toBeTruthy());
    expect(screen.getByText(/attachments aren't supported/)).toBeTruthy();
  });

  it("New chat clears the transcript, and the next send mints a fresh session id", async () => {
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "textDelta", text: "first reply" });
      onEvent({ type: "turnDone" });
      return null;
    });
    render(<AssistantConversation />);
    fireEvent.change(await screen.findByPlaceholderText(/ask/i), { target: { value: "first question" } });
    fireEvent.click(screen.getByRole("button", { name: /send/i }));
    await waitFor(() => expect(screen.getByText("first reply")).toBeTruthy());
    await waitFor(() => expect(chatHistory.saveSession).toHaveBeenCalledTimes(1));

    await openHistory();
    fireEvent.click(screen.getByRole("button", { name: /new chat/i }));
    // The transcript is empty again — back to the placeholder — even though
    // the just-saved session still shows up in the history popover (New chat
    // clears the *editing* state; it doesn't delete anything from disk).
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
      return null;
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
      return null;
    });
    render(<AssistantConversation />);
    await openHistory();
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
    await openHistory();
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
    await openHistory();
    await screen.findByText("Old chat");

    fireEvent.click(screen.getByLabelText("Delete Old chat"));

    await waitFor(() => expect(chatHistory.deleteSession).toHaveBeenCalledWith("old-1"));
    expect(screen.queryByText("Old chat")).toBeFalsy();
  });

  it("delete waits for an in-flight transcript save before touching the disk", async () => {
    vi.mocked(chatHistory.listSessions).mockResolvedValue([
      { id: "s1", title: "Old chat", createdAt: 1, updatedAt: 2 },
    ]);
    // Hold the auto-save open: the turn settles (Send returns) while the
    // session file is still being written.
    let resolveSave: () => void = () => {};
    vi.mocked(chatHistory.saveSession).mockImplementation(
      () => new Promise<void>((resolve) => { resolveSave = () => resolve(); }),
    );
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "textDelta", text: "ok" });
      onEvent({ type: "turnDone" });
      return null;
    });
    render(<AssistantConversation />);
    fireEvent.change(await screen.findByPlaceholderText(/ask/i), { target: { value: "hi" } });
    fireEvent.click(screen.getByRole("button", { name: /^send$/i }));
    await waitFor(() => expect(chatHistory.saveSession).toHaveBeenCalledTimes(1));

    await openHistory();
    fireEvent.click(await screen.findByLabelText("Delete Old chat"));
    // The save hasn't landed yet — deleting now would let it recreate the
    // file/index entry afterward, so the delete must hold until it settles.
    await new Promise((r) => setTimeout(r, 0));
    expect(chatHistory.deleteSession).not.toHaveBeenCalled();

    resolveSave();
    await waitFor(() => expect(chatHistory.deleteSession).toHaveBeenCalledWith("s1"));
  });

  it("renders the session list in the newest-first order listSessions returns", async () => {
    vi.mocked(chatHistory.listSessions).mockResolvedValue([
      { id: "newest", title: "Newest chat", createdAt: 1, updatedAt: 300 },
      { id: "oldest", title: "Oldest chat", createdAt: 1, updatedAt: 10 },
    ]);
    render(<AssistantConversation />);
    await openHistory();
    await screen.findByText("Newest chat");

    const newest = screen.getByText("Newest chat");
    const oldest = screen.getByText("Oldest chat");
    expect(newest.compareDocumentPosition(oldest) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });
});

describe("AssistantConversation New chat resets composer state", () => {
  // 3 zero bytes -> base64 "AAAA" with no padding — mirrors the helper in the
  // image-attachments describe block below (not shared across `describe`s).
  function pngFile(name = "a.png"): File {
    return new File([Uint8Array.from([0, 0, 0])], name, { type: "image/png" });
  }

  it("New chat clears skill and context chips restored from a reopened session", async () => {
    vi.mocked(chatHistory.listSessions).mockResolvedValue([
      { id: "old-1", title: "Old chat", createdAt: 1, updatedAt: 2 },
    ]);
    vi.mocked(chatHistory.loadSession).mockResolvedValue({
      id: "old-1",
      title: "Old chat",
      createdAt: 1,
      updatedAt: 2,
      contexts: ["prod"],
      skills: ["deploy-triage"],
      cliSessionId: null,
      messages: [{ id: 0, role: "user", text: "hi" }],
    });
    render(<AssistantConversation availableContexts={["prod", "staging"]} />);
    await openHistory();
    fireEvent.click(await screen.findByText("Old chat"));
    await screen.findByText("hi");

    // The reopened session's skill and context chips are visible.
    expect(screen.getByLabelText("Remove skill deploy-triage")).toBeTruthy();
    expect(screen.getByLabelText("Remove prod")).toBeTruthy();

    await openHistory();
    fireEvent.click(screen.getByRole("button", { name: /new chat/i }));

    // A true fresh start: neither chip survives New chat.
    expect(screen.queryByLabelText("Remove skill deploy-triage")).toBeFalsy();
    expect(screen.queryByLabelText("Remove prod")).toBeFalsy();
  });

  it("after New chat, a freshly picked context sends with no bleed from the previous session's skill/context", async () => {
    vi.mocked(chatHistory.listSessions).mockResolvedValue([
      { id: "old-1", title: "Old chat", createdAt: 1, updatedAt: 2 },
    ]);
    vi.mocked(chatHistory.loadSession).mockResolvedValue({
      id: "old-1",
      title: "Old chat",
      createdAt: 1,
      updatedAt: 2,
      contexts: ["prod"],
      skills: ["deploy-triage"],
      cliSessionId: null,
      messages: [{ id: 0, role: "user", text: "hi" }],
    });
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "turnDone" });
      return null;
    });
    render(<AssistantConversation availableContexts={["prod", "staging"]} />);
    await openHistory();
    fireEvent.click(await screen.findByText("Old chat"));
    await screen.findByText("hi");

    await openHistory();
    fireEvent.click(screen.getByRole("button", { name: /new chat/i }));

    fireEvent.click(await screen.findByRole("button", { name: /contexts \(0\)/i }));
    fireEvent.click(await screen.findByText("staging"));

    fireEvent.change(screen.getByPlaceholderText(/ask/i), { target: { value: "what's up?" } });
    fireEvent.click(screen.getByRole("button", { name: /^send$/i }));

    await waitFor(() => expect(chat.sendChat).toHaveBeenCalled());
    const sentPrompt = vi.mocked(chat.sendChat).mock.calls[0][1] as string;
    // Staging's single-context preface is present...
    expect(sentPrompt).toContain("Work in the cluster `staging` (the default context).");
    // ...but nothing from the previous session (its skill guidance or its
    // "prod" context) leaked into this brand-new one.
    expect(sentPrompt).not.toContain("deploy-triage");
    expect(sentPrompt).not.toContain("Apply these skills");
    expect(sentPrompt).not.toContain("prod");
  });

  it("New chat clears a pending image thumbnail", async () => {
    render(<AssistantConversation />);
    fireEvent.change(await screen.findByLabelText(/attach image/i), { target: { files: [pngFile()] } });
    await screen.findByAltText(/pending image 1/i);

    await openHistory();
    fireEvent.click(screen.getByRole("button", { name: /new chat/i }));

    expect(screen.queryByAltText(/pending image 1/i)).toBeFalsy();
  });
});

describe("AssistantConversation image attachments", () => {
  // 3 zero bytes -> base64 "AAAA" with no padding, so the data URI jsdom's
  // real (in-memory, no filesystem) FileReader produces is a stable literal:
  // `data:image/png;base64,AAAA`.
  function pngFile(name = "a.png"): File {
    return new File([Uint8Array.from([0, 0, 0])], name, { type: "image/png" });
  }

  // Different byte content (4, 16, 65) -> base64 "BBBB" — distinguishable
  // from `pngFile()`'s "AAAA" so a two-image test can tell them apart.
  function pngFile2(name = "b.png"): File {
    return new File([Uint8Array.from([4, 16, 65])], name, { type: "image/png" });
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

  it("Send is held while an attachment is still being read, then includes it", async () => {
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "turnDone" });
      return null;
    });
    render(<AssistantConversation />);
    fireEvent.change(await screen.findByPlaceholderText(/ask/i), { target: { value: "look at this" } });
    fireEvent.change(await screen.findByLabelText(/attach image/i), { target: { files: [pngFile()] } });
    // Immediately after selecting: the FileReader hasn't resolved yet — this
    // click must not send without (and thereby orphan) the attachment.
    fireEvent.click(screen.getByRole("button", { name: /^send$/i }));
    expect(chat.sendChat).not.toHaveBeenCalled();

    await screen.findByAltText(/pending image 1/i);
    fireEvent.click(screen.getByRole("button", { name: /^send$/i }));
    await waitFor(() => expect(chat.sendChat).toHaveBeenCalledTimes(1));
    expect(vi.mocked(chat.sendChat).mock.calls[0][4]).toEqual(["AAAA"]);
  });

  it("removing a pending image chip clears it, and it isn't sent", async () => {
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "turnDone" });
      return null;
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

  it("removing a non-last pending image (index 0 of 2) keeps the other one, not just drops the last", async () => {
    // Guards against a regressed `removePendingImage` that always drops the
    // last element (e.g. `imgs.slice(0, -1)`) — with only one pending image
    // (as in the test above) that bug would still pass.
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "turnDone" });
      return null;
    });
    render(<AssistantConversation />);
    fireEvent.change(await screen.findByLabelText(/attach image/i), { target: { files: [pngFile(), pngFile2()] } });
    await screen.findByAltText(/pending image 2/i);

    // Remove the first chip (base64 "AAAA") — the second ("BBBB") must remain.
    fireEvent.click(screen.getByLabelText(/remove image 1/i));

    expect(screen.queryByAltText(/pending image 2/i)).toBeFalsy();
    const remaining = (await screen.findByAltText(/pending image 1/i)) as HTMLImageElement;
    expect(remaining.src).toBe("data:image/png;base64,BBBB");

    fireEvent.change(screen.getByPlaceholderText(/ask/i), { target: { value: "hi" } });
    fireEvent.click(screen.getByRole("button", { name: /^send$/i }));

    await waitFor(() => expect(chat.sendChat).toHaveBeenCalled());
    expect(vi.mocked(chat.sendChat).mock.calls[0][4]).toEqual(["BBBB"]);
  });

  it("sending strips the data URI prefix, passing raw base64 as sendChat's 5th arg", async () => {
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "turnDone" });
      return null;
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
      return null;
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
      return null;
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
    await openHistory();
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
      return null;
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
      return null;
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
      return null;
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
    await openHistory();
    fireEvent.click(await screen.findByText("Old chat"));

    await screen.findByText("hi");
    expect(screen.getByLabelText("Remove a")).toBeTruthy();
    expect(screen.getByRole("button", { name: /contexts \(1\)/i })).toBeTruthy();
  });
});

describe("AssistantConversation composer (Task 19)", () => {
  it("groups the inline agent picker and attach control into one composer surface", async () => {
    render(<AssistantConversation />);
    const composer = (await screen.findByTestId("assistant-composer")) as HTMLElement;
    const { getByRole: composerRole, getByLabelText: composerLabel } = within(composer);
    expect(composerRole("combobox", { name: /agent/i })).toBeTruthy();
    expect(composerLabel(/attach image/i)).toBeTruthy();
  });

  it("also places the multi-context control inside the composer on the global tab", async () => {
    render(<AssistantConversation availableContexts={["a", "b"]} />);
    const composer = (await screen.findByTestId("assistant-composer")) as HTMLElement;
    expect(within(composer).getByRole("button", { name: /contexts \(0\)/i })).toBeTruthy();
  });

  it("Send becomes Stop while a turn is streaming, and Stop calls cancelChat with the active session and turn", async () => {
    vi.mocked(chat.cancelChat).mockResolvedValue(undefined);
    let resolveSend: () => void = () => {};
    vi.mocked(chat.sendChat).mockImplementation(
      () =>
        new Promise((resolve) => {
          resolveSend = () => resolve(null);
        }),
    );
    render(<AssistantConversation />);
    fireEvent.change(await screen.findByPlaceholderText(/ask/i), { target: { value: "keep going" } });
    fireEvent.click(screen.getByRole("button", { name: /^send$/i }));

    const stopButton = await screen.findByRole("button", { name: /^stop$/i });
    expect(screen.queryByRole("button", { name: /^send$/i })).toBeFalsy();

    fireEvent.click(stopButton);
    // The turn generation matches what sendChat was stamped with (first turn
    // bumps the nonce to 1), so the backend can pair this Stop with that send.
    expect(chat.cancelChat).toHaveBeenCalledWith("s1", 1);

    // Clean up the still-pending sendChat promise so it doesn't leak into
    // the next test.
    resolveSend();
    await waitFor(() => expect(screen.getByRole("button", { name: /^send$/i })).toBeTruthy());
  });

  it("Send is restored once the (cancelled) turn settles", async () => {
    vi.mocked(chat.cancelChat).mockResolvedValue(undefined);
    vi.mocked(chat.sendChat).mockResolvedValue(null);
    render(<AssistantConversation />);
    fireEvent.change(await screen.findByPlaceholderText(/ask/i), { target: { value: "hi" } });
    fireEvent.click(screen.getByRole("button", { name: /^send$/i }));

    await waitFor(() => expect(screen.getByRole("button", { name: /^send$/i })).toBeTruthy());
  });

  it("cancels the in-flight turn when the conversation unmounts", async () => {
    vi.mocked(chat.cancelChat).mockResolvedValue(undefined);
    let resolveSend: () => void = () => {};
    vi.mocked(chat.sendChat).mockImplementation(
      () => new Promise((resolve) => { resolveSend = () => resolve(null); }),
    );
    const { unmount } = render(<AssistantConversation />);
    fireEvent.change(await screen.findByPlaceholderText(/ask/i), { target: { value: "keep going" } });
    fireEvent.click(screen.getByRole("button", { name: /^send$/i }));
    await screen.findByRole("button", { name: /^stop$/i });

    // Closing the drawer/tab (unmount) mid-turn must stop the backend turn.
    unmount();
    expect(chat.cancelChat).toHaveBeenCalled();

    resolveSend();
  });

  it("Stop pressed while the turn is still preparing prevents the agent launch", async () => {
    vi.mocked(chat.cancelChat).mockResolvedValue(undefined);
    // Hold the turn in its prep phase: startChat never resolves until we say so,
    // so no child exists yet and cancelChat would be a no-op.
    let resolveStart: (id: string) => void = () => {};
    vi.mocked(chat.startChat).mockImplementation(
      () => new Promise((resolve) => { resolveStart = (id) => resolve(id); }),
    );
    render(<AssistantConversation />);
    fireEvent.change(await screen.findByPlaceholderText(/ask/i), { target: { value: "do a thing" } });
    fireEvent.click(screen.getByRole("button", { name: /^send$/i }));

    // Mid-prep the button is Stop; press it before any child is registered.
    fireEvent.click(await screen.findByRole("button", { name: /^stop$/i }));

    // Now let startChat resolve — handleSend must see the cancel and bail.
    await act(async () => resolveStart("s1"));
    await waitFor(() => expect(screen.getByRole("button", { name: /^send$/i })).toBeTruthy());
    expect(chat.sendChat).not.toHaveBeenCalled();
  });

  it("re-lists agents when the vault gate reports the unlock", async () => {
    // Mounted under the gate: the first listAgents sees a locked vault, so
    // the native agent reads unavailable.
    vi.mocked(chat.listAgents).mockResolvedValue([
      { kind: "srelens", label: "srelens agent", available: false, path: null, version: null, installUrl: "", gated: false },
    ]);
    render(<AssistantConversation />);
    await waitFor(() => expect(chat.listAgents).toHaveBeenCalledTimes(1));

    // The gate unlocks — the broadcast must trigger a fresh listing that now
    // sees the key and enables the agent.
    vi.mocked(chat.listAgents).mockResolvedValue([
      { kind: "srelens", label: "srelens agent", available: true, path: null, version: null, installUrl: "", gated: false },
    ]);
    act(() => {
      window.dispatchEvent(new Event("srelens:vault-unlocked"));
    });
    await waitFor(() => expect(chat.listAgents).toHaveBeenCalledTimes(2));
  });

  it("unmounting while startChat is still pending prevents the agent launch", async () => {
    vi.mocked(chat.cancelChat).mockResolvedValue(undefined);
    // Hold the first send in prep: no session id exists yet, so the unmount
    // cleanup has nothing to cancel on the backend — it must still flag the
    // cancel so the resolved prep doesn't launch an invisible turn.
    let resolveStart: (id: string) => void = () => {};
    vi.mocked(chat.startChat).mockImplementation(
      () => new Promise((resolve) => { resolveStart = (id) => resolve(id); }),
    );
    const { unmount } = render(<AssistantConversation />);
    fireEvent.change(await screen.findByPlaceholderText(/ask/i), { target: { value: "do a thing" } });
    fireEvent.click(screen.getByRole("button", { name: /^send$/i }));

    unmount();
    await act(async () => resolveStart("s1"));
    expect(chat.sendChat).not.toHaveBeenCalled();
  });

  it("a slow session load can't clobber a newer selection", async () => {
    const resolvers: Record<string, (v: unknown) => void> = {};
    vi.mocked(chatHistory.loadSession).mockImplementation(
      (id: string) =>
        new Promise((resolve) => {
          resolvers[id] = resolve as (v: unknown) => void;
        }),
    );
    const sessionOf = (id: string, text: string) => ({
      id,
      title: id,
      createdAt: 1,
      updatedAt: 2,
      contexts: [],
      skills: [],
      cliSessionId: null,
      messages: [{ id: 0, role: "user", text }],
    });

    const ref = createRef<AssistantConversationHandle>();
    render(<AssistantConversation ref={ref} />);
    // Two selections in flight; the first (A) will resolve LAST.
    act(() => void ref.current!.selectSession("A"));
    act(() => void ref.current!.selectSession("B"));
    // Loads sit behind the (empty) persist chain — flush the microtask so both
    // `loadSession` calls have actually been issued before resolving them.
    await act(async () => {});

    // Resolve the newest (B) first, then the stale A.
    await act(async () => resolvers["B"](sessionOf("B", "from session B")));
    await act(async () => resolvers["A"](sessionOf("A", "from session A")));

    // B's transcript stands; the late A load was discarded.
    expect(await screen.findByText("from session B")).toBeTruthy();
    expect(screen.queryByText("from session A")).toBeFalsy();
  });

  it("a send invalidates a still-pending session load so it can't swap the transcript mid-turn", async () => {
    const resolvers: Record<string, (v: unknown) => void> = {};
    vi.mocked(chatHistory.loadSession).mockImplementation(
      (id: string) =>
        new Promise((resolve) => {
          resolvers[id] = resolve as (v: unknown) => void;
        }),
    );
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "textDelta", text: "fresh reply" });
      onEvent({ type: "turnDone" });
      return null;
    });
    const ref = createRef<AssistantConversationHandle>();
    render(<AssistantConversation ref={ref} />);
    act(() => void ref.current!.selectSession("A"));
    await act(async () => {}); // the load is now issued and pending

    // The user sends from the still-visible old conversation before A loads.
    fireEvent.change(await screen.findByPlaceholderText(/ask/i), { target: { value: "hello" } });
    fireEvent.click(screen.getByRole("button", { name: /^send$/i }));
    await screen.findByText("fresh reply");

    // The stale load resolving now must be discarded, not replace the turn.
    await act(async () =>
      resolvers["A"]({
        id: "A",
        title: "A",
        createdAt: 1,
        updatedAt: 2,
        contexts: [],
        skills: [],
        cliSessionId: null,
        messages: [{ id: 0, role: "user", text: "from session A" }],
      }),
    );
    expect(screen.queryByText("from session A")).toBeFalsy();
    expect(screen.getByText("fresh reply")).toBeTruthy();
  });
});

describe("AssistantConversation slash menu (Task 21)", () => {
  const summaries = [
    { name: "pod-crashloop", description: "Work out why a pod keeps restarting", arguments: [] },
    { name: "pod-pending", description: "Work out why a pod is stuck pending", arguments: [] },
  ];

  it("typing `/` opens a menu listing the mocked prompt summaries by name and description", async () => {
    vi.mocked(prompts.listPrompts).mockResolvedValue(summaries);
    render(<AssistantConversation />);
    fireEvent.change(await screen.findByPlaceholderText(/ask/i), { target: { value: "/" } });

    expect(await screen.findByText("pod-crashloop")).toBeTruthy();
    expect(screen.getByText("Work out why a pod keeps restarting")).toBeTruthy();
    expect(screen.getByText("pod-pending")).toBeTruthy();
  });

  it("selecting a prompt renders it via getPrompt with a context arg, fills the input, and does NOT send", async () => {
    vi.mocked(prompts.listPrompts).mockResolvedValue(summaries);
    vi.mocked(prompts.getPrompt).mockResolvedValue("Triage `pod-a` on `prod-cluster`.");
    render(<AssistantConversation context={{ context: "prod-cluster" }} />);
    fireEvent.change(await screen.findByPlaceholderText(/ask/i), { target: { value: "/" } });

    fireEvent.click(await screen.findByText("pod-crashloop"));

    await waitFor(() =>
      expect(prompts.getPrompt).toHaveBeenCalledWith("pod-crashloop", { context: "prod-cluster" }),
    );
    expect((await screen.findByPlaceholderText(/ask/i) as HTMLInputElement).value).toBe(
      "Triage `pod-a` on `prod-cluster`.",
    );
    expect(chat.sendChat).not.toHaveBeenCalled();
    // The menu itself is gone once a prompt has been picked.
    expect(screen.queryByText("pod-pending")).toBeFalsy();
  });

  it("Escape closes the menu without changing the input", async () => {
    vi.mocked(prompts.listPrompts).mockResolvedValue(summaries);
    render(<AssistantConversation />);
    const input = await screen.findByPlaceholderText(/ask/i);
    fireEvent.change(input, { target: { value: "/" } });
    await screen.findByText("pod-crashloop");

    fireEvent.keyDown(input, { key: "Escape" });

    expect(screen.queryByText("pod-crashloop")).toBeFalsy();
    expect((input as HTMLInputElement).value).toBe("/");
  });

  it("a rejected getPrompt surfaces a small inline error and leaves the input as-is", async () => {
    vi.mocked(prompts.listPrompts).mockResolvedValue(summaries);
    vi.mocked(prompts.getPrompt).mockRejectedValue(new Error("missing required argument `context`"));
    render(<AssistantConversation />);
    fireEvent.change(await screen.findByPlaceholderText(/ask/i), { target: { value: "/" } });

    fireEvent.click(await screen.findByText("pod-crashloop"));

    expect(await screen.findByText(/missing required argument `context`/)).toBeTruthy();
    expect((screen.getByPlaceholderText(/ask/i) as HTMLInputElement).value).toBe("/");
  });
});

describe("AssistantConversation skills activation (Task 23)", () => {
  const SKILL_METAS = [
    { name: "crashloop-triage", description: "Systematic triage for a crashlooping pod" },
    { name: "pending-triage", description: "Work out why a pod is stuck pending" },
  ];

  it("typing `/` lists skills under a Skills group alongside a Prompts group", async () => {
    vi.mocked(prompts.listPrompts).mockResolvedValue([
      { name: "pod-crashloop", description: "Work out why a pod keeps restarting", arguments: [] },
    ]);
    vi.mocked(skills.listSkills).mockResolvedValue(SKILL_METAS);
    render(<AssistantConversation />);
    fireEvent.change(await screen.findByPlaceholderText(/ask/i), { target: { value: "/" } });

    expect(await screen.findByText("Prompts")).toBeTruthy();
    expect(screen.getByText("Skills")).toBeTruthy();
    expect(screen.getByText("pod-crashloop")).toBeTruthy();
    expect(screen.getByText("crashloop-triage")).toBeTruthy();
    expect(screen.getByText("pending-triage")).toBeTruthy();
  });

  it("selecting a skill adds a removable chip and does NOT fill the input (unlike picking a prompt)", async () => {
    vi.mocked(skills.listSkills).mockResolvedValue(SKILL_METAS);
    render(<AssistantConversation />);
    fireEvent.change(await screen.findByPlaceholderText(/ask/i), { target: { value: "/" } });

    fireEvent.click(await screen.findByText("crashloop-triage"));

    expect(await screen.findByLabelText("Remove skill crashloop-triage")).toBeTruthy();
    expect((screen.getByPlaceholderText(/ask/i) as HTMLInputElement).value).toBe("/");
    expect(prompts.getPrompt).not.toHaveBeenCalled();
  });

  it("selecting the same skill twice does not duplicate its chip", async () => {
    vi.mocked(skills.listSkills).mockResolvedValue(SKILL_METAS);
    render(<AssistantConversation />);
    fireEvent.change(await screen.findByPlaceholderText(/ask/i), { target: { value: "/" } });
    fireEvent.click(await screen.findByText("crashloop-triage"));
    await screen.findByLabelText("Remove skill crashloop-triage");

    fireEvent.change(screen.getByPlaceholderText(/ask/i), { target: { value: "/" } });
    fireEvent.click(await screen.findByText("crashloop-triage"));

    expect(screen.getAllByLabelText("Remove skill crashloop-triage")).toHaveLength(1);
  });

  it("removing the chip drops the activated skill", async () => {
    vi.mocked(skills.listSkills).mockResolvedValue(SKILL_METAS);
    render(<AssistantConversation />);
    fireEvent.change(await screen.findByPlaceholderText(/ask/i), { target: { value: "/" } });
    fireEvent.click(await screen.findByText("crashloop-triage"));
    await screen.findByLabelText("Remove skill crashloop-triage");

    fireEvent.click(screen.getByLabelText("Remove skill crashloop-triage"));

    expect(screen.queryByLabelText("Remove skill crashloop-triage")).toBeFalsy();
  });

  it("on send, fetches each active skill's body and prepends a guidance block after the preface and before the user text; the visible bubble shows only the typed text", async () => {
    vi.mocked(skills.listSkills).mockResolvedValue(SKILL_METAS);
    vi.mocked(skills.loadSkill).mockImplementation(async (name: string) => ({
      name,
      description: "",
      body: "Check the exit code first.",
    }));
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "turnDone" });
      return null;
    });
    render(<AssistantConversation context={{ context: "prod-cluster" }} />);
    fireEvent.change(await screen.findByPlaceholderText(/ask/i), { target: { value: "/" } });
    fireEvent.click(await screen.findByText("crashloop-triage"));
    fireEvent.change(screen.getByPlaceholderText(/ask/i), { target: { value: "pod-a is restarting" } });
    fireEvent.click(screen.getByRole("button", { name: /^send$/i }));

    await waitFor(() => expect(chat.sendChat).toHaveBeenCalled());
    expect(skills.loadSkill).toHaveBeenCalledWith("crashloop-triage");
    // Hand-written expected string built from the mocked skill body — never
    // echoed from the component's own guidance-block builder.
    expect(vi.mocked(chat.sendChat).mock.calls[0][1]).toBe(
      "Current context: cluster prod-cluster.\n\n" +
        "Apply these skills:\n\n" +
        "Check the exit code first.\n\n" +
        "pod-a is restarting",
    );

    // The visible user bubble shows only the typed text, never the guidance.
    expect(await screen.findByText("pod-a is restarting")).toBeTruthy();
    expect(screen.queryByText(/apply these skills/i)).toBeFalsy();
    expect(screen.queryByText(/check the exit code first/i)).toBeFalsy();
  });

  it("persists activeSkills into the saved session's `skills` field", async () => {
    vi.mocked(skills.listSkills).mockResolvedValue(SKILL_METAS);
    vi.mocked(skills.loadSkill).mockResolvedValue({
      name: "crashloop-triage",
      description: "Systematic triage for a crashlooping pod",
      body: "Check the exit code first.",
    });
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "turnDone" });
      return null;
    });
    render(<AssistantConversation />);
    fireEvent.change(await screen.findByPlaceholderText(/ask/i), { target: { value: "/" } });
    fireEvent.click(await screen.findByText("crashloop-triage"));
    fireEvent.change(screen.getByPlaceholderText(/ask/i), { target: { value: "hi" } });
    fireEvent.click(screen.getByRole("button", { name: /^send$/i }));

    await waitFor(() => expect(chatHistory.saveSession).toHaveBeenCalledTimes(1));
    expect(vi.mocked(chatHistory.saveSession).mock.calls[0][0].skills).toEqual(["crashloop-triage"]);
  });

  it("a reloaded session restores its chips from the session's `skills`", async () => {
    vi.mocked(chatHistory.listSessions).mockResolvedValue([
      { id: "old-1", title: "Old chat", createdAt: 1, updatedAt: 2 },
    ]);
    vi.mocked(chatHistory.loadSession).mockResolvedValue({
      id: "old-1",
      title: "Old chat",
      createdAt: 1,
      updatedAt: 2,
      contexts: [],
      skills: ["crashloop-triage"],
      cliSessionId: null,
      messages: [{ id: 0, role: "user", text: "hi" }],
    });
    render(<AssistantConversation />);
    await openHistory();
    fireEvent.click(await screen.findByText("Old chat"));

    await screen.findByText("hi");
    expect(screen.getByLabelText("Remove skill crashloop-triage")).toBeTruthy();
  });

  it("a rejected loadSkill for one active skill doesn't abort the turn — its guidance is dropped, the surviving skill's still sends", async () => {
    vi.mocked(skills.listSkills).mockResolvedValue(SKILL_METAS);
    vi.mocked(skills.loadSkill).mockImplementation(async (name: string) => {
      if (name === "pending-triage") throw new Error("skill file missing");
      return { name, description: "", body: "Check the exit code first." };
    });
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "turnDone" });
      return null;
    });
    render(<AssistantConversation />);
    fireEvent.change(await screen.findByPlaceholderText(/ask/i), { target: { value: "/" } });
    fireEvent.click(await screen.findByText("crashloop-triage"));
    // Re-typing the exact same "/" value here wouldn't re-fire React's
    // onChange (the DOM value is already "/", unchanged since the line
    // above), so the token narrows to "/pending" instead — a genuinely
    // different value that reopens the dismissed menu, filtered to the one
    // remaining match.
    fireEvent.change(screen.getByPlaceholderText(/ask/i), { target: { value: "/pending" } });
    fireEvent.click(await screen.findByText("pending-triage"));
    fireEvent.change(screen.getByPlaceholderText(/ask/i), { target: { value: "pod-a is restarting" } });
    fireEvent.click(screen.getByRole("button", { name: /^send$/i }));

    await waitFor(() => expect(chat.sendChat).toHaveBeenCalled());
    // Hand-written literal: only the surviving skill's guidance appears.
    expect(vi.mocked(chat.sendChat).mock.calls[0][1]).toBe(
      "Apply these skills:\n\nCheck the exit code first.\n\npod-a is restarting",
    );
    // The turn still sent despite the missing skill — no generic transport
    // error bubble from an aborted `handleSend`.
    expect(screen.queryByText(/skill file missing/i)).toBeFalsy();
  });
});

describe("AssistantConversation answer layout", () => {
  it("folds a turn's tool calls into a collapsed Tools group, hidden until expanded", async () => {
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "toolCallStart", id: "t1", tool: "k8s.listPods", args: {} });
      onEvent({ type: "toolResult", id: "t1", status: "ok" });
      onEvent({ type: "textDelta", text: "Here are the pods." });
      onEvent({ type: "turnDone" });
      return null;
    });
    render(<AssistantConversation />);
    fireEvent.change(await screen.findByPlaceholderText(/ask/i), { target: { value: "pods?" } });
    fireEvent.click(screen.getByRole("button", { name: /^send$/i }));
    await screen.findByText(/here are the pods/i);
    // Collapsed by default — the individual call isn't in the DOM yet.
    expect(screen.queryByText("k8s.listPods")).toBeFalsy();
    fireEvent.click(screen.getByRole("button", { name: /tools \(1\)/i }));
    expect(screen.getByText("k8s.listPods")).toBeTruthy();
  });

  it("a turn ending with an unresolved tool call settles it instead of leaving a spinner", async () => {
    // Stop (or a crash) ends the turn before the toolResult ever arrives.
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "toolCallStart", id: "t1", tool: "k8s.listPods", args: {} });
      onEvent({ type: "turnDone" });
      return null;
    });
    render(<AssistantConversation />);
    fireEvent.change(await screen.findByPlaceholderText(/ask/i), { target: { value: "pods?" } });
    fireEvent.click(screen.getByRole("button", { name: /^send$/i }));
    await waitFor(() => expect(screen.getByRole("button", { name: /^send$/i })).toBeTruthy());

    // The group shows a settled failure, not a forever-running spinner.
    fireEvent.click(screen.getByRole("button", { name: /tools \(1\)/i }));
    expect(screen.queryByLabelText(/running/i)).toBeFalsy();
    // And the persisted transcript carries the settled status too.
    await waitFor(() => expect(chatHistory.saveSession).toHaveBeenCalled());
    const saved = vi.mocked(chatHistory.saveSession).mock.calls.at(-1)![0];
    const messages = saved.messages as Array<{ role: string; toolCalls?: Array<{ status: string | null }> }>;
    const assistant = messages.find((m) => m.role === "assistant" && m.toolCalls);
    expect(assistant?.toolCalls?.[0].status).toBe("error");
  });

  it("copies the assistant answer to the clipboard", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.assign(navigator, { clipboard: { writeText } });
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "textDelta", text: "The answer." });
      onEvent({ type: "turnDone" });
      return null;
    });
    render(<AssistantConversation />);
    fireEvent.change(await screen.findByPlaceholderText(/ask/i), { target: { value: "q" } });
    fireEvent.click(screen.getByRole("button", { name: /^send$/i }));
    await screen.findByText(/the answer/i);
    fireEvent.click(screen.getByRole("button", { name: /copy answer/i }));
    expect(writeText).toHaveBeenCalledWith("The answer.");
  });

  it("folds streamed thinking into a collapsed Thoughts group above the answer", async () => {
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "thinking", text: "Let me reason about this." });
      onEvent({ type: "textDelta", text: "The answer." });
      onEvent({ type: "turnDone" });
      return null;
    });
    render(<AssistantConversation />);
    fireEvent.change(await screen.findByPlaceholderText(/ask/i), { target: { value: "q" } });
    fireEvent.click(screen.getByRole("button", { name: /^send$/i }));
    await screen.findByText(/the answer/i);
    // Collapsed by default — the reasoning text isn't shown until expanded.
    expect(screen.queryByText(/let me reason/i)).toBeFalsy();
    fireEvent.click(screen.getByRole("button", { name: /thoughts/i }));
    expect(screen.getByText(/let me reason/i)).toBeTruthy();
  });
});

describe("AssistantConversation agent persistence", () => {
  const twoAgents: chat.AgentInfo[] = [
    { kind: "claude", label: "Claude Code", available: true, path: "/usr/bin/claude", version: null, installUrl: "", gated: false },
    { kind: "codex", label: "Codex", available: true, path: "/usr/bin/codex", version: null, installUrl: "", gated: false },
  ];

  it("saves the selected agent kind with the session", async () => {
    vi.mocked(chat.listAgents).mockResolvedValue(twoAgents);
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "turnDone" });
      return null;
    });
    render(<AssistantConversation />);
    fireEvent.click(await screen.findByRole("combobox", { name: /agent/i }));
    fireEvent.click(await screen.findByRole("option", { name: /codex/i }));
    fireEvent.change(screen.getByPlaceholderText(/ask/i), { target: { value: "hi" } });
    fireEvent.click(screen.getByRole("button", { name: /^send$/i }));
    await waitFor(() => expect(chatHistory.saveSession).toHaveBeenCalled());
    expect(vi.mocked(chatHistory.saveSession).mock.calls[0][0].agentKind).toBe("codex");
  });

  it("a fresh chat defaults to the last-used agent from localStorage, not always the first", async () => {
    localStorage.setItem("srelens.assistant.lastAgent", "codex");
    vi.mocked(chat.listAgents).mockResolvedValue(twoAgents); // claude (first) + codex
    render(<AssistantConversation />);
    const trigger = await screen.findByRole("combobox", { name: /agent/i });
    // Codex was last used, so it wins over the first-in-list Claude default.
    await waitFor(() => expect(trigger.textContent).toMatch(/codex/i));
  });

  it("persists the agent used on send so the next chat defaults to it", async () => {
    vi.mocked(chat.listAgents).mockResolvedValue(twoAgents);
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "turnDone" });
      return null;
    });
    render(<AssistantConversation />);
    fireEvent.click(await screen.findByRole("combobox", { name: /agent/i }));
    fireEvent.click(await screen.findByRole("option", { name: /codex/i }));
    fireEvent.change(screen.getByPlaceholderText(/ask/i), { target: { value: "hi" } });
    fireEvent.click(screen.getByRole("button", { name: /^send$/i }));
    await waitFor(() => expect(chat.sendChat).toHaveBeenCalled());
    expect(localStorage.getItem("srelens.assistant.lastAgent")).toBe("codex");
  });

  it("falls back to the first available when the last-used agent is no longer installed", async () => {
    localStorage.setItem("srelens.assistant.lastAgent", "codex");
    // Only Claude is available now — the stored codex is gone.
    vi.mocked(chat.listAgents).mockResolvedValue([twoAgents[0]]);
    render(<AssistantConversation />);
    const trigger = await screen.findByRole("combobox", { name: /agent/i });
    await waitFor(() => expect(trigger.textContent).toMatch(/claude/i));
  });

  it("restores the agent used when a session is reopened", async () => {
    vi.mocked(chat.listAgents).mockResolvedValue(twoAgents);
    vi.mocked(chatHistory.listSessions).mockResolvedValue([{ id: "s-old", title: "Old", createdAt: 1, updatedAt: 2 }]);
    vi.mocked(chatHistory.loadSession).mockResolvedValue({
      id: "s-old",
      title: "Old",
      createdAt: 1,
      updatedAt: 2,
      contexts: [],
      skills: [],
      cliSessionId: null,
      agentKind: "codex",
      messages: [],
    });
    render(<AssistantConversation />);
    const trigger = await screen.findByRole("combobox", { name: /agent/i });
    // Default lands on Claude; reopening the Codex session restores Codex.
    await waitFor(() => expect(trigger.textContent).toMatch(/claude/i));
    await openHistory();
    fireEvent.click(await screen.findByText("Old"));
    await waitFor(() => expect(trigger.textContent).toMatch(/codex/i));
  });
});
