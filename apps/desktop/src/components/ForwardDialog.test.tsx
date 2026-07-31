import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import React from "react";

const { startPortForwardMock, saveForwardMock } = vi.hoisted(() => ({
  startPortForwardMock: vi.fn(),
  saveForwardMock: vi.fn(),
}));
vi.mock("../lib/forward", () => ({ startPortForward: startPortForwardMock }));
vi.mock("../lib/savedForwards", () => ({ saveForward: saveForwardMock }));

import { ForwardDialog } from "./ForwardDialog";

beforeEach(() => {
  startPortForwardMock.mockReset();
  saveForwardMock.mockReset();
});

const base = { context: "kind-dev", namespace: "default", kind: "Pod", name: "web-1" };

describe("ForwardDialog", () => {
  it("starts a forward with the entered remote port", async () => {
    startPortForwardMock.mockResolvedValue({ id: 1, localPort: 5000 });
    const onClose = vi.fn();
    render(<ForwardDialog {...base} onClose={onClose} />);

    fireEvent.change(screen.getByLabelText("Remote port"), { target: { value: "8080" } });
    fireEvent.click(screen.getByRole("button", { name: "Forward" }));

    await waitFor(() =>
      expect(startPortForwardMock).toHaveBeenCalledWith({
        context: "kind-dev",
        namespace: "default",
        kind: "Pod",
        name: "web-1",
        remotePort: 8080,
        localPort: undefined,
      }),
    );
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });

  it("passes an explicit local port when provided", async () => {
    startPortForwardMock.mockResolvedValue({ id: 1, localPort: 3000 });
    render(<ForwardDialog {...base} defaultRemotePort={80} onClose={vi.fn()} />);

    fireEvent.change(screen.getByLabelText("Local port"), { target: { value: "3000" } });
    fireEvent.click(screen.getByRole("button", { name: "Forward" }));

    await waitFor(() =>
      expect(startPortForwardMock).toHaveBeenCalledWith(
        expect.objectContaining({ remotePort: 80, localPort: 3000 }),
      ),
    );
  });

  it("rejects an out-of-range port without calling the backend", () => {
    render(<ForwardDialog {...base} onClose={vi.fn()} />);
    fireEvent.change(screen.getByLabelText("Remote port"), { target: { value: "99999" } });
    fireEvent.click(screen.getByRole("button", { name: "Forward" }));

    expect(screen.getByText(/between 1 and 65535/)).toBeDefined();
    expect(startPortForwardMock).not.toHaveBeenCalled();
  });

  it("surfaces the backend's suggested port on a conflict and retries on it", async () => {
    startPortForwardMock.mockRejectedValueOnce(
      new Error("port 8080 is already in use; 54321 is free"),
    );
    startPortForwardMock.mockResolvedValueOnce({ id: 1, localPort: 54321 });
    const onClose = vi.fn();
    render(<ForwardDialog {...base} defaultRemotePort={80} onClose={onClose} />);

    fireEvent.click(screen.getByRole("button", { name: "Forward" }));
    await waitFor(() => expect(startPortForwardMock).toHaveBeenCalledTimes(1));
    expect(await screen.findByText(/54321 is free/)).toBeDefined();

    fireEvent.click(screen.getByRole("button", { name: "Use port 54321" }));
    await waitFor(() =>
      expect(startPortForwardMock).toHaveBeenLastCalledWith(
        expect.objectContaining({ remotePort: 80, localPort: 54321 }),
      ),
    );
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });

  it("does not show a suggestion for a non-conflict error", async () => {
    startPortForwardMock.mockRejectedValueOnce(new Error("cluster unreachable"));
    render(<ForwardDialog {...base} defaultRemotePort={80} onClose={vi.fn()} />);

    fireEvent.click(screen.getByRole("button", { name: "Forward" }));
    expect(await screen.findByText(/cluster unreachable/)).toBeDefined();
    expect(screen.queryByRole("button", { name: /Use port/ })).toBeNull();
  });

  it("saves the current target as a shortcut without starting it", async () => {
    saveForwardMock.mockResolvedValueOnce(undefined);
    render(<ForwardDialog {...base} defaultRemotePort={80} onClose={vi.fn()} />);

    fireEvent.click(screen.getByRole("button", { name: "Save this forward" }));

    await waitFor(() =>
      expect(saveForwardMock).toHaveBeenCalledWith(
        "kind-dev",
        expect.objectContaining({
          namespace: "default",
          kind: "Pod",
          target: "web-1",
          remotePort: 80,
          localPort: undefined,
        }),
      ),
    );
    expect(startPortForwardMock).not.toHaveBeenCalled();
    expect(await screen.findByRole("button", { name: "Saved" })).toBeDefined();
  });
});
