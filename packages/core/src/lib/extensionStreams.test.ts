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
  extensionStreamMetrics,
  extensionStreamPayload,
  openExtensionView,
  type ExtensionStreamEnd,
} from "./extensionStreams";
// What the Rust `OpenStreamIn` test deserializes, byte for byte.
import wrapperPayload from "./extension-stream-open.json";

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
});
