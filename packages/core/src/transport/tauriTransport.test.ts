import { afterEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { invokeCapability } from "./tauriTransport";
import { requestClusterLogin } from "../lib/clusterLogin";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
vi.mock("@tauri-apps/api/app", () => ({ getVersion: vi.fn() }));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: vi.fn() }));

vi.mock("../lib/clusterLogin", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../lib/clusterLogin")>()),
  requestClusterLogin: vi.fn(),
}));

describe("tauriTransport.invokeCapability", () => {
  afterEach(() => { vi.clearAllMocks(); vi.restoreAllMocks(); });

  it("prompts cluster sign-in and rethrows a stable sentinel when the rejection carries the marker", async () => {
    vi.mocked(invoke).mockRejectedValue("NEEDS_CLUSTER_LOGIN:k:ctx");
    await expect(invokeCapability("k8s.listPods")).rejects.toThrow("cluster_login_required");
    expect(requestClusterLogin).toHaveBeenCalledWith(
      expect.objectContaining({ key: "k", context: "ctx" }),
    );
  });

  it("rethrows a normal rejection as-is and does not prompt", async () => {
    vi.mocked(invoke).mockRejectedValue("boom");
    await expect(invokeCapability("k8s.listPods")).rejects.toBe("boom");
    expect(requestClusterLogin).not.toHaveBeenCalled();
  });
});

/**
 * #700: a reloaded page's window still holds the streams the page before it
 * opened. The first stream this page opens waits for the host to end them, so
 * the reset cannot end the new page's streams, and it is asked once a page.
 */
describe("tauriTransport window stream reset", () => {
  const realWarn = console.warn;
  afterEach(() => {
    vi.clearAllMocks();
    vi.restoreAllMocks();
    vi.resetModules();
    // A silenced console.warn must not outlive its case.
    expect(console.warn).toBe(realWarn);
  });

  async function fresh() {
    vi.resetModules();
    const core = await import("@tauri-apps/api/core");
    return { invoke: vi.mocked(core.invoke), transport: await import("./tauriTransport") };
  }

  it("ends the window's old streams before the first stream opens, and only once", async () => {
    const { invoke, transport } = await fresh();
    let finishReset: () => void = () => {};
    invoke.mockImplementation((command: string) =>
      command === "window_streams_reset"
        ? new Promise((resolve) => { finishReset = () => resolve({ appStreams: 2, watches: 1, execs: 0 }); })
        : Promise.resolve("ok"),
    );
    const watch = transport.invokeCommand("start_resource_watch", { channel: "watch:1" });
    const exec = transport.invokeCommand("start_pod_exec", { channel: "exec-1" });
    const app = transport.invokeCommand("extension_stream_open", { input: {} });
    await Promise.resolve();
    expect(invoke.mock.calls.map(([c]) => c)).toEqual(["window_streams_reset"]);
    finishReset();
    await Promise.all([watch, exec, app]);
    expect(invoke.mock.calls.map(([c]) => c)).toEqual([
      "window_streams_reset", "start_resource_watch", "start_pod_exec", "extension_stream_open",
    ]);
    await transport.invokeCommand("start_resource_watch", { channel: "watch:2" });
    expect(invoke.mock.calls.filter(([c]) => c === "window_streams_reset")).toHaveLength(1);
  });

  it("does not hold up a command that opens nothing", async () => {
    const { invoke, transport } = await fresh();
    invoke.mockResolvedValue(undefined);
    await transport.invokeCommand("stop_watch", { channel: "watch:1" });
    expect(invoke.mock.calls.map(([c]) => c)).toEqual(["stop_watch"]);
  });

  it("opens no stream while the old ones may still run: a failed reset rejects each opener, uninvoked", async () => {
    const { invoke, transport } = await fresh();
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    invoke.mockImplementation((command: string) =>
      command === "window_streams_reset" ? Promise.reject("bridge down") : Promise.resolve(7),
    );
    const opens = ["start_resource_watch", "start_pod_exec", "extension_stream_open"].map((c) =>
      transport.invokeCommand(c, {}),
    );
    for (const open of opens) {
      await expect(open).rejects.toThrow(/could not end this window's streams from before the reload.*bridge down/);
    }
    // One attempt, shared by the opens that arrived during it; none ran.
    expect(invoke.mock.calls.map(([c]) => c)).toEqual(["window_streams_reset"]);
    expect(warn).toHaveBeenCalledWith(expect.stringContaining("could not end"), "bridge down");
  });

  it("retries a failed reset on the next open, and opens once one succeeds", async () => {
    const { invoke, transport } = await fresh();
    vi.spyOn(console, "warn").mockImplementation(() => {});
    let resets = 0;
    invoke.mockImplementation((command: string) =>
      command === "window_streams_reset"
        ? (++resets === 1 ? Promise.reject("busy") : Promise.resolve({ appStreams: 0, watches: 0, execs: 0 }))
        : Promise.resolve(7),
    );
    await expect(transport.invokeCommand("start_pod_exec", {})).rejects.toThrow(/busy/);
    await expect(transport.invokeCommand("start_pod_exec", {})).resolves.toBe(7);
    await expect(transport.invokeCommand("start_resource_watch", {})).resolves.toBe(7);
    // Never again after a success: it would end this page's own streams.
    expect(invoke.mock.calls.map(([c]) => c)).toEqual([
      "window_streams_reset", "window_streams_reset", "start_pod_exec", "start_resource_watch",
    ]);
  });

  it("never holds up or blocks a command that opens nothing, even while the reset fails", async () => {
    const { invoke, transport } = await fresh();
    vi.spyOn(console, "warn").mockImplementation(() => {});
    let failReset: () => void = () => {};
    invoke.mockImplementation((command: string) =>
      command === "window_streams_reset"
        ? new Promise((_, reject) => { failReset = () => reject("down"); })
        : Promise.resolve("stopped"),
    );
    const open = transport.invokeCommand("start_resource_watch", {});
    await expect(transport.invokeCommand("stop_watch", { channel: "w" })).resolves.toBe("stopped");
    failReset();
    await expect(open).rejects.toThrow(/down/);
    await expect(transport.invokeCommand("stop_watch", { channel: "w" })).resolves.toBe("stopped");
  });

  it("handles a failed page-load reset, so it only warns", async () => {
    const w = window as unknown as Record<string, unknown>;
    w.__TAURI_INTERNALS__ = {};
    try {
      const { invoke, transport } = await fresh();
      const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
      let failReset: () => void = () => {};
      invoke.mockImplementation(() => new Promise((_, reject) => { failReset = () => reject("bridge down"); }));
      // The page-load call shares this pending attempt; see what it attaches.
      const attempt = transport.resetWindowStreams();
      const then = vi.spyOn(attempt, "then");
      await import("./transport");
      const handlers = then.mock.calls.map(([, onRejected]) => onRejected);
      then.mockRestore();
      expect(handlers.some((h) => typeof h === "function")).toBe(true);
      failReset();
      await expect(attempt).rejects.toBe("bridge down");
      expect(warn).toHaveBeenCalledWith(expect.stringContaining("could not end"), "bridge down");
    } finally {
      delete w.__TAURI_INTERNALS__;
    }
  });

  it("is asked as the desktop transport loads, and never on the web", async () => {
    const { invoke } = await fresh();
    invoke.mockResolvedValue(undefined);
    await import("./transport");
    expect(invoke).not.toHaveBeenCalled();

    vi.resetModules();
    const w = window as unknown as Record<string, unknown>;
    w.__TAURI_INTERNALS__ = {};
    try {
      const core = await import("@tauri-apps/api/core");
      vi.mocked(core.invoke).mockResolvedValue(undefined);
      await import("./transport");
      expect(vi.mocked(core.invoke).mock.calls).toEqual([["window_streams_reset"]]);
    } finally {
      delete w.__TAURI_INTERNALS__;
    }
  });

  it("can be asked for up front, as the page loads", async () => {
    const { invoke, transport } = await fresh();
    invoke.mockResolvedValue(undefined);
    await transport.resetWindowStreams();
    await transport.resetWindowStreams();
    await transport.invokeCommand("extension_stream_open", { input: {} });
    expect(invoke.mock.calls.map(([c]) => c)).toEqual(["window_streams_reset", "extension_stream_open"]);
  });
});
