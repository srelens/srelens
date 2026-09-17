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
    label: "ctx-test",
    onCloseRequested: onCloseRequestedMock,
    destroy: windowDestroyMock,
  }),
}));

import { invokeCapability, invokeCommand, on, relaunchApp, appVersion, onWindowCloseRequested, currentWindowLabel } from "./tauriTransport";
import { currentWindowLabel as webWindowLabel } from "./webTransport";

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

  it("onWindowCloseRequested destroys only after the handler settles", async () => {
    let closeHandler: ((event: { preventDefault: () => void }) => Promise<void>) | undefined;
    onCloseRequestedMock.mockImplementation(async (fn) => {
      closeHandler = fn;
      return vi.fn();
    });
    windowDestroyMock.mockResolvedValue(undefined);
    let resolveFlush!: () => void;
    const flush = new Promise<void>((r) => {
      resolveFlush = r;
    });
    onWindowCloseRequested(() => flush, 200);
    await Promise.resolve();
    expect(closeHandler).toBeDefined();
    const close = closeHandler!({ preventDefault: vi.fn() });
    await new Promise((r) => setTimeout(r, 10));
    expect(windowDestroyMock).not.toHaveBeenCalled();
    resolveFlush();
    await close;
    expect(windowDestroyMock).toHaveBeenCalled();
  });

  it("onWindowCloseRequested leaves the window up when the flush times out", async () => {
    let closeHandler: ((event: { preventDefault: () => void }) => Promise<void>) | undefined;
    onCloseRequestedMock.mockImplementation(async (fn) => {
      closeHandler = fn;
      return vi.fn();
    });
    windowDestroyMock.mockResolvedValue(undefined);
    onWindowCloseRequested(() => new Promise(() => {}), 20);
    await Promise.resolve();
    await closeHandler!({ preventDefault: vi.fn() });
    expect(windowDestroyMock).not.toHaveBeenCalled();
  });

  it("onWindowCloseRequested still preventsDefault while a flush is in flight", async () => {
    let closeHandler: ((event: { preventDefault: () => void }) => Promise<void>) | undefined;
    onCloseRequestedMock.mockImplementation(async (fn) => {
      closeHandler = fn;
      return vi.fn();
    });
    onWindowCloseRequested(() => new Promise(() => {}), 50);
    await Promise.resolve();
    const first = { preventDefault: vi.fn() };
    const second = { preventDefault: vi.fn() };
    const pending = closeHandler!(first);
    await closeHandler!(second);
    expect(first.preventDefault).toHaveBeenCalled();
    expect(second.preventDefault).toHaveBeenCalled();
    await Promise.race([pending, new Promise((r) => setTimeout(r, 80))]);
  });

  it("currentWindowLabel returns the label of the current window or main on web", () => {
    expect(currentWindowLabel()).toBe("ctx-test");
    expect(webWindowLabel()).toBe("main");
  });
});
