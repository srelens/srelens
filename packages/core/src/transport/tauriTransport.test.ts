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
  afterEach(() => vi.restoreAllMocks());

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
