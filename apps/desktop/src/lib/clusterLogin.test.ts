import { afterEach, describe, expect, it, vi } from "vitest";
import {
  parseClusterLoginRequired,
  requestClusterLogin,
  __resetClusterLoginDedupeForTests,
} from "./clusterLogin";

// isWeb is true under jsdom (no __TAURI_INTERNALS__); notify is mocked.
vi.mock("./notify", () => ({ notify: { clusterSignIn: vi.fn() } }));
import { notify } from "./notify";

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
