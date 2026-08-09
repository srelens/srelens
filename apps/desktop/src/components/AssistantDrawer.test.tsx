import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { AssistantDrawer } from "./AssistantDrawer";
import * as chat from "../lib/chat";

vi.mock("../lib/chat");

// This repo doesn't pull in @testing-library/jest-dom, so assert directly on
// DOM presence (`getByText` throws if not found) instead of `toBeInTheDocument`.

beforeEach(() => {
  vi.mocked(chat.listAgents).mockResolvedValue([
    { kind: "claude", label: "Claude Code", available: true, path: "/usr/bin/claude", version: null, installUrl: "" },
  ]);
  vi.mocked(chat.startChat).mockResolvedValue("s1");
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
});
