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

  it("disables Send for an unavailable agent until an available one is picked", async () => {
    vi.mocked(chat.listAgents).mockResolvedValue([
      {
        kind: "codex",
        label: "Codex",
        available: false,
        path: null,
        version: null,
        installUrl: "https://example.com/install-codex",
      },
      { kind: "claude", label: "Claude Code", available: true, path: "/usr/bin/claude", version: null, installUrl: "" },
    ]);
    render(<AssistantDrawer open onClose={() => {}} />);
    await screen.findByText(/example\.com\/install-codex/);
    fireEvent.change(screen.getByPlaceholderText(/ask/i), { target: { value: "why?" } });
    expect((screen.getByRole("button", { name: /send/i }) as HTMLButtonElement).disabled).toBe(true);

    fireEvent.change(screen.getByRole("combobox", { name: /agent/i }), { target: { value: "claude" } });
    await waitFor(() =>
      expect((screen.getByRole("button", { name: /send/i }) as HTMLButtonElement).disabled).toBe(false),
    );
  });
});
