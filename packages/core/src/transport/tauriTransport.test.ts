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
  afterEach(() => { vi.clearAllMocks(); vi.resetModules(); });

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

  it("still opens streams when the reset fails, and says so", async () => {
    const { invoke, transport } = await fresh();
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    invoke.mockImplementation((command: string) =>
      command === "window_streams_reset" ? Promise.reject("no such command") : Promise.resolve(7),
    );
    await expect(transport.invokeCommand("start_pod_exec", {})).resolves.toBe(7);
    expect(warn).toHaveBeenCalledWith(expect.stringContaining("could not end"), "no such command");
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
