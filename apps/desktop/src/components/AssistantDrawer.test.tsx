import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AssistantDrawer } from "./AssistantDrawer";
import * as chat from "../lib/chat";

const respondToConfirm = vi.fn();
let emitConfirm: (payload: unknown) => void = () => {};

vi.mock("../lib/chat");
vi.mock("../lib/mcpSecurity", () => ({
  respondToConfirm: (...a: unknown[]) => respondToConfirm(...a),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (_name: string, cb: (e: { payload: unknown }) => void) => {
    emitConfirm = (payload) => cb({ payload });
    return Promise.resolve(() => {});
  },
}));

// This repo doesn't pull in @testing-library/jest-dom, so assert directly on
// DOM presence (`getByText` throws if not found) instead of `toBeInTheDocument`.

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
  respondToConfirm.mockReset();
});

describe("AssistantDrawer", () => {
  it("streams a reply and renders a tool-call card", async () => {
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "toolCallStart", id: "t1", tool: "k8s.listPods", args: {} });
      onEvent({ type: "toolResult", id: "t1", status: "ok" });
      onEvent({ type: "textDelta", text: "3 pods running." });
      onEvent({ type: "turnDone" });
    });
    render(<AssistantDrawer open onClose={() => {}} />);
    fireEvent.change(await screen.findByPlaceholderText(/ask/i), { target: { value: "why?" } });
    fireEvent.click(screen.getByRole("button", { name: /send/i }));
    await waitFor(() => expect(screen.getByText(/3 pods running/)).toBeTruthy());
    expect(screen.getByText("k8s.listPods")).toBeTruthy();
  });

  it("starts a new paragraph for a text segment that follows a tool call, instead of running it together", async () => {
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "textDelta", text: "A." });
      onEvent({ type: "toolCallStart", id: "t1", tool: "k8s.listPods", args: {} });
      onEvent({ type: "toolResult", id: "t1", status: "ok" });
      onEvent({ type: "textDelta", text: "B." });
      onEvent({ type: "turnDone" });
    });
    render(<AssistantDrawer open onClose={() => {}} />);
    fireEvent.change(await screen.findByPlaceholderText(/ask/i), { target: { value: "why?" } });
    fireEvent.click(screen.getByRole("button", { name: /send/i }));
    const first = await screen.findByText("A.");
    const second = await screen.findByText("B.");
    // Two separate paragraph elements, not one node containing "A.B.".
    expect(first).not.toBe(second);
    expect(first.closest("p")).toBeTruthy();
    expect(second.closest("p")).toBeTruthy();
    expect(first.closest("p")).not.toBe(second.closest("p"));
    expect(screen.queryByText("A.B.")).toBeFalsy();
  });

  it("does not add an extra paragraph break when the delta itself already starts with whitespace", async () => {
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "textDelta", text: "A." });
      onEvent({ type: "toolCallStart", id: "t1", tool: "k8s.listPods", args: {} });
      onEvent({ type: "toolResult", id: "t1", status: "ok" });
      // Leading "\n" already separates this from "A." — inserting another
      // "\n\n" on top would produce a blank line and split this into two
      // paragraphs instead of one continued line.
      onEvent({ type: "textDelta", text: "\nB." });
      onEvent({ type: "turnDone" });
    });
    render(<AssistantDrawer open onClose={() => {}} />);
    fireEvent.change(await screen.findByPlaceholderText(/ask/i), { target: { value: "why?" } });
    fireEvent.click(screen.getByRole("button", { name: /send/i }));
    const merged = await screen.findByText("A. B.");
    expect(merged.closest("p")).toBeTruthy();
  });

  it("keeps consecutive textDeltas with no tool event between them in one paragraph", async () => {
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "textDelta", text: "Hello " });
      onEvent({ type: "textDelta", text: "world" });
      onEvent({ type: "turnDone" });
    });
    render(<AssistantDrawer open onClose={() => {}} />);
    fireEvent.change(await screen.findByPlaceholderText(/ask/i), { target: { value: "why?" } });
    fireEvent.click(screen.getByRole("button", { name: /send/i }));
    const merged = await screen.findByText("Hello world");
    expect(merged.closest("p")).toBeTruthy();
  });

  it("shows a context chip and removes it on click", async () => {
    render(
      <AssistantDrawer
        open
        onClose={() => {}}
        context={{ context: "kind", namespace: "payments", kind: "Deployment", name: "api" }}
      />,
    );
    expect(await screen.findByText("kind / payments / Deployment api")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /remove/i }));
    expect(screen.queryByText("kind / payments / Deployment api")).toBeFalsy();
  });

  it("disables Send and shows the install link when no agent is available", async () => {
    vi.mocked(chat.listAgents).mockResolvedValue([
      {
        kind: "claude",
        label: "Claude Code",
        available: false,
        path: null,
        version: null,
        installUrl: "https://example.com/install-claude",
        gated: false,
      },
    ]);
    render(<AssistantDrawer open onClose={() => {}} />);
    await screen.findByText(/example\.com\/install-claude/);
    fireEvent.change(screen.getByPlaceholderText(/ask/i), { target: { value: "why?" } });
    expect((screen.getByRole("button", { name: /send/i }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("defaults to the first available agent, and disables Send when the user switches to an unavailable one", async () => {
    vi.mocked(chat.listAgents).mockResolvedValue([
      {
        kind: "codex",
        label: "Codex",
        available: false,
        path: null,
        version: null,
        installUrl: "https://example.com/install-codex",
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
    render(<AssistantDrawer open onClose={() => {}} />);
    fireEvent.change(await screen.findByPlaceholderText(/ask/i), { target: { value: "why?" } });
    // The mock lists the unavailable agent first — the default selection must
    // still land on the available one, proving "first available" wins over
    // list order.
    await waitFor(() =>
      expect((screen.getByRole("button", { name: /send/i }) as HTMLButtonElement).disabled).toBe(false),
    );

    fireEvent.change(screen.getByRole("combobox", { name: /agent/i }), { target: { value: "codex" } });
    await waitFor(() =>
      expect((screen.getByRole("button", { name: /send/i }) as HTMLButtonElement).disabled).toBe(true),
    );
    expect(screen.getByText(/example\.com\/install-codex/)).toBeTruthy();
  });

  it("keeps a tool call's args collapsed until the disclosure toggle is clicked", async () => {
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "toolCallStart", id: "t1", tool: "k8s.getSecret", args: { name: "db-creds", namespace: "prod" } });
      onEvent({ type: "toolResult", id: "t1", status: "ok" });
      onEvent({ type: "turnDone" });
    });
    render(<AssistantDrawer open onClose={() => {}} />);
    fireEvent.change(await screen.findByPlaceholderText(/ask/i), { target: { value: "why?" } });
    fireEvent.click(screen.getByRole("button", { name: /send/i }));
    await screen.findByText("k8s.getSecret");
    expect(screen.queryByText(/db-creds/)).toBeFalsy();

    fireEvent.click(screen.getByRole("button", { name: /k8s\.getSecret/i }));
    expect(screen.getByText(/db-creds/)).toBeTruthy();
  });

  it("renders an error AgentEvent as a distinct error bubble", async () => {
    vi.mocked(chat.sendChat).mockImplementation(async (_s, _p, _a, onEvent) => {
      onEvent({ type: "error", message: "the agent crashed" });
      onEvent({ type: "turnDone" });
    });
    render(<AssistantDrawer open onClose={() => {}} />);
    fireEvent.change(await screen.findByPlaceholderText(/ask/i), { target: { value: "why?" } });
    fireEvent.click(screen.getByRole("button", { name: /send/i }));
    const bubble = await screen.findByText("the agent crashed");
    expect(bubble.className).toContain("text-destructive");
  });

  it("catches a rejected sendChat (transport failure) as an error message and re-enables Send", async () => {
    vi.mocked(chat.sendChat).mockRejectedValue(
      new Error("Start the MCP server in Settings → MCP before using the assistant."),
    );
    render(<AssistantDrawer open onClose={() => {}} />);
    fireEvent.change(await screen.findByPlaceholderText(/ask/i), { target: { value: "why?" } });
    fireEvent.click(screen.getByRole("button", { name: /send/i }));
    const bubble = await screen.findByText(/start the mcp server/i);
    expect(bubble.className).toContain("text-destructive");

    // `sending` must have been cleared even though the promise rejected —
    // re-type into the box and confirm Send isn't stuck disabled.
    fireEvent.change(screen.getByPlaceholderText(/ask/i), { target: { value: "again" } });
    expect((screen.getByRole("button", { name: /send/i }) as HTMLButtonElement).disabled).toBe(false);
  });

  it("renders the inline confirm card for a gated call and approves it", async () => {
    render(<AssistantDrawer open onClose={() => {}} />);
    await screen.findByPlaceholderText(/ask/i);

    emitConfirm({ id: "c1", tool: "k8s.deletePod", args: { name: "web-1", namespace: "prod" } });
    await screen.findByText("k8s.deletePod");
    expect(screen.getByRole("button", { name: /approve/i })).toBeTruthy();
    expect(screen.getByRole("button", { name: /deny/i })).toBeTruthy();

    await userEvent.click(screen.getByRole("button", { name: /approve/i }));
    await waitFor(() => expect(respondToConfirm).toHaveBeenCalledWith("c1", true));
    expect(screen.queryByText("k8s.deletePod")).toBeFalsy();
  });
});
