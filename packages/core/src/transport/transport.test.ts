import { describe, it, expect, vi, beforeEach } from "vitest";

const { invokeMock, listenMock, relaunchMock, getVersionMock, onCloseRequestedMock, windowDestroyMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  listenMock: vi.fn(),
  relaunchMock: vi.fn(),
  getVersionMock: vi.fn(),
  onCloseRequestedMock: vi.fn(),
  windowDestroyMock: vi.fn(),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));
vi.mock("@tauri-apps/api/event", () => ({ listen: listenMock }));
vi.mock("@tauri-apps/api/app", () => ({ getVersion: getVersionMock }));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: relaunchMock }));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    onCloseRequested: onCloseRequestedMock,
    destroy: windowDestroyMock,
  }),
}));

import { invokeCapability, invokeCommand, on, relaunchApp, appVersion, onWindowCloseRequested } from "./tauriTransport";

beforeEach(() => {
  invokeMock.mockReset();
  listenMock.mockReset();
  relaunchMock.mockReset();
  getVersionMock.mockReset();
  onCloseRequestedMock.mockReset();
  windowDestroyMock.mockReset();
});

describe("transport", () => {
  it("invokeCapability forwards id+input to the tauri command", async () => {
    invokeMock.mockResolvedValue({ pong: "hi" });
    const out = await invokeCapability<{ pong: string }>("ping", "hi");
    expect(invokeMock).toHaveBeenCalledWith("invoke_capability", { id: "ping", input: "hi" });
    expect(out).toEqual({ pong: "hi" });
  });

  it("on subscribes and returns a disposer", async () => {
    const flush = () => new Promise<void>((r) => setTimeout(r, 0));
    const unlisten = vi.fn();
    listenMock.mockResolvedValue(unlisten);
    const handler = vi.fn();
    const dispose = on("catalog:changed", handler);
    await flush();
    expect(listenMock).toHaveBeenCalledWith("catalog:changed", expect.any(Function));
    dispose();
    await flush();
    expect(unlisten).toHaveBeenCalled();
  });

  it("invokeCommand forwards command name and args", async () => {
    invokeMock.mockResolvedValue("ok");
    const out = await invokeCommand<string>("save_text_file", { filename: "a.yaml" });
    expect(invokeMock).toHaveBeenCalledWith("save_text_file", { filename: "a.yaml" });
    expect(out).toBe("ok");
  });

  it("relaunchApp delegates to the process plugin", async () => {
    relaunchMock.mockResolvedValue(undefined);
    await relaunchApp();
    expect(relaunchMock).toHaveBeenCalledTimes(1);
  });

  it("appVersion reads the bundle version", async () => {
    getVersionMock.mockResolvedValue("1.2.3");
    expect(await appVersion()).toBe("1.2.3");
  });

  it("onWindowCloseRequested registers a close handler on the current window", async () => {
    const unlisten = vi.fn();
    onCloseRequestedMock.mockResolvedValue(unlisten);
    const handler = vi.fn();
    const dispose = onWindowCloseRequested(handler);
    expect(onCloseRequestedMock).toHaveBeenCalledWith(expect.any(Function));
    const flush = () => new Promise<void>((r) => setTimeout(r, 0));
    await flush();
    dispose();
    expect(unlisten).toHaveBeenCalled();
  });
});
