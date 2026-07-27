import { afterEach, describe, it, expect, vi, beforeEach } from "vitest";

const { invokeCommandMock, onMock } = vi.hoisted(() => ({
  invokeCommandMock: vi.fn(),
  onMock: vi.fn(),
}));
vi.mock("../transport/transport", () => ({
  invokeCommand: invokeCommandMock,
  on: onMock,
}));

import {
  startPortForward,
  stopPortForward,
  getForwards,
  subscribeForwards,
  forwardUrl,
  forwardAddress,
  type ActiveForward,
} from "./forward";

// Capture the `forward:closed:<id>` handlers so tests can fire the event.
const closedHandlers = new Map<string, () => void>();

beforeEach(async () => {
  invokeCommandMock.mockReset();
  onMock.mockReset();
  closedHandlers.clear();
  onMock.mockImplementation((channel: string, handler: () => void) => {
    closedHandlers.set(channel, handler);
    return () => closedHandlers.delete(channel);
  });
  // Drain any forwards left over from a previous test.
  for (const f of [...getForwards()]) {
    invokeCommandMock.mockResolvedValueOnce(undefined);
    await stopPortForward(f.id);
  }
  invokeCommandMock.mockReset();
});

const req = {
  context: "kind-dev",
  namespace: "default",
  kind: "Pod",
  name: "web-1",
  remotePort: 80,
};

describe("forward store", () => {
  it("adds a forward and notifies subscribers", async () => {
    const notify = vi.fn();
    const unsub = subscribeForwards(notify);
    invokeCommandMock.mockResolvedValueOnce({ id: 1, localPort: 54321 });

    const fwd = await startPortForward(req);

    expect(fwd).toMatchObject({ id: 1, localPort: 54321, name: "web-1", remotePort: 80 });
    expect(getForwards()).toHaveLength(1);
    expect(notify).toHaveBeenCalled();
    expect(invokeCommandMock).toHaveBeenCalledWith("start_port_forward", {
      context: "kind-dev",
      namespace: "default",
      kind: "Pod",
      name: "web-1",
      remotePort: 80,
      localPort: null,
    });
    unsub();
  });

  it("stops a forward and removes it from the store", async () => {
    invokeCommandMock.mockResolvedValueOnce({ id: 2, localPort: 5000 });
    await startPortForward(req);
    expect(getForwards()).toHaveLength(1);

    invokeCommandMock.mockResolvedValueOnce(undefined);
    await stopPortForward(2);

    expect(getForwards()).toHaveLength(0);
    expect(invokeCommandMock).toHaveBeenLastCalledWith("stop_port_forward", { id: 2 });
  });

  it("auto-removes a forward when the backend emits forward:closed", async () => {
    invokeCommandMock.mockResolvedValueOnce({ id: 3, localPort: 5001 });
    await startPortForward(req);
    expect(getForwards()).toHaveLength(1);

    // Simulate the backend serve-loop ending.
    closedHandlers.get("forward:closed:3")?.();

    expect(getForwards()).toHaveLength(0);
  });

  it("passes a preferred local port through", async () => {
    invokeCommandMock.mockResolvedValueOnce({ id: 4, localPort: 8080 });
    const withLocal: ActiveForward = { ...req, id: 0, localPort: 0 };
    await startPortForward({ ...withLocal, localPort: 8080 });
    expect(invokeCommandMock).toHaveBeenCalledWith(
      "start_port_forward",
      expect.objectContaining({ localPort: 8080 }),
    );
  });
});

describe("forwardUrl", () => {
  afterEach(() => {
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  });
  it("uses localhost:port on desktop", () => {
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    expect(forwardUrl({ id: 3, localPort: 5000 })).toBe("http://localhost:5000");
  });
  it("uses the /pf proxy path on web", () => {
    expect(forwardUrl({ id: 3, localPort: 5000 })).toBe("/pf/3/");
  });
});

describe("forwardAddress", () => {
  afterEach(() => {
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  });
  it("uses localhost:port on desktop", () => {
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    expect(forwardAddress({ id: 3, localPort: 5000 })).toBe("localhost:5000");
  });
  it("uses the absolute /pf proxy URL on web", () => {
    expect(forwardAddress({ id: 3, localPort: 5000 })).toBe(`${window.location.origin}/pf/3/`);
  });
});
