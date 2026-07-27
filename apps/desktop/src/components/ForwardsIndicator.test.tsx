import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import React from "react";

const { invokeCommandMock, onMock } = vi.hoisted(() => ({
  invokeCommandMock: vi.fn(),
  onMock: vi.fn(() => () => {}),
}));
vi.mock("../transport/transport", () => ({ invokeCommand: invokeCommandMock, on: onMock }));

import { ForwardsIndicator } from "./ForwardsIndicator";
import { startPortForward, stopPortForward, getForwards } from "../lib/forward";

beforeEach(async () => {
  // Reset the module-level store between tests.
  for (const f of [...getForwards()]) {
    invokeCommandMock.mockResolvedValueOnce(undefined);
    await stopPortForward(f.id);
  }
  invokeCommandMock.mockReset();
  onMock.mockClear();
});

describe("ForwardsIndicator", () => {
  it("renders nothing when there are no forwards", () => {
    const { container } = render(<ForwardsIndicator />);
    expect(container.firstChild).toBeNull();
  });

  it("shows the count and lists/stops a forward (desktop shows localhost address)", async () => {
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    try {
      invokeCommandMock.mockResolvedValueOnce({ id: 1, localPort: 5000 });
      await act(async () => {
        await startPortForward({
          context: "kind-dev",
          namespace: "default",
          kind: "Pod",
          name: "web-1",
          remotePort: 80,
        });
      });

      render(<ForwardsIndicator />);
      const trigger = screen.getByRole("button", { name: /active port forwards/ });
      expect(trigger.textContent).toContain("1");

      await userEvent.click(trigger);
      expect(await screen.findByText("web-1")).toBeDefined();
      expect(screen.getByText("localhost:5000 → 80")).toBeDefined();

      invokeCommandMock.mockResolvedValueOnce(undefined);
      fireEvent.click(screen.getByRole("button", { name: "Stop" }));
      await waitFor(() => expect(invokeCommandMock).toHaveBeenCalledWith("stop_port_forward", { id: 1 }));
    } finally {
      delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
    }
  });

  it("shows the proxied /pf address in web mode", async () => {
    invokeCommandMock.mockResolvedValueOnce({ id: 2, localPort: 5001 });
    await act(async () => {
      await startPortForward({
        context: "kind-dev",
        namespace: "default",
        kind: "Pod",
        name: "web-1",
        remotePort: 80,
      });
    });

    render(<ForwardsIndicator />);
    await userEvent.click(screen.getByRole("button", { name: /active port forwards/ }));
    expect(await screen.findByText(`${window.location.origin}/pf/2/ → 80`)).toBeDefined();
  });
});
