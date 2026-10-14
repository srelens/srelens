import { describe, it, expect, vi, beforeEach } from "vitest";

const { invokeCommandMock, subscribeMock } = vi.hoisted(() => ({
  invokeCommandMock: vi.fn(),
  subscribeMock: vi.fn(),
}));
vi.mock("../transport/transport", () => ({
  invokeCommand: invokeCommandMock,
  subscribe: subscribeMock,
}));

import { watchNamespaces, watchResource } from "./watch";

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

describe("watchNamespaces", () => {
  /** Every subscribe records its channel and handler; start_resource_watch records its payload. */
  function capture() {
    const handlers = new Map<string, (payload: unknown) => void>();
    const disposes: Array<ReturnType<typeof vi.fn>> = [];
    subscribeMock.mockImplementation(async (ch: string, handler: (p: unknown) => void) => {
      handlers.set(ch, handler);
      const dispose = vi.fn();
      disposes.push(dispose);
      return dispose;
    });
    invokeCommandMock.mockResolvedValue(undefined);
    const started = () =>
      invokeCommandMock.mock.calls
        .filter(([cmd]) => cmd === "start_resource_watch")
        .map(([, args]) => args as { namespace: string; channel: string });
    /** Deliver a payload to the watch started for `namespace`. */
    const emit = (namespace: string, payload: unknown) => {
      const start = started().find((s) => s.namespace === namespace);
      if (!start) throw new Error(`no watch started for ${JSON.stringify(namespace)}`);
      handlers.get(start.channel)?.(payload);
    };
    return { started, emit, disposes };
  }

  it("opens one namespaced watch per selected namespace, never a cluster-scope one", async () => {
    const { started } = capture();

    await watchNamespaces("c", ["team-a", "team-b"], "pods", vi.fn());

    expect(started().map((s) => s.namespace)).toEqual(["team-a", "team-b"]);
  });

  it("opens a single cluster-scope watch for an empty selection (all namespaces)", async () => {
    const { started } = capture();

    await watchNamespaces("c", [], "pods", vi.fn());

    expect(started().map((s) => s.namespace)).toEqual([""]);
  });

  it("merges every namespace's latest snapshot, ordered by name then namespace", async () => {
    const { emit } = capture();
    const onRows = vi.fn();

    await watchNamespaces("c", ["team-b", "team-a"], "pods", onRows);
    emit("team-b", [{ name: "db", namespace: "team-b" }, { name: "web", namespace: "team-b" }]);
    emit("team-a", [{ name: "db", namespace: "team-a" }]);

    expect(onRows).toHaveBeenLastCalledWith([
      { name: "db", namespace: "team-a" },
      { name: "db", namespace: "team-b" },
      { name: "web", namespace: "team-b" },
    ]);
    // A later snapshot from one namespace replaces only that namespace's rows.
    emit("team-b", [{ name: "api", namespace: "team-b" }]);
    expect(onRows).toHaveBeenLastCalledWith([
      { name: "api", namespace: "team-b" },
      { name: "db", namespace: "team-a" },
    ]);
  });

  it("holds the merged snapshot until every namespace has answered or failed", async () => {
    // Emitting team-a's rows alone would paint a list that is missing team-b
    // entirely and call it loaded.
    const { emit } = capture();
    const onRows = vi.fn();
    const onError = vi.fn();

    await watchNamespaces("c", ["team-a", "team-b"], "pods", onRows, undefined, onError);
    emit("team-a", [{ name: "web", namespace: "team-a" }]);
    expect(onRows).not.toHaveBeenCalled();

    emit("team-b", { error: "pods is forbidden" });
    expect(onRows).toHaveBeenLastCalledWith([{ name: "web", namespace: "team-a" }]);
  });

  it("reports a failure with the namespace it came from, and keeps the others' rows", async () => {
    const { emit } = capture();
    const onRows = vi.fn();
    const onError = vi.fn();

    await watchNamespaces("c", ["team-a", "team-b"], "pods", onRows, undefined, onError);
    emit("team-b", { error: 'pods is forbidden: cannot watch resource "pods" in the namespace "team-b"' });
    emit("team-a", [{ name: "web", namespace: "team-a" }]);

    // The message is passed through untouched, so describeError can still
    // classify the apiserver's words; the namespace travels beside it.
    expect(onError).toHaveBeenCalledWith(
      'pods is forbidden: cannot watch resource "pods" in the namespace "team-b"',
      "team-b",
    );
    expect(onRows).toHaveBeenLastCalledWith([{ name: "web", namespace: "team-a" }]);
  });

  it("drops a namespace's rows once its watch fails, rather than keeping them frozen", async () => {
    const { emit } = capture();
    const onRows = vi.fn();

    await watchNamespaces("c", ["team-a", "team-b"], "pods", onRows, undefined, vi.fn());
    emit("team-a", [{ name: "web", namespace: "team-a" }]);
    emit("team-b", [{ name: "db", namespace: "team-b" }]);
    emit("team-b", { error: "pods is forbidden" });

    expect(onRows).toHaveBeenLastCalledWith([{ name: "web", namespace: "team-a" }]);
  });

  it("empties the merged list when every namespace fails after answering", async () => {
    const { emit } = capture();
    const onRows = vi.fn();

    await watchNamespaces("c", ["team-a", "team-b"], "pods", onRows, undefined, vi.fn());
    emit("team-a", [{ name: "web", namespace: "team-a" }]);
    emit("team-b", [{ name: "db", namespace: "team-b" }]);
    emit("team-a", { error: "pods is forbidden" });
    emit("team-b", { error: "pods is forbidden" });

    expect(onRows).toHaveBeenLastCalledWith([]);
  });

  it("emits nothing when every namespace fails before any answered", async () => {
    const { emit } = capture();
    const onRows = vi.fn();

    await watchNamespaces("c", ["team-a", "team-b"], "pods", onRows, undefined, vi.fn());
    emit("team-a", { error: "pods is forbidden" });
    emit("team-b", { error: "pods is forbidden" });

    expect(onRows).not.toHaveBeenCalled();
  });

  it("is reconnecting while any one namespace's watch is, and live once all are", async () => {
    const { emit } = capture();
    const onStatus = vi.fn();

    await watchNamespaces("c", ["team-a", "team-b"], "pods", vi.fn(), onStatus);
    emit("team-a", { status: "reconnecting" });
    expect(onStatus).toHaveBeenLastCalledWith("reconnecting");
    emit("team-b", { status: "reconnecting" });
    emit("team-a", { status: "live" });
    // team-b is still down.
    expect(onStatus).toHaveBeenLastCalledWith("reconnecting");
    emit("team-b", { status: "live" });
    expect(onStatus).toHaveBeenLastCalledWith("live");
  });

  it("goes live again when the only reconnecting namespace fails for good", async () => {
    // A failed watch sends no further status, so leaving it in the
    // reconnecting set would hold "Stream lost" over a live list forever.
    const { emit } = capture();
    const onStatus = vi.fn();

    await watchNamespaces("c", ["team-a", "team-b"], "pods", vi.fn(), onStatus, vi.fn());
    emit("team-b", { status: "reconnecting" });
    emit("team-b", { error: "pods is forbidden" });

    expect(onStatus).toHaveBeenLastCalledWith("live");
  });

  it("stops every namespace's watch through the one handle", async () => {
    const { started, disposes } = capture();

    const handle = await watchNamespaces("c", ["team-a", "team-b"], "pods", vi.fn());
    handle.stop();

    expect(disposes).toHaveLength(2);
    for (const d of disposes) expect(d).toHaveBeenCalled();
    for (const s of started()) {
      expect(invokeCommandMock).toHaveBeenCalledWith("stop_watch", { channel: s.channel });
    }
  });

  it("stops the watches that started when another fails to start, and rejects", async () => {
    const { disposes } = capture();
    invokeCommandMock.mockImplementation(async (cmd: string, args: { namespace?: string }) => {
      if (cmd === "start_resource_watch" && args.namespace === "team-b") throw new Error("boom");
    });

    await expect(watchNamespaces("c", ["team-a", "team-b"], "pods", vi.fn())).rejects.toThrow("boom");
    expect(disposes.every((d) => d.mock.calls.length > 0)).toBe(true);
    expect(invokeCommandMock).toHaveBeenCalledWith("stop_watch", expect.anything());
  });
});
