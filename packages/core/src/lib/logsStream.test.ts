import { describe, it, expect, vi, beforeEach } from "vitest";

const { invokeCommandMock, subscribeMock } = vi.hoisted(() => ({
  invokeCommandMock: vi.fn(),
  subscribeMock: vi.fn(),
}));
vi.mock("../transport/transport", () => ({
  invokeCommand: invokeCommandMock,
  subscribe: subscribeMock,
}));

import { startLogStream } from "./logsStream";

beforeEach(() => {
  invokeCommandMock.mockReset();
  subscribeMock.mockReset();
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

    // Line payloads → onLine; {status} → onStatus.
    captured?.({ source: "", line: "hello" });
    expect(onLine).toHaveBeenCalledWith("", "hello");
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

  it("still reads a line payload as a line, now that both carry a source", async () => {
    // The two payloads differ only by their `status`/`line` key. A line whose
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

    captured?.({ source: "web-1", line: "status: live" });
    expect(onLine).toHaveBeenCalledWith("web-1", "status: live");
    expect(onStatus).not.toHaveBeenCalled();
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
