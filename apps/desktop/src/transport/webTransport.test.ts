import { afterEach, describe, expect, it, vi } from "vitest";
import { invokeCapability } from "./webTransport";

describe("webTransport.invokeCapability", () => {
  afterEach(() => vi.restoreAllMocks());

  it("POSTs JSON with the csrf header and credentials, returns the body", async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ pong: "hi" }), { status: 200, headers: { "content-type": "application/json" } }),
    );
    vi.stubGlobal("fetch", fetchMock);
    const out = await invokeCapability<{ pong: string }>("ping", "hi");
    expect(out).toEqual({ pong: "hi" });
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe("/api/capability/ping");
    expect(init.method).toBe("POST");
    expect(init.credentials).toBe("include");
    expect(init.headers["Content-Type"]).toBe("application/json");
    expect(init.headers["X-Srelens-Csrf"]).toBeTruthy();
    expect(init.body).toBe(JSON.stringify("hi"));
  });

  it("maps an error body to a thrown Error", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ error: "capability not found: nope" }), { status: 404 }),
    ));
    await expect(invokeCapability("nope")).rejects.toThrow("capability not found: nope");
  });

  it("throws 'unauthenticated' on 401", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response("", { status: 401 })));
    await expect(invokeCapability("ping")).rejects.toThrow("unauthenticated");
  });
});
