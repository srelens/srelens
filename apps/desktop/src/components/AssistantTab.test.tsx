import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { AssistantTab } from "./AssistantTab";
import * as chat from "../lib/chat";
import * as chatHistory from "../lib/chatHistory";
import * as skills from "../lib/skills";

// This repo doesn't pull in @testing-library/jest-dom, so assert directly on
// DOM presence (`getByText`/`queryByText` throws-or-null) instead of
// `toBeInTheDocument`.

vi.mock("../lib/chat");
vi.mock("../lib/chatHistory");
vi.mock("../lib/skills");
// Stub the Skills panel down to a close affordance so a test can drive its
// close callback without standing up the whole editor dialog.
vi.mock("./SkillsPanel", () => ({
  SkillsPanel: ({ onClose }: { onClose: () => void }) => (
    <button onClick={onClose}>close-skills-panel</button>
  ),
}));
vi.mock("../lib/mcpSecurity", () => ({
  respondToConfirm: vi.fn(),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: () => Promise.resolve(() => {}),
}));

beforeEach(() => {
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
  vi.mocked(chat.startChat).mockResolvedValue("s1");
  vi.mocked(chatHistory.listSessions).mockResolvedValue([]);
  vi.mocked(chatHistory.loadSession).mockRejectedValue(new Error("not stubbed"));
  vi.mocked(chatHistory.saveSession).mockResolvedValue(undefined);
  vi.mocked(chatHistory.deleteSession).mockResolvedValue(undefined);
  vi.mocked(skills.listSkills).mockResolvedValue([]);
});

describe("AssistantTab", () => {
  it("renders the left rail (New Chat + history) alongside the conversation, with no raw session list in the main pane", async () => {
    vi.mocked(chatHistory.listSessions).mockResolvedValue([
      { id: "s-old", title: "Old chat", createdAt: 1, updatedAt: 2 },
    ]);
    render(<AssistantTab cluster={null} availableContexts={["a", "b"]} />);

    expect(await screen.findByRole("button", { name: /^new chat$/i })).toBeTruthy();
    expect(await screen.findByText("Old chat")).toBeTruthy();

    // The rail replaces the conversation's own compact popover entirely —
    // no "History" trigger duplicating the same list in the main pane.
    expect(screen.queryByRole("button", { name: /^history$/i })).toBeFalsy();
  });

  it("clicking a rail item loads that session, replaying its messages", async () => {
    vi.mocked(chatHistory.listSessions).mockResolvedValue([
      { id: "s-old", title: "Old chat", createdAt: 1, updatedAt: 2 },
    ]);
    vi.mocked(chatHistory.loadSession).mockResolvedValue({
      id: "s-old",
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
    render(<AssistantTab cluster={null} availableContexts={["a", "b"]} />);

    fireEvent.click(await screen.findByText("Old chat"));

    expect(await screen.findByText("what pods are crashing?")).toBeTruthy();
    expect(await screen.findByText(/pod-a is crashlooping\./)).toBeTruthy();
    expect(chatHistory.loadSession).toHaveBeenCalledWith("s-old");
  });

  it("hover-delete on a rail item removes it from disk and from the rail", async () => {
    vi.mocked(chatHistory.listSessions).mockResolvedValue([
      { id: "s-old", title: "Old chat", createdAt: 1, updatedAt: 2 },
    ]);
    render(<AssistantTab cluster={null} availableContexts={["a", "b"]} />);
    await screen.findByText("Old chat");

    fireEvent.click(screen.getByLabelText("Delete Old chat"));

    await waitFor(() => expect(chatHistory.deleteSession).toHaveBeenCalledWith("s-old"));
    // The rail mirrors `AssistantConversation`'s session state one effect-tick
    // behind (`onSessionsChanged`), so give that extra render a moment too.
    await waitFor(() => expect(screen.queryByText("Old chat")).toBeFalsy());
  });

  it("clicking New Chat in the rail clears the transcript back to the empty state", async () => {
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "textDelta", text: "reply" });
      onEvent({ type: "turnDone" });
      return null;
    });
    render(<AssistantTab cluster={null} availableContexts={["a", "b"]} />);

    fireEvent.click(await screen.findByRole("button", { name: /contexts \(0\)/i }));
    fireEvent.click(await screen.findByText("a"));
    fireEvent.change(await screen.findByPlaceholderText(/ask/i), { target: { value: "hi" } });
    fireEvent.click(screen.getByRole("button", { name: /^send$/i }));
    await screen.findByText("reply");

    fireEvent.click(screen.getByRole("button", { name: /^new chat$/i }));
    expect(await screen.findByText(/ask about this cluster to get started/i)).toBeTruthy();
  });

  it("the composer shows the inline agent picker, attach control, and global context control", async () => {
    render(<AssistantTab cluster={null} availableContexts={["a", "b"]} />);
    const composer = (await screen.findByTestId("assistant-composer")) as HTMLElement;
    const scoped = within(composer);
    expect(scoped.getByRole("combobox", { name: /agent/i })).toBeTruthy();
    expect(scoped.getByLabelText(/attach image/i)).toBeTruthy();
    expect(scoped.getByRole("button", { name: /contexts \(0\)/i })).toBeTruthy();
  });

  it("collapses and expands the rail", async () => {
    vi.mocked(chatHistory.listSessions).mockResolvedValue([
      { id: "s-old", title: "Old chat", createdAt: 1, updatedAt: 2 },
    ]);
    render(<AssistantTab cluster={null} availableContexts={["a", "b"]} />);
    await screen.findByText("Old chat");

    fireEvent.click(screen.getByLabelText(/collapse history/i));
    expect(screen.queryByText("Old chat")).toBeFalsy();
    expect(screen.queryByRole("button", { name: /^new chat$/i })).toBeFalsy();

    fireEvent.click(screen.getByLabelText(/expand history/i));
    expect(await screen.findByText("Old chat")).toBeTruthy();
  });

  it("reloads the slash-menu skills after the Skills panel closes", async () => {
    render(<AssistantTab cluster={null} availableContexts={["a", "b"]} />);
    // Let the initial mount/settle finish, then take the load count as baseline.
    await screen.findByRole("button", { name: /^skills$/i });
    const before = vi.mocked(skills.listSkills).mock.calls.length;

    // Open the Skills panel from the rail, then close it — the panel may have
    // created/renamed/deleted a skill, so the slash menu must reload.
    fireEvent.click(screen.getByRole("button", { name: /^skills$/i }));
    fireEvent.click(screen.getByText("close-skills-panel"));

    await waitFor(() =>
      expect(vi.mocked(skills.listSkills).mock.calls.length).toBeGreaterThan(before),
    );
  });
});
