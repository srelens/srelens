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
    captured?.({ status: "reconnecting" });
    expect(onStatus).toHaveBeenCalledWith("reconnecting");
    expect(onLine).toHaveBeenCalledTimes(1);

    stream.stop();
    expect(dispose).toHaveBeenCalled();
    expect(invokeCommandMock).toHaveBeenCalledWith("stop_log_stream", { channel });
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
