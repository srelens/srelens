import { afterEach, describe, expect, it, vi } from "vitest";
import { devLogin, fetchMe, logout } from "./session";

describe("fetchMe", () => {
  afterEach(() => vi.restoreAllMocks());

  it("returns null on 401", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response("", { status: 401 })));
    expect(await fetchMe()).toBeNull();
  });

  it("maps snake_case identity to camelCase", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ user_id: 3, email: "a@x", display_name: "A" }), { status: 200 }),
    ));
    expect(await fetchMe()).toEqual({ userId: 3, email: "a@x", displayName: "A" });
  });

  it("sends credentials and the csrf header", async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response("", { status: 401 }));
    vi.stubGlobal("fetch", fetchMock);
    await fetchMe();
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe("/api/me");
    expect(init.credentials).toBe("include");
    expect(init.headers["X-Srelens-Csrf"]).toBeTruthy();
  });

  it("throws on a non-401 error status", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response("", { status: 500 })));
    await expect(fetchMe()).rejects.toThrow("session check failed: 500");
  });
});

describe("devLogin", () => {
  afterEach(() => vi.restoreAllMocks());

  it("POSTs to /auth/dev-login with credentials and csrf header", async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response("", { status: 200 }));
    vi.stubGlobal("fetch", fetchMock);
    await devLogin();
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe("/auth/dev-login");
    expect(init.method).toBe("POST");
    expect(init.credentials).toBe("include");
    expect(init.headers["X-Srelens-Csrf"]).toBeTruthy();
  });

  it("throws when dev login is disabled", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response("", { status: 403 })));
    await expect(devLogin()).rejects.toThrow("dev login is not enabled");
  });
});

describe("logout", () => {
  afterEach(() => vi.restoreAllMocks());

  it("POSTs to /auth/logout and reloads the page", async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response(null, { status: 204 }));
    vi.stubGlobal("fetch", fetchMock);
    const reload = vi.fn();
    vi.stubGlobal("location", { ...location, reload });
    await logout();
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe("/auth/logout");
    expect(init.method).toBe("POST");
    expect(reload).toHaveBeenCalled();
  });
});
