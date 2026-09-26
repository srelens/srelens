import { beforeEach, describe, expect, it, vi } from "vitest";

const { invokeCommandMock, invokeCapabilityMock, subscribeMock } = vi.hoisted(() => ({
  invokeCommandMock: vi.fn(),
  invokeCapabilityMock: vi.fn(),
  subscribeMock: vi.fn(),
}));
vi.mock("../transport/transport", () => ({
  invokeCommand: invokeCommandMock,
  invokeCapability: invokeCapabilityMock,
  subscribe: subscribeMock,
}));

import {
  describeStreamEnd,
  extensionPods,
  extensionStreamMetrics,
  extensionStreamPayload,
  EXTENSION_INVENTORY_CHANNEL,
  isExtensionExecEvent,
  isExtensionForwardEvent,
  isExtensionLogEvent,
  isExtensionWatchEvent,
  onExtensionInventoryChanged,
  openExtensionView,
  startExtensionLogStream,
  type ExtensionStreamEnd,
} from "./extensionStreams";
// What the Rust `OpenStreamIn` test deserializes, byte for byte.
import wrapperPayload from "./extension-stream-open.json";
import watchPayload from "./extension-stream-watch.json";
// The pod sources (#567), which `pods_tests.rs` deserializes too.
import logsPayload from "./extension-stream-logs.json";
import execPayload from "./extension-stream-exec.json";
import forwardPayload from "./extension-stream-port-forward.json";

describe("watch events (#566)", () => {
  it("accepts exactly the three events a watch sends", () => {
    expect(isExtensionWatchEvent({ event: "synced" })).toBe(true);
    expect(isExtensionWatchEvent({ event: "changed" })).toBe(true);
    expect(isExtensionWatchEvent({ event: "reconnecting", message: "reset" })).toBe(true);
    expect(isExtensionWatchEvent({ event: "reconnecting" })).toBe(false);
    expect(isExtensionWatchEvent({ event: "other" })).toBe(false);
    expect(isExtensionWatchEvent(null)).toBe(false);
  });

  it("hears the host's inventory announcements on their channel", async () => {
    const heard = vi.fn();
    const stop = await onExtensionInventoryChanged(heard);
    expect(calls).toEqual([`subscribe ${EXTENSION_INVENTORY_CHANNEL}`]);
    emit(EXTENSION_INVENTORY_CHANNEL, { type: "changed" });
    expect(heard).toHaveBeenCalledTimes(1);
    stop();
    expect(channels.get(EXTENSION_INVENTORY_CHANNEL)?.dispose).toHaveBeenCalled();
  });
});

// Every channel the client subscribed to, with its handler and its dispose spy.
const channels = new Map<string, { handler: (payload: unknown) => void; dispose: ReturnType<typeof vi.fn> }>();
const calls: string[] = [];

function emit(channel: string, frame: unknown) {
  channels.get(channel)?.handler(frame);
}

beforeEach(() => {
  channels.clear();
  calls.length = 0;
  invokeCommandMock.mockReset();
  invokeCapabilityMock.mockReset();
  subscribeMock.mockReset();
  subscribeMock.mockImplementation(async (channel: string, handler: (payload: unknown) => void) => {
    calls.push(`subscribe ${channel}`);
    const dispose = vi.fn();
    channels.set(channel, { handler, dispose });
    return dispose;
  });
  invokeCommandMock.mockImplementation(async (command: string, args: Record<string, unknown>) => {
    calls.push(command);
    if (command === "extension_stream_open") {
      const input = args.input as { channel: string };
      return { stream: "s-1", channel: input.channel };
    }
    return command === "extension_stream_cancel" ? true : 0;
  });
});

const request = {
  id: "org.example.argocd",
  revision: 1,
  context: "cluster/a",
  namespace: "team",
  source: { kind: "read" as const, capability: "applications", intervalSeconds: 30 },
};

describe("extensionStreamPayload", () => {
  it("is exactly what the host's OpenStreamIn accepts", () => {
    const payload = extensionStreamPayload("org.example.argocd/page:applications#1", "extstream:1-k2j3h4", request);
    expect(payload).toEqual(wrapperPayload);
    // The camelCase spelling, never the struct's own.
    expect(Object.keys(payload.source)).toContain("intervalSeconds");
    expect(JSON.stringify(payload)).not.toContain("interval_seconds");
  });

  it("sends a watch exactly as the host's OpenStreamIn accepts it (#566)", () => {
    const watch = { ...request, source: { kind: "watch" as const, capability: "applications" } };
    expect(extensionStreamPayload("org.example.argocd/page:applications#1", "extstream:2-a8f3k1", watch)).toEqual(watchPayload);
  });

  it("sends the pod sources exactly as the host's OpenStreamIn accepts them (#567)", () => {
    const pods = { id: "org.example.certmanager", revision: 1, context: "cluster/a", namespace: "cert-manager" };
    const view = "org.example.certmanager/resource:controllers#1";
    const logs = extensionStreamPayload(view, "extstream:3-q7w2e9", {
      ...pods,
      source: {
        kind: "logs", capability: "controllerLogs", name: "cert-manager", pod: "cert-manager-7d9f8b6c5-x2x9k",
        container: "cert-manager-controller", tailLines: 200, sinceSeconds: 3600, timestamps: true,
      },
    });
    expect(logs).toEqual(logsPayload);
    expect(JSON.stringify(logs)).not.toMatch(/tail_lines|since_seconds/);
    const command = ["cmctl", "status", "certificate", "--all-namespaces"];
    const exec = extensionStreamPayload(view, "extstream:4-m3n8b1", {
      ...pods,
      source: {
        kind: "exec", capability: "status", name: "cert-manager", pod: "cert-manager-7d9f8b6c5-x2x9k",
        container: "cert-manager-controller",
        confirmed: { pod: "cert-manager-7d9f8b6c5-x2x9k", container: "cert-manager-controller", command },
      },
    });
    expect(exec).toEqual(execPayload);
    const forward = extensionStreamPayload(view, "extstream:5-z4c6v2", {
      ...pods,
      source: { kind: "portForward", capability: "webhook", name: "cert-manager", service: "cert-manager-webhook" },
    });
    expect(forward).toEqual(forwardPayload);
  });

  it("sends a pod source field by field, so nothing the host refuses rides along", () => {
    const sneaky = {
      ...request,
      source: { kind: "exec" as const, capability: "status", pod: "p", command: ["sh", "-c", "id"], localPort: 1 } as never,
    };
    const payload = extensionStreamPayload("v", "extstream:1", sneaky);
    expect(payload.source).toEqual({ kind: "exec", capability: "status", pod: "p" });
  });

  it("sends an empty namespace rather than leaving it out", () => {
    const { namespace: _omitted, ...clusterWide } = request;
    expect(extensionStreamPayload("v", "extstream:1", clusterWide).namespace).toBe("");
  });
});

describe("openExtensionView", () => {
  it("listens before it opens, so the first frame cannot be lost", async () => {
    const view = openExtensionView("org.example.argocd", "page:applications");
    const stream = await view.open(request, { onData: vi.fn() });
    expect(stream.stream).toBe("s-1");
    expect(calls[0]).toMatch(/^subscribe extstream:/);
    expect(calls[1]).toBe("extension_stream_open");
    const input = invokeCommandMock.mock.calls[0][1].input;
    expect(input.view).toBe(view.view);
    expect(input.view.startsWith("org.example.argocd/page:applications#")).toBe(true);
    expect(input.channel).toMatch(/^extstream:[a-zA-Z0-9/:_-]+$/);
  });

  it("gives each mounted view its own id, so closing one leaves the other", () => {
    expect(openExtensionView("a", "page").view).not.toBe(openExtensionView("a", "page").view);
  });

  // The host groups by view id across the whole process: two windows, or a
  // page before and after a reload, each start this module's counter at zero,
  // and closing one must not end the other's streams.
  it("keeps view ids distinct across module instances (windows, reloads)", async () => {
    vi.resetModules();
    const windowA = await import("./extensionStreams");
    vi.resetModules();
    const windowB = await import("./extensionStreams");
    const first = windowA.openExtensionView("org.example.flux", "page:kustomizations").view;
    const second = windowB.openExtensionView("org.example.flux", "page:kustomizations").view;
    expect(second).not.toBe(first);
    expect(second.startsWith("org.example.flux/page:kustomizations#")).toBe(true);
  });

  it("routes data, and tells a close from an error", async () => {
    const view = openExtensionView("org.example.argocd", "page");
    const onData = vi.fn();
    const ends: ExtensionStreamEnd[] = [];
    await view.open(request, { onData, onEnd: (end) => ends.push(end) });
    await view.open(request, { onData, onEnd: (end) => ends.push(end) });
    const [first, second] = [...channels.keys()];
    emit(first, { type: "open", stream: "s-1" });
    emit(first, { type: "data", stream: "s-1", seq: 1, data: { items: [] } });
    expect(onData).toHaveBeenCalledWith({ items: [] }, 1);
    emit(first, { type: "close", stream: "s-1", reason: "appDisabled" });
    emit(second, { type: "error", stream: "s-2", code: "source", message: "the cluster said no" });
    expect(ends).toEqual([
      { type: "close", reason: "appDisabled" },
      { type: "error", code: "source", message: "the cluster said no" },
    ]);
    // A terminal frame is the last: the client stops listening.
    expect(channels.get(first)!.dispose).toHaveBeenCalledTimes(1);
    expect(channels.get(second)!.dispose).toHaveBeenCalledTimes(1);
    emit(first, { type: "data", stream: "s-1", seq: 2, data: {} });
    expect(onData).toHaveBeenCalledTimes(1);
  });

  it("ignores frames it does not know rather than guessing", async () => {
    const view = openExtensionView("a", "page");
    const onData = vi.fn();
    const onEnd = vi.fn();
    await view.open(request, { onData, onEnd });
    const [channel] = [...channels.keys()];
    for (const frame of [null, "x", { type: "later" }, { type: "data" }]) emit(channel, frame);
    expect(onData).not.toHaveBeenCalled();
    expect(onEnd).not.toHaveBeenCalled();
  });

  it("stops listening when the host refuses the open, and says why", async () => {
    invokeCommandMock.mockRejectedValueOnce("App org.example.argocd already has 8 open streams");
    const view = openExtensionView("org.example.argocd", "page");
    await expect(view.open(request, { onData: vi.fn() })).rejects.toBe(
      "App org.example.argocd already has 8 open streams",
    );
    const [channel] = [...channels.keys()];
    expect(channels.get(channel)!.dispose).toHaveBeenCalledTimes(1);
  });

  it("cancels one stream, and only once", async () => {
    const view = openExtensionView("a", "page");
    const stream = await view.open(request, { onData: vi.fn() });
    await stream.cancel();
    expect(invokeCommandMock).toHaveBeenCalledWith("extension_stream_cancel", { stream: "s-1" });
    const [channel] = [...channels.keys()];
    emit(channel, { type: "close", stream: "s-1", reason: "cancelled" });
    await stream.cancel();
    expect(invokeCommandMock.mock.calls.filter(([c]) => c === "extension_stream_cancel")).toHaveLength(1);
  });

  it("closing the view ends its streams on the host, once", async () => {
    const view = openExtensionView("a", "page");
    await view.open(request, { onData: vi.fn() });
    await view.close();
    await view.close();
    const closes = invokeCommandMock.mock.calls.filter(([c]) => c === "extension_stream_close_view");
    expect(closes).toEqual([["extension_stream_close_view", { view: view.view }]]);
    await expect(view.open(request, { onData: vi.fn() })).rejects.toThrow("closed");
  });

  it("cancels a stream whose open was still in flight when the view closed", async () => {
    let answer: (value: unknown) => void = () => {};
    invokeCommandMock.mockImplementationOnce(
      () => new Promise((resolve) => (answer = resolve)),
    );
    const view = openExtensionView("a", "page");
    const pending = view.open(request, { onData: vi.fn() });
    await vi.waitFor(() => expect(invokeCommandMock).toHaveBeenCalledTimes(1));
    await view.close();
    answer({ stream: "s-9", channel: "extstream:x" });
    await pending;
    expect(invokeCommandMock).toHaveBeenCalledWith("extension_stream_cancel", { stream: "s-9" });
  });
});

describe("extensionStreamMetrics", () => {
  it("reads the host's counters", async () => {
    invokeCapabilityMock.mockResolvedValue({ apps: [], maxOpenPerApp: 8, messagesPerSecond: 50 });
    await expect(extensionStreamMetrics()).resolves.toEqual({ apps: [], maxOpenPerApp: 8, messagesPerSecond: 50 });
    expect(invokeCapabilityMock).toHaveBeenCalledWith("extensions.streams", {});
  });
});

describe("describeStreamEnd", () => {
  it("says why a stream ended, and a failure is never worded as an ending", () => {
    expect(describeStreamEnd({ type: "close", reason: "appUpdated" })).toBe(
      "The app was updated; reopen the view to follow it again.",
    );
    expect(describeStreamEnd({ type: "close", reason: "appDisabled" })).toBe("The app was disabled.");
    expect(describeStreamEnd({ type: "error", code: "source", message: "forbidden" })).toBe(
      "The stream failed: forbidden",
    );
    expect(describeStreamEnd({ type: "error", code: "rateLimited", message: "App a sent more than 50 stream messages per second" })).toBe(
      "The host stopped the stream: App a sent more than 50 stream messages per second",
    );
  });

  // #700: the host ends a window's streams when it closes or reloads.
  it("says a stream ended with its window, as an ending", () => {
    expect(describeStreamEnd({ type: "close", reason: "windowClosed" })).toBe("The window that opened the stream closed.");
    expect(describeStreamEnd({ type: "close", reason: "windowReloaded" })).toBe("The window that opened the stream reloaded.");
  });
});

describe("a window's ending (#700)", () => {
  it("reaches onEnd as a close with its reason, and nothing after it", async () => {
    const onEnd = vi.fn();
    const onData = vi.fn();
    const view = openExtensionView("org.example.argocd", "page:applications");
    await view.open(request, { onData, onEnd });
    const [channel] = [...channels.keys()];
    emit(channel, { type: "close", stream: "s-1", reason: "windowReloaded" });
    emit(channel, { type: "data", stream: "s-1", seq: 1, data: {} });
    expect(onEnd).toHaveBeenCalledWith({ type: "close", reason: "windowReloaded" } satisfies ExtensionStreamEnd);
    expect(onData).not.toHaveBeenCalled();
    expect(channels.get(channel)?.dispose).toHaveBeenCalled();
  });
});

describe("pod source frames (#567)", () => {
  it("accepts exactly what the pod sources send", () => {
    expect(isExtensionLogEvent({ event: "lines", lines: [{ source: "p/c", line: "x" }] })).toBe(true);
    expect(isExtensionLogEvent({ event: "lines", lines: [{ source: "p/c", line: "x" }], dropped: 3 })).toBe(true);
    expect(isExtensionLogEvent({ event: "status", source: "p/c", status: "reconnecting", message: "EOF" })).toBe(true);
    expect(isExtensionLogEvent({ event: "status", source: "p/c", status: "sideways" })).toBe(false);
    expect(isExtensionLogEvent({ event: "lines", lines: [{ line: "no source" }] })).toBe(false);
    expect(isExtensionExecEvent({ event: "output", chunks: [{ stream: "stderr", text: "x" }] })).toBe(true);
    expect(isExtensionExecEvent({ event: "exit", code: 3 })).toBe(true);
    expect(isExtensionExecEvent({ event: "exit", code: "3" })).toBe(false);
    expect(isExtensionExecEvent({ event: "output", chunks: [{ stream: "stdin", text: "x" }] })).toBe(false);
    expect(isExtensionForwardEvent({ event: "ready", localPort: 54321, pod: "p", port: 9402 })).toBe(true);
    expect(isExtensionForwardEvent({ event: "ready", localPort: "54321", pod: "p", port: 9402 })).toBe(false);
  });

  it("reads the pods a binding may reach through extensions.pods", async () => {
    invokeCapabilityMock.mockResolvedValue({ pods: [], scope: "pods selected by Deployment web" });
    await extensionPods({ id: "a.b", revision: 2, capability: "logs", context: "c", namespace: "team", name: "web" });
    expect(invokeCapabilityMock).toHaveBeenCalledWith("extensions.pods", {
      id: "a.b", revision: 2, capability: "logs", context: "c", namespace: "team", name: "web",
    });
    await extensionPods({ id: "a.b", revision: 2, capability: "logs", context: "c", namespace: "cert-manager" });
    expect(invokeCapabilityMock).toHaveBeenLastCalledWith("extensions.pods", {
      id: "a.b", revision: 2, capability: "logs", context: "c", namespace: "cert-manager",
    });
  });
});

describe("startExtensionLogStream (#567)", () => {
  const logs = {
    id: "org.example.certmanager",
    revision: 1,
    context: "cluster/a",
    namespace: "team",
    source: { kind: "logs" as const, capability: "controllerLogs", name: "web", pod: "web-1", container: "app" },
  };

  // The pod log view follows any log source through the callbacks it gives
  // `startLogStream`, so an app's logs — and a log provider's later (#569) —
  // land in the same buffer as the cluster's own.
  it("speaks startLogStream's callbacks: lines by source, status by source", async () => {
    const onLine = vi.fn();
    const onStatus = vi.fn();
    const onDropped = vi.fn();
    const onEnd = vi.fn();
    const view = openExtensionView("org.example.certmanager", "logs");
    const stream = await startExtensionLogStream(view, logs, onLine, onStatus, { onDropped, onEnd });
    const [channel] = [...channels.keys()];
    emit(channel, { type: "open", stream: "s-1" });
    emit(channel, { type: "data", stream: "s-1", seq: 1, data: { event: "status", source: "web-1/app", status: "live" } });
    emit(channel, {
      type: "data", stream: "s-1", seq: 2,
      data: { event: "lines", lines: [{ source: "web-1/app", line: "one" }, { source: "web-1/app", line: "two" }], dropped: 4 },
    });
    emit(channel, { type: "data", stream: "s-1", seq: 3, data: { event: "surprise" } });
    expect(onStatus).toHaveBeenCalledWith("live", "web-1/app");
    expect(onLine.mock.calls).toEqual([["web-1/app", "one"], ["web-1/app", "two"]]);
    expect(onDropped).toHaveBeenCalledWith(4);
    emit(channel, { type: "error", stream: "s-1", code: "source", message: "forbidden" });
    expect(onEnd).toHaveBeenCalledWith({ type: "error", code: "source", message: "forbidden" });
    stream.stop();
    expect(invokeCommandMock).not.toHaveBeenCalledWith("extension_stream_cancel", expect.anything());
  });

  it("stops by cancelling its stream while it runs", async () => {
    const view = openExtensionView("org.example.certmanager", "logs");
    const stream = await startExtensionLogStream(view, logs, vi.fn());
    stream.stop();
    await Promise.resolve();
    expect(invokeCommandMock).toHaveBeenCalledWith("extension_stream_cancel", { stream: "s-1" });
  });
});
