import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { AssistantConversation } from "./AssistantConversation";
import * as chat from "../lib/chat";

vi.mock("../lib/chat");
vi.mock("../lib/mcpSecurity", () => ({
  respondToConfirm: vi.fn(),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: () => Promise.resolve(() => {}),
}));

// This repo doesn't pull in @testing-library/jest-dom, so assert directly on
// DOM presence (`getByText`/`queryByText` throws-or-null) instead of
// `toBeInTheDocument`.

beforeEach(() => {
  vi.mocked(chat.listAgents).mockResolvedValue([
    { kind: "claude", label: "Claude Code", available: true, path: "/usr/bin/claude", version: null, installUrl: "" },
  ]);
  vi.mocked(chat.startChat).mockResolvedValue("s1");
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
});
