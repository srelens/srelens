import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

const { invokeCommandMock, subscribeMock, platform } = vi.hoisted(() => ({
  invokeCommandMock: vi.fn(),
  subscribeMock: vi.fn(),
  platform: { tauri: true },
}));
vi.mock("../transport/transport", () => ({
  invokeCommand: invokeCommandMock,
  subscribe: subscribeMock,
}));
vi.mock("../transport/platform", () => ({ isTauri: () => platform.tauri }));
vi.mock("../transport/webTransport", () => ({ csrfHeader: () => ({ "X-Srelens-Csrf": "t" }) }));

import { startLogStream } from "./logsStream";

beforeEach(() => {
  invokeCommandMock.mockReset();
  subscribeMock.mockReset();
  platform.tauri = true;
});

describe("startLogStream", () => {
  it("rejects an empty target set without opening a subscription", async () => {
    await expect(startLogStream("kind-dev", "default", [], vi.fn())).rejects.toThrow(
      "without a pod target",
    );
    expect(subscribeMock).not.toHaveBeenCalled();
    expect(invokeCommandMock).not.toHaveBeenCalled();
  });

  it("subscribes first, forwards lines and status, and stops cleanly", async () => {
    let captured: ((payload: unknown) => void) | undefined;
    let channel = "";
    const dispose = vi.fn();
    subscribeMock.mockImplementation(async (ch: string, handler: (p: unknown) => void) => {
      channel = ch;
      captured = handler;
      return dispose;
    });
    invokeCommandMock.mockResolvedValue(undefined);

    const onLine = vi.fn();
    const onStatus = vi.fn();
    const stream = await startLogStream(
      "kind-dev",
      "default",
      [{ pod: "web-1", container: "app", label: "" }],
      onLine,
      onStatus,
    );

    // Subscribed before starting the backend stream, on the same channel.
    expect(subscribeMock.mock.invocationCallOrder[0]).toBeLessThan(
      invokeCommandMock.mock.invocationCallOrder[0],
    );
    expect(invokeCommandMock).toHaveBeenCalledWith(
      "start_log_stream",
      expect.objectContaining({ channel, targets: [{ pod: "web-1", container: "app", label: "" }] }),
    );

    // Frames → onLine; {status} → onStatus.
    captured?.({ seq: 1, lines: [{ source: "", line: "hello" }] });
    expect(onLine).toHaveBeenCalledWith("", "hello", false);
    captured?.({ source: "", status: "reconnecting" });
    expect(onStatus).toHaveBeenCalledWith("reconnecting", "");
    expect(onLine).toHaveBeenCalledTimes(1);

    stream.stop();
    expect(dispose).toHaveBeenCalled();
    expect(invokeCommandMock).toHaveBeenCalledWith("stop_log_stream", { channel });
  });

  it("tells the caller WHICH target each status came from", async () => {
    // A stream fans out over many targets, each reconnecting on its own
    // schedule. The backend tags every status with the same source its lines
    // carry (`crates/streams/src/logs.rs`); dropping that tag here is what
    // forced a subscriber to guess whether one pod dropped out or all of them.
    let captured: ((payload: unknown) => void) | undefined;
    subscribeMock.mockImplementation(async (_ch: string, handler: (p: unknown) => void) => {
      captured = handler;
      return vi.fn();
    });
    invokeCommandMock.mockResolvedValue(undefined);

    const onLine = vi.fn();
    const onStatus = vi.fn();
    await startLogStream(
      "kind-dev",
      "default",
      [
        { pod: "web-1", container: "app", label: "web-1" },
        { pod: "web-2", container: "app", label: "web-2" },
      ],
      onLine,
      onStatus,
    );

    captured?.({ source: "web-1", status: "reconnecting" });
    captured?.({ source: "web-2", status: "live" });
    expect(onStatus).toHaveBeenNthCalledWith(1, "reconnecting", "web-1");
    expect(onStatus).toHaveBeenNthCalledWith(2, "live", "web-2");
    expect(onLine).not.toHaveBeenCalled();
  });

  it("still reads a frame's line as a line, now that both carry a source", async () => {
    // The two payloads differ only by their `status`/`lines` key. A line whose
    // text happens to mention a status word is still a line.
    let captured: ((payload: unknown) => void) | undefined;
    subscribeMock.mockImplementation(async (_ch: string, handler: (p: unknown) => void) => {
      captured = handler;
      return vi.fn();
    });
    invokeCommandMock.mockResolvedValue(undefined);

    const onLine = vi.fn();
    const onStatus = vi.fn();
    await startLogStream(
      "kind-dev",
      "default",
      [{ pod: "web-1", container: "app", label: "web-1" }],
      onLine,
      onStatus,
    );

    captured?.({ seq: 1, lines: [{ source: "web-1", line: "status: live" }] });
    expect(onLine).toHaveBeenCalledWith("web-1", "status: live", false);
    expect(onStatus).not.toHaveBeenCalled();
  });

  it("says when the host cut a line, and only then (#747)", async () => {
    let captured: ((payload: unknown) => void) | undefined;
    subscribeMock.mockImplementation(async (_ch: string, handler: (p: unknown) => void) => {
      captured = handler;
      return vi.fn();
    });
    invokeCommandMock.mockResolvedValue(undefined);
    const onLine = vi.fn();
    await startLogStream("kind-dev", "default", [{ pod: "web-1", label: "web-1" }], onLine);
    captured?.({ seq: 1, lines: [{ source: "web-1", line: "the start of it", truncated: true }] });
    captured?.({ seq: 2, lines: [{ source: "web-1", line: "whole" }] });
    expect(onLine.mock.calls).toEqual([
      ["web-1", "the start of it", true],
      ["web-1", "whole", false],
    ]);
  });

  it("reports an untagged status as the unlabelled source rather than dropping it", async () => {
    // A single-target stream labels nothing (`label: ""`), so an absent tag
    // and an empty one mean the same thing: the one source there is.
    let captured: ((payload: unknown) => void) | undefined;
    subscribeMock.mockImplementation(async (_ch: string, handler: (p: unknown) => void) => {
      captured = handler;
      return vi.fn();
    });
    invokeCommandMock.mockResolvedValue(undefined);

    const onStatus = vi.fn();
    await startLogStream(
      "kind-dev",
      "default",
      [{ pod: "web-1", container: "app", label: "" }],
      vi.fn(),
      onStatus,
    );

    captured?.({ status: "live" });
    expect(onStatus).toHaveBeenCalledWith("live", "");
  });

  it("forwards tail/since/timestamps options to the backend command", async () => {
    subscribeMock.mockResolvedValue(vi.fn());
    invokeCommandMock.mockResolvedValue(undefined);
    await startLogStream(
      "kind-dev",
      "default",
      [{ pod: "web-1", container: "app", label: "" }],
      vi.fn(),
      vi.fn(),
      { timestamps: true, sinceSeconds: 600, tailLines: 1000 },
    );
    expect(invokeCommandMock).toHaveBeenCalledWith(
      "start_log_stream",
      expect.objectContaining({ timestamps: true, sinceSeconds: 600, tailLines: 1000 }),
    );
  });
});

/** Opens a stream and hands back the backend's side of its channel. */
async function open(
  options: Parameters<typeof startLogStream>[5] = {},
  onStatus?: Parameters<typeof startLogStream>[4],
) {
  let emit!: (payload: unknown) => void;
  let channel = "";
  subscribeMock.mockImplementation(async (ch: string, handler: (p: unknown) => void) => {
    channel = ch;
    emit = handler;
    return vi.fn();
  });
  invokeCommandMock.mockResolvedValue(undefined);
  const onLine = vi.fn();
  const stream = await startLogStream("kind-dev", "default", [{ pod: "web-1", label: "" }], onLine, onStatus, options);
  return { stream, onLine, emit: (p: unknown) => emit(p), channel: () => channel };
}

describe("startLogStream frames (#914)", () => {
  // The web tests stub `fetch`, the URL blob functions (jsdom has none) and
  // an anchor's `click`. Put them all back after each test, even one that
  // failed halfway, so a failure cannot leak a stub into the next test.
  const realCreateObjectURL = URL.createObjectURL;
  const realRevokeObjectURL = URL.revokeObjectURL;
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    URL.createObjectURL = realCreateObjectURL;
    URL.revokeObjectURL = realRevokeObjectURL;
  });

  it("hands a frame whole to onLines, with where it starts and where history starts", async () => {
    const onLines = vi.fn();
    const { emit } = await open({ onLines });
    emit({ seq: 7, first: 1, lines: [{ source: "", line: "a" }, { source: "web-2", line: "b", truncated: true }] });
    expect(onLines).toHaveBeenCalledWith(
      [{ source: "", text: "a" }, { source: "web-2", text: "b", truncated: true }],
      { seq: 7, first: 1 },
    );
  });

  it("calls onLine per line when no onLines was given", async () => {
    const { emit, onLine } = await open();
    emit({ seq: 1, lines: [{ source: "", line: "a" }, { source: "", line: "b" }] });
    expect(onLine.mock.calls).toEqual([["", "a", false], ["", "b", false]]);
  });

  it("ignores a payload that is not an object, or is none of the three kinds", async () => {
    // Nothing guarantees the channel carries only what the backend means to
    // send: a stray `null` or string must not throw out of the subscription.
    const onLines = vi.fn();
    const onSpoolFailed = vi.fn();
    const onStatus = vi.fn();
    const { emit, onLine } = await open({ onLines, onSpoolFailed, spoolMaxBytes: 1 << 20 }, onStatus);
    for (const stray of [null, "x", {}, { foo: 1 }]) expect(() => emit(stray)).not.toThrow();
    for (const callback of [onLine, onLines, onStatus, onSpoolFailed]) expect(callback).not.toHaveBeenCalled();
  });

  it("says when the backend stopped keeping history", async () => {
    const onSpoolFailed = vi.fn();
    const { emit } = await open({ onSpoolFailed, spoolMaxBytes: 1 << 20 });
    emit({ spool: { failed: "No space left on device" } });
    expect(onSpoolFailed).toHaveBeenCalledWith("No space left on device");
  });

  it("asks for history only when told to, and only then can read or export it", async () => {
    const plain = await open();
    expect(invokeCommandMock).toHaveBeenCalledWith("start_log_stream", expect.objectContaining({ spoolMaxBytes: null }));
    expect(plain.stream.readEarlier).toBeUndefined();
    expect(plain.stream.exportSpool).toBeUndefined();

    const spooled = await open({ spoolMaxBytes: 256 << 20 });
    expect(invokeCommandMock).toHaveBeenLastCalledWith(
      "start_log_stream",
      expect.objectContaining({ spoolMaxBytes: 256 << 20 }),
    );
    invokeCommandMock.mockResolvedValueOnce({ first: 1, seq: 3, lines: [{ source: "", line: "c" }] });
    await expect(spooled.stream.readEarlier!(4, 5000)).resolves.toEqual({
      first: 1,
      seq: 3,
      lines: [{ source: "", text: "c" }],
    });
    expect(invokeCommandMock).toHaveBeenLastCalledWith("read_log_spool", { channel: spooled.channel(), before: 4, limit: 5000 });
  });

  it("exports through the native dialog on the desktop", async () => {
    const { stream, channel } = await open({ spoolMaxBytes: 1 << 20 });
    invokeCommandMock.mockResolvedValueOnce("/home/u/web-1.log");
    await expect(stream.exportSpool!("web-1.log", 600)).resolves.toBe("/home/u/web-1.log");
    expect(invokeCommandMock).toHaveBeenLastCalledWith("export_log_spool", {
      channel: channel(),
      filename: "web-1.log",
      sinceSeconds: 600,
    });
  });

  it("downloads the server's export in a browser, and says the server's reason when it refuses", async () => {
    platform.tauri = false;
    const { stream, channel } = await open({ spoolMaxBytes: 1 << 20 });
    const fetchMock = vi.fn(async (_url: string, _init?: RequestInit) => ({ ok: true, status: 200, statusText: "OK", blob: async () => new Blob(["x"]), text: async () => "" }));
    vi.stubGlobal("fetch", fetchMock);
    const created = vi.fn(() => "blob:x");
    const revoked = vi.fn();
    URL.createObjectURL = created;
    URL.revokeObjectURL = revoked;
    const click = vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => {});

    await expect(stream.exportSpool!("web-1.log")).resolves.toBe("web-1.log");
    expect(fetchMock).toHaveBeenCalledWith(
      `/api/logs/export?channel=${encodeURIComponent(channel())}`,
      { credentials: "include", headers: { "X-Srelens-Csrf": "t" } },
    );
    expect(click).toHaveBeenCalled();
    // The browser saves the blob under the name asked for, and nothing is left behind.
    const anchor = click.mock.contexts[0] as HTMLAnchorElement;
    expect(anchor.download).toBe("web-1.log");
    expect(document.body.contains(anchor)).toBe(false);
    expect(revoked).toHaveBeenCalledWith("blob:x");

    fetchMock.mockResolvedValueOnce({ ok: false, status: 404, statusText: "Not Found", blob: async () => new Blob([]), text: async () => '{"error":"this log stream keeps no history"}' });
    await expect(stream.exportSpool!("web-1.log", 3600)).rejects.toThrow("this log stream keeps no history");
    // A window asks the server for only the last hour.
    expect(fetchMock.mock.calls[1][0]).toBe(`/api/logs/export?channel=${encodeURIComponent(channel())}&sinceSeconds=3600`);
  });

  it("falls back to the status text when a browser's refusal is not JSON", async () => {
    // A reverse proxy in front of the server answers in HTML, not `{error}`.
    platform.tauri = false;
    const { stream } = await open({ spoolMaxBytes: 1 << 20 });
    const refuse = (statusText: string) => ({ ok: false, status: 502, statusText, blob: async () => new Blob([]), text: async () => "<html>502</html>" });
    const fetchMock = vi.fn(async () => refuse("Bad Gateway"));
    vi.stubGlobal("fetch", fetchMock);
    await expect(stream.exportSpool!("web-1.log")).rejects.toThrow("Bad Gateway");

    // Over HTTP/2 there is no reason phrase, so the status text is empty: say the code instead.
    fetchMock.mockResolvedValueOnce(refuse(""));
    await expect(stream.exportSpool!("web-1.log")).rejects.toThrow("HTTP 502");
  });
});
