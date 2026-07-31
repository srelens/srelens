import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor, act } from "@testing-library/react";
import React from "react";

const { invokeCommandMock, onMock } = vi.hoisted(() => ({
  invokeCommandMock: vi.fn(),
  onMock: vi.fn((_channel: string, _handler: (payload?: unknown) => void) => () => {}),
}));
vi.mock("../transport/transport", () => ({ invokeCommand: invokeCommandMock, on: onMock }));

const { listSavedForwardsMock, saveForwardMock, deleteSavedForwardMock } = vi.hoisted(() => ({
  listSavedForwardsMock: vi.fn(),
  saveForwardMock: vi.fn(),
  deleteSavedForwardMock: vi.fn(),
}));
vi.mock("../lib/savedForwards", () => ({
  listSavedForwards: listSavedForwardsMock,
  saveForward: saveForwardMock,
  deleteSavedForward: deleteSavedForwardMock,
}));

import { PortForwardsView } from "./PortForwardsView";
import { startPortForward, stopPortForward, getForwards } from "../lib/forward";

// Capture `forward:status:<id>` handlers so tests can fire status events.
const statusHandlers = new Map<string, (payload: unknown) => void>();

beforeEach(async () => {
  for (const f of [...getForwards()]) {
    invokeCommandMock.mockResolvedValueOnce(undefined);
    await stopPortForward(f.id);
  }
  invokeCommandMock.mockReset();
  onMock.mockClear();
  statusHandlers.clear();
  onMock.mockImplementation((channel: string, handler: (payload?: unknown) => void) => {
    if (channel.startsWith("forward:status:")) statusHandlers.set(channel, handler);
    return () => statusHandlers.delete(channel);
  });
  listSavedForwardsMock.mockReset().mockResolvedValue([]);
  saveForwardMock.mockReset();
  deleteSavedForwardMock.mockReset().mockResolvedValue(undefined);
});

describe("PortForwardsView", () => {
  it("shows an empty state when there are no forwards", () => {
    render(<PortForwardsView context="kind-dev" />);
    expect(screen.getByText(/No active port forwards/)).toBeDefined();
  });

  it("lists active forwards and can stop one (desktop shows localhost address)", async () => {
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    try {
      invokeCommandMock.mockResolvedValueOnce({ id: 1, localPort: 5000 });
      await act(async () => {
        await startPortForward({
          context: "kind-dev",
          namespace: "default",
          kind: "Service",
          name: "web",
          remotePort: 80,
        });
      });

      render(<PortForwardsView context="kind-dev" />);
      expect(screen.getByText("web")).toBeDefined();
      expect(screen.getByText("Service")).toBeDefined();
      expect(screen.getByText("localhost:5000")).toBeDefined();

      invokeCommandMock.mockResolvedValueOnce(undefined);
      fireEvent.click(screen.getByRole("button", { name: "Stop" }));
      await waitFor(() =>
        expect(invokeCommandMock).toHaveBeenCalledWith("stop_port_forward", { id: 1 }),
      );
      await waitFor(() => expect(screen.getByText(/No active port forwards/)).toBeDefined());
    } finally {
      delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
    }
  });

  it("shows the proxied /pf URL in web mode", async () => {
    invokeCommandMock.mockResolvedValueOnce({ id: 2, localPort: 5001 });
    await act(async () => {
      await startPortForward({
        context: "kind-dev",
        namespace: "default",
        kind: "Service",
        name: "web",
        remotePort: 80,
      });
    });

    render(<PortForwardsView context="kind-dev" />);
    expect(screen.getByText(`${window.location.origin}/pf/2/`)).toBeDefined();
  });

  it("shows a reconnecting/failed active forward's status", async () => {
    invokeCommandMock.mockResolvedValueOnce({ id: 7, localPort: 5010 });
    await act(async () => {
      await startPortForward({
        context: "kind-dev",
        namespace: "default",
        kind: "Service",
        name: "web",
        remotePort: 80,
      });
    });

    render(<PortForwardsView context="kind-dev" />);
    expect(screen.getByText("Active")).toBeDefined();

    act(() => {
      statusHandlers.get("forward:status:7")?.({ state: "reconnecting" });
    });
    expect(await screen.findByText("Reconnecting")).toBeDefined();
  });

  describe("saved forwards", () => {
    const sf = {
      id: "sf-1",
      name: "web console",
      namespace: "default",
      kind: "Service",
      target: "web",
      remotePort: 80,
      localPort: 8080,
    };

    it("lists saved forwards for the context", async () => {
      listSavedForwardsMock.mockResolvedValue([sf]);
      render(<PortForwardsView context="kind-dev" />);

      expect(await screen.findByText("web console")).toBeDefined();
      expect(screen.getByText("web")).toBeDefined();
      expect(listSavedForwardsMock).toHaveBeenCalledWith("kind-dev");
    });

    it("does not list saved forwards without a context", () => {
      render(<PortForwardsView />);
      expect(listSavedForwardsMock).not.toHaveBeenCalled();
    });

    it("Start on a saved row calls startPortForward with the saved target", async () => {
      listSavedForwardsMock.mockResolvedValue([sf]);
      invokeCommandMock.mockResolvedValueOnce({ id: 8, localPort: 8080 });
      render(<PortForwardsView context="kind-dev" />);

      fireEvent.click(await screen.findByRole("button", { name: "Start" }));

      await waitFor(() =>
        expect(invokeCommandMock).toHaveBeenCalledWith("start_port_forward", {
          context: "kind-dev",
          namespace: "default",
          kind: "Service",
          name: "web",
          remotePort: 80,
          localPort: 8080,
        }),
      );
    });

    it("surfaces an error when starting a saved forward fails (not silent)", async () => {
      listSavedForwardsMock.mockResolvedValue([sf]);
      invokeCommandMock.mockRejectedValueOnce(
        new Error("port 8080 is already in use; 8081 is free"),
      );
      render(<PortForwardsView context="kind-dev" />);

      fireEvent.click(await screen.findByRole("button", { name: "Start" }));

      const alert = await screen.findByRole("alert");
      expect(alert.textContent).toContain("web console");
      expect(alert.textContent).toContain("8080 is already in use");
    });

    it("Delete removes a saved row via deleteSavedForward", async () => {
      listSavedForwardsMock.mockResolvedValueOnce([sf]).mockResolvedValueOnce([]);
      render(<PortForwardsView context="kind-dev" />);

      fireEvent.click(await screen.findByRole("button", { name: "Delete" }));

      await waitFor(() => expect(deleteSavedForwardMock).toHaveBeenCalledWith("kind-dev", "sf-1"));
      await waitFor(() => expect(screen.queryByText("web console")).toBeNull());
    });
  });
});
