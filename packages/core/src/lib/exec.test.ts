import { describe, it, expect, vi, beforeEach } from "vitest";

const { invokeCommandMock, onMock } = vi.hoisted(() => ({
  invokeCommandMock: vi.fn(),
  onMock: vi.fn(),
}));
vi.mock("../transport/transport", () => ({
  invokeCommand: invokeCommandMock,
  on: onMock,
}));

import { startPodExec } from "./exec";

beforeEach(() => {
  invokeCommandMock.mockReset();
  onMock.mockReset();
});

describe("startPodExec", () => {
  it("starts a session, wires output/exit, and sends input + close", async () => {
    invokeCommandMock.mockResolvedValueOnce(7); // session id
    const handlers: Record<string, (p: unknown) => void> = {};
    onMock.mockImplementation((ch: string, h: (p: unknown) => void) => {
      handlers[ch] = h;
      return vi.fn();
    });
    const onData = vi.fn();
    const onExit = vi.fn();

    const session = await startPodExec("kind-dev", "default", "web-1", onData, onExit);

    expect(invokeCommandMock).toHaveBeenCalledWith("start_pod_exec", {
      context: "kind-dev",
      namespace: "default",
      pod: "web-1",
      container: null,
      shell: null,
      command: null,
      cols: null,
      rows: null,
    });
    expect(onMock).toHaveBeenCalledWith("exec:out:7", expect.any(Function));
    expect(onMock).toHaveBeenCalledWith("exec:exit:7", expect.any(Function));

    handlers["exec:out:7"]("hello\n");
    expect(onData).toHaveBeenCalledWith("hello\n");
    handlers["exec:exit:7"](null);
    expect(onExit).toHaveBeenCalledWith(null);

    session.send("ls\n");
    expect(invokeCommandMock).toHaveBeenCalledWith("exec_input", { session: 7, data: "ls\n" });
    session.resize(120, 40);
    expect(invokeCommandMock).toHaveBeenCalledWith("exec_resize", { session: 7, cols: 120, rows: 40 });
    session.close();
    expect(invokeCommandMock).toHaveBeenCalledWith("exec_close", { session: 7 });
  });

  it("forwards a container when execing into a specific one", async () => {
    invokeCommandMock.mockResolvedValueOnce(9);
    onMock.mockReturnValue(vi.fn());
    await startPodExec("kind-dev", "default", "web-1", vi.fn(), vi.fn(), "sidecar");
    expect(invokeCommandMock).toHaveBeenCalledWith("start_pod_exec", {
      context: "kind-dev",
      namespace: "default",
      pod: "web-1",
      container: "sidecar",
      shell: null,
      command: null,
      cols: null,
      rows: null,
    });
  });
});
