import { afterEach, describe, expect, it, vi } from "vitest";
import {
  parseClusterLoginRequired,
  requestClusterLogin,
  __resetClusterLoginDedupeForTests,
} from "./clusterLogin";

// isWeb is true under jsdom (no __TAURI_INTERNALS__); notify is mocked.
vi.mock("./notify", () => ({ notify: { clusterSignIn: vi.fn(), error: vi.fn() } }));
import { notify } from "./notify";

function setDesktop(on: boolean) {
  if (on) {
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  } else {
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  }
}

afterEach(() => {
  __resetClusterLoginDedupeForTests();
  vi.clearAllMocks();
});

describe("parseClusterLoginRequired", () => {
  it("parses the HTTP 401 body shape", () => {
    const info = parseClusterLoginRequired({
      error: "cluster_login_required",
      key: "abc",
      context: "prod",
      loginUrl: "/auth/cluster/login?key=abc",
    });
    expect(info).toEqual({ key: "abc", context: "prod", loginUrl: "/auth/cluster/login?key=abc" });
  });

  it("parses a raw marker string and derives the login url", () => {
    const info = parseClusterLoginRequired("connect failed: NEEDS_CLUSTER_LOGIN:abc:prod");
    expect(info).toEqual({ key: "abc", context: "prod", loginUrl: "/auth/cluster/login?key=abc" });
  });

  it("parses a marker nested in an object's error field", () => {
    const info = parseClusterLoginRequired({ error: "NEEDS_CLUSTER_LOGIN:k:ctx" });
    expect(info?.key).toBe("k");
    expect(info?.context).toBe("ctx");
  });

  it("handles a context containing a colon", () => {
    const info = parseClusterLoginRequired("NEEDS_CLUSTER_LOGIN:k:prod:us-east");
    expect(info).toEqual({ key: "k", context: "prod:us-east", loginUrl: "/auth/cluster/login?key=k" });
  });

  it("returns null for unrelated errors", () => {
    expect(parseClusterLoginRequired("connection refused")).toBeNull();
    expect(parseClusterLoginRequired({ error: "boom" })).toBeNull();
    expect(parseClusterLoginRequired(null)).toBeNull();
  });
});

describe("requestClusterLogin dedupe", () => {
  it("prompts once per key within the cooldown", () => {
    const info = { key: "k", context: "prod", loginUrl: "/auth/cluster/login?key=k" };
    requestClusterLogin(info);
    requestClusterLogin(info);
    expect(notify.clusterSignIn).toHaveBeenCalledTimes(1);
  });

  it("prompts separately for different keys", () => {
    requestClusterLogin({ key: "a", context: "x", loginUrl: "/auth/cluster/login?key=a" });
    requestClusterLogin({ key: "b", context: "y", loginUrl: "/auth/cluster/login?key=b" });
    expect(notify.clusterSignIn).toHaveBeenCalledTimes(2);
  });
});

// isWeb is a module-level const captured at import time, so exercising the
// desktop branch needs a fresh module instance: set the Tauri marker, mock
// the transport, then reimport after vi.resetModules() picks it up.
describe("requestClusterLogin on desktop", () => {
  afterEach(() => {
    setDesktop(false);
    vi.doUnmock("../transport/transport");
    vi.unstubAllGlobals();
    vi.resetModules();
  });

  it("prompts via notify.clusterSignIn and drives sign-in through invokeCommand (not a navigation)", async () => {
    setDesktop(true);
    const invokeCommandMock = vi.fn().mockResolvedValue(undefined);
    vi.doMock("../transport/transport", () => ({ invokeCommand: invokeCommandMock }));
    vi.resetModules();

    const {
      requestClusterLogin: requestClusterLoginDesktop,
      __resetClusterLoginDedupeForTests: resetDedupeDesktop,
    } = await import("./clusterLogin");
    const { notify: notifyDesktop } = await import("./notify");
    resetDedupeDesktop();

    const reload = vi.fn();
    vi.stubGlobal("location", { ...location, reload });

    const info = { key: "kd", context: "prod", loginUrl: "/auth/cluster/login?key=kd" };
    requestClusterLoginDesktop(info);

    expect(notifyDesktop.clusterSignIn).toHaveBeenCalledTimes(1);
    expect(notifyDesktop.clusterSignIn).toHaveBeenCalledWith(
      "Sign in to “prod”",
      "This cluster uses OIDC and needs you to sign in.",
      expect.any(Function),
    );
    expect(invokeCommandMock).not.toHaveBeenCalled();

    const onSignIn = vi.mocked(notifyDesktop.clusterSignIn).mock.calls[0][2];
    onSignIn();
    await Promise.resolve();
    await Promise.resolve();

    expect(invokeCommandMock).toHaveBeenCalledWith("cluster_login", { key: "kd" });
    expect(reload).toHaveBeenCalledTimes(1);
  });

  it("reports an error toast (and does not reload) when the sign-in command fails", async () => {
    setDesktop(true);
    const invokeCommandMock = vi.fn().mockRejectedValue(new Error("boom"));
    vi.doMock("../transport/transport", () => ({ invokeCommand: invokeCommandMock }));
    vi.resetModules();

    const {
      requestClusterLogin: requestClusterLoginDesktop,
      __resetClusterLoginDedupeForTests: resetDedupeDesktop,
    } = await import("./clusterLogin");
    const { notify: notifyDesktop } = await import("./notify");
    resetDedupeDesktop();

    const reload = vi.fn();
    vi.stubGlobal("location", { ...location, reload });

    requestClusterLoginDesktop({ key: "kd2", context: "prod", loginUrl: "/auth/cluster/login?key=kd2" });
    const onSignIn = vi.mocked(notifyDesktop.clusterSignIn).mock.calls[0][2];
    onSignIn();
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();

    expect(notifyDesktop.error).toHaveBeenCalledWith("Sign-in failed", "Error: boom");
    expect(reload).not.toHaveBeenCalled();
  });
});
