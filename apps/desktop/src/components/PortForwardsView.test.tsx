import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor, act } from "@testing-library/react";
import React from "react";

const { invokeCommandMock, onMock } = vi.hoisted(() => ({
  invokeCommandMock: vi.fn(),
  onMock: vi.fn(() => () => {}),
}));
vi.mock("../transport/transport", () => ({ invokeCommand: invokeCommandMock, on: onMock }));

import { PortForwardsView } from "./PortForwardsView";
import { startPortForward, stopPortForward, getForwards } from "../lib/forward";

beforeEach(async () => {
  for (const f of [...getForwards()]) {
    invokeCommandMock.mockResolvedValueOnce(undefined);
    await stopPortForward(f.id);
  }
  invokeCommandMock.mockReset();
  onMock.mockClear();
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
});
