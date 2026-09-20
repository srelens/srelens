import { describe, it, expect, vi, beforeEach } from "vitest";

const { invokeMock, listenMock, relaunchMock, getVersionMock, onCloseRequestedMock, windowDestroyMock, windowCloseMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  listenMock: vi.fn(),
  relaunchMock: vi.fn(),
  getVersionMock: vi.fn(),
  onCloseRequestedMock: vi.fn(),
  windowDestroyMock: vi.fn(),
  windowCloseMock: vi.fn(),
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
    close: windowCloseMock,
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
  windowCloseMock.mockReset();
});

/** Register a close interceptor and hand back the listener Tauri would call. */
async function closeInterceptor(
  handler: () => Promise<void> | void,
  timeoutMs: number,
  maxRefusals?: number,
) {
  let closeHandler: ((event: { preventDefault: () => void }) => Promise<void>) | undefined;
  onCloseRequestedMock.mockImplementation(async (fn) => {
    closeHandler = fn;
    return vi.fn();
  });
  onWindowCloseRequested(handler, timeoutMs, maxRefusals);
  await Promise.resolve();
  expect(closeHandler).toBeDefined();
  return (event = { preventDefault: vi.fn() }) => closeHandler!(event);
}

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

  /**
   * A cleanup that keeps timing out must not cost the user the window.
   *
   * The first refusal is the stall guard doing its job — the flush may still
   * land, and destroying over it drops the settings write. The second is the
   * user telling us the first answer was not good enough: the close was
   * already prevented, so the titlebar button reads as inert, and refusing
   * forever leaves a window that cannot be closed at all. Same decision as the
   * classic path's bounded flush (#425).
   */
  it("onWindowCloseRequested destroys the window on a second timed-out attempt", async () => {
    windowDestroyMock.mockResolvedValue(undefined);
    const close = await closeInterceptor(() => new Promise(() => {}), 20);

    await close();
    expect(windowDestroyMock).not.toHaveBeenCalled();

    await close();
    expect(windowDestroyMock).toHaveBeenCalledTimes(1);
  });

  /** A handler that rejects every time cannot wedge the window either. */
  it("onWindowCloseRequested destroys after a second failed cleanup", async () => {
    windowDestroyMock.mockResolvedValue(undefined);
    const close = await closeInterceptor(() => Promise.reject(new Error("disk gone")), 20);

    await close();
    expect(windowDestroyMock).not.toHaveBeenCalled();

    await close();
    expect(windowDestroyMock).toHaveBeenCalledTimes(1);
  });

  /**
   * The bound counts attempts over the window's whole life, not per burst: a
   * flush that succeeded once and then wedged still gets its one refusal and
   * no more.
   */
  it("onWindowCloseRequested honours a wider refusal bound before giving up", async () => {
    windowDestroyMock.mockResolvedValue(undefined);
    const close = await closeInterceptor(() => new Promise(() => {}), 20, 2);

    await close();
    await close();
    expect(windowDestroyMock).not.toHaveBeenCalled();

    await close();
    expect(windowDestroyMock).toHaveBeenCalledTimes(1);
  });

  /**
   * destroy() is refused when `core:window:allow-destroy` was not granted —
   * the ungranted-permission case #425 was about. close() re-emits the event,
   * so the guard has to let that one through rather than intercepting itself
   * into a loop.
   */
  it("onWindowCloseRequested falls back to close() when destroy is refused", async () => {
    windowDestroyMock.mockRejectedValue(new Error("not allowed"));
    windowCloseMock.mockResolvedValue(undefined);
    const close = await closeInterceptor(() => Promise.resolve(), 20);

    await close();
    expect(windowDestroyMock).toHaveBeenCalledTimes(1);
    expect(windowCloseMock).toHaveBeenCalledTimes(1);
  });

  it("currentWindowLabel returns the label of the current window or main on web", () => {
    expect(currentWindowLabel()).toBe("ctx-test");
    expect(webWindowLabel()).toBe("main");
  });
});
