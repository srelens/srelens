import { describe, it, expect, vi, beforeEach } from "vitest";

const { startPodExecMock, startLocalTerminalMock } = vi.hoisted(() => ({
  startPodExecMock: vi.fn(),
  startLocalTerminalMock: vi.fn(),
}));
vi.mock("./exec", () => ({ startPodExec: startPodExecMock }));
vi.mock("./terminal", () => ({ startLocalTerminal: startLocalTerminalMock }));

import { podExecDriver, localTerminalDriver } from "./terminalDriver";

const conn = () => ({ send: vi.fn(), resize: vi.fn(), close: vi.fn() });

beforeEach(() => {
  startPodExecMock.mockReset();
  startLocalTerminalMock.mockReset();
});

describe("podExecDriver", () => {
  it("maps a backend error to an unexpected drop and a clean end to closed", async () => {
    let backendOnExit: ((err: string | null) => void) | undefined;
    startPodExecMock.mockImplementation((_c, _n, _p, _onData, onExit) => {
      backendOnExit = onExit;
      return Promise.resolve(conn());
    });
    const driver = podExecDriver({ context: "c", namespace: "n", pod: "p" });
    expect(driver.kind).toBe("pod");
    expect(driver.reconnectable).toBe(true);

    const onExit = vi.fn();
    await driver.connect({ onData: vi.fn(), onExit });
    backendOnExit?.("boom");
    expect(onExit).toHaveBeenCalledWith({ kind: "error", message: "boom" });
    backendOnExit?.(null);
    expect(onExit).toHaveBeenCalledWith({ kind: "closed" });
  });

  it("threads command, container, and initial size through", async () => {
    startPodExecMock.mockResolvedValue(conn());
    const driver = podExecDriver({ context: "c", namespace: "n", pod: "p", command: ["nsenter"], kind: "node" });
    expect(driver.kind).toBe("node");
    await driver.connect({ onData: vi.fn(), onExit: vi.fn(), initialSize: { cols: 100, rows: 30 } });
    expect(startPodExecMock).toHaveBeenCalledWith(
      "c",
      "n",
      "p",
      expect.any(Function),
      expect.any(Function),
      undefined,
      ["nsenter"],
      { cols: 100, rows: 30 },
    );
  });
});

describe("localTerminalDriver", () => {
  it("treats a shell exit as an intentional clean close", async () => {
    let backendOnExit: (() => void) | undefined;
    startLocalTerminalMock.mockImplementation((_c, _k, _onData, onExit) => {
      backendOnExit = onExit;
      return Promise.resolve(conn());
    });
    const driver = localTerminalDriver({ context: "c", extraKubeconfigs: [] });
    expect(driver.kind).toBe("local");

    const onExit = vi.fn();
    await driver.connect({ onData: vi.fn(), onExit });
    backendOnExit?.();
    expect(onExit).toHaveBeenCalledWith({ kind: "closed" });
  });
});
