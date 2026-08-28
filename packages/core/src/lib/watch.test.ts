import { describe, it, expect, vi, beforeEach } from "vitest";

const { invokeCommandMock, subscribeMock } = vi.hoisted(() => ({
  invokeCommandMock: vi.fn(),
  subscribeMock: vi.fn(),
}));
vi.mock("../transport/transport", () => ({
  invokeCommand: invokeCommandMock,
  subscribe: subscribeMock,
}));

import { watchResource } from "./watch";

beforeEach(() => {
  invokeCommandMock.mockReset();
  subscribeMock.mockReset();
});

describe("watchResource", () => {
  it("subscribes first, then starts the watch on the same channel", async () => {
    let captured: ((payload: unknown) => void) | undefined;
    let subscribedChannel = "";
    const dispose = vi.fn();
    subscribeMock.mockImplementation(async (ch: string, handler: (p: unknown) => void) => {
      subscribedChannel = ch;
      captured = handler;
      return dispose;
    });
    invokeCommandMock.mockResolvedValue(undefined);
    const onRows = vi.fn();
    const onStatus = vi.fn();

    const handle = await watchResource("kind-dev", "default", "deployments", onRows, onStatus);

    // Subscribed before invoking the backend watch.
    expect(subscribeMock.mock.invocationCallOrder[0]).toBeLessThan(
      invokeCommandMock.mock.invocationCallOrder[0],
    );
    expect(subscribedChannel).toMatch(/^watch:deployments:kind-dev:default:/);
    // Backend watch started on the SAME channel the listener is on.
    expect(invokeCommandMock).toHaveBeenCalledWith("start_resource_watch", {
      context: "kind-dev",
      namespace: "default",
      kind: "deployments",
      channel: subscribedChannel,
      kubeconfigPaths: [],
    });

    // Array payloads are snapshots; `{status}` objects drive the status callback.
    captured?.([{ name: "web" }]);
    expect(onRows).toHaveBeenCalledWith([{ name: "web" }]);
    captured?.({ status: "reconnecting" });
    expect(onStatus).toHaveBeenCalledWith("reconnecting");
    expect(onRows).toHaveBeenCalledTimes(1); // status didn't count as a snapshot

    handle.stop();
    expect(dispose).toHaveBeenCalled();
    expect(invokeCommandMock).toHaveBeenCalledWith("stop_watch", { channel: subscribedChannel });
  });

  it("passes the kubeconfig files to the backend as kubeconfigPaths", async () => {
    let subscribedChannel = "";
    subscribeMock.mockImplementation(async (ch: string) => {
      subscribedChannel = ch;
      return vi.fn();
    });
    invokeCommandMock.mockResolvedValue(undefined);

    await watchResource("kind-dev", "default", "pods", vi.fn(), undefined, undefined, [
      "/some/pasted.yaml",
    ]);

    expect(invokeCommandMock).toHaveBeenCalledWith("start_resource_watch", {
      context: "kind-dev",
      namespace: "default",
      kind: "pods",
      channel: subscribedChannel,
      kubeconfigPaths: ["/some/pasted.yaml"],
    });
  });

  it("routes an { error } payload to onError (permanent watch failure)", async () => {
    let captured: ((payload: unknown) => void) | undefined;
    subscribeMock.mockImplementation(async (_ch: string, handler: (p: unknown) => void) => {
      captured = handler;
      return vi.fn();
    });
    invokeCommandMock.mockResolvedValue(undefined);
    const onRows = vi.fn();
    const onStatus = vi.fn();
    const onError = vi.fn();

    await watchResource("kind-dev", "", "pods", onRows, onStatus, onError);

    captured?.({ error: "watch pods is forbidden: User cannot watch" });
    expect(onError).toHaveBeenCalledWith("watch pods is forbidden: User cannot watch");
    // An error is neither a snapshot nor a status transition.
    expect(onRows).not.toHaveBeenCalled();
    expect(onStatus).not.toHaveBeenCalled();
  });

  it("sanitizes illegal characters in the channel name (Tauri event constraint)", async () => {
    let subscribedChannel = "";
    subscribeMock.mockImplementation(async (ch: string) => {
      subscribedChannel = ch;
      return vi.fn();
    });
    invokeCommandMock.mockResolvedValue(undefined);

    // Context names using the "<user>@<cluster>" convention contain "@", which
    // Tauri rejects in event names.
    await watchResource("admin@prod.example.com", "kube-system", "pods", vi.fn());

    expect(subscribedChannel).not.toMatch(/[@.]/);
    expect(subscribedChannel).toMatch(/^[a-zA-Z0-9/:_-]+$/);
    // The backend watch is started on the exact sanitized channel we subscribed to.
    expect(invokeCommandMock).toHaveBeenCalledWith("start_resource_watch", {
      context: "admin@prod.example.com",
      namespace: "kube-system",
      kind: "pods",
      channel: subscribedChannel,
      kubeconfigPaths: [],
    });
  });

  it("disposes the subscription if starting the watch fails", async () => {
    const dispose = vi.fn();
    subscribeMock.mockResolvedValue(dispose);
    invokeCommandMock.mockRejectedValueOnce(new Error("boom"));

    await expect(watchResource("c", "ns", "pods", vi.fn())).rejects.toThrow("boom");
    expect(dispose).toHaveBeenCalled();
  });
});
