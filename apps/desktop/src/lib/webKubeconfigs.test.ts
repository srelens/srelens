import { afterEach, describe, expect, it, vi } from "vitest";
import { list, remove, upload } from "./webKubeconfigs";

describe("webKubeconfigs.list", () => {
  afterEach(() => vi.restoreAllMocks());

  it("GETs /api/kubeconfigs with credentials and returns the array", async () => {
    const metas = [{ id: 1, name: "prod", createdAt: 1, updatedAt: 2 }];
    const fetchMock = vi.fn().mockResolvedValue(new Response(JSON.stringify(metas), { status: 200 }));
    vi.stubGlobal("fetch", fetchMock);

    expect(await list()).toEqual(metas);
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe("/api/kubeconfigs");
    expect(init.credentials).toBe("include");
    expect(init.headers["X-Srelens-Csrf"]).toBeTruthy();
  });

  it("throws on a non-ok response", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response("", { status: 500 })));
    await expect(list()).rejects.toThrow("list kubeconfigs failed: 500");
  });
});

describe("webKubeconfigs.upload", () => {
  afterEach(() => vi.restoreAllMocks());

  it("POSTs name/yaml as JSON and returns the new id", async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ id: 7 }), { status: 201 }),
    );
    vi.stubGlobal("fetch", fetchMock);

    expect(await upload("prod", "contexts: []")).toBe(7);
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe("/api/kubeconfigs");
    expect(init.method).toBe("POST");
    expect(init.credentials).toBe("include");
    expect(init.headers["Content-Type"]).toBe("application/json");
    expect(init.headers["X-Srelens-Csrf"]).toBeTruthy();
    expect(JSON.parse(init.body)).toEqual({ name: "prod", yaml: "contexts: []" });
  });

  it("maps an error body to a thrown Error", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ error: "name must not be empty" }), { status: 400 }),
    ));
    await expect(upload("", "contexts: []")).rejects.toThrow("name must not be empty");
  });

  it("falls back to a status-based message when the error body isn't JSON", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response("", { status: 500 })));
    await expect(upload("prod", "contexts: []")).rejects.toThrow("upload failed: 500");
  });
});

describe("webKubeconfigs.remove", () => {
  afterEach(() => vi.restoreAllMocks());

  it("DELETEs /api/kubeconfigs/:id", async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response(null, { status: 204 }));
    vi.stubGlobal("fetch", fetchMock);

    await remove(7);
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe("/api/kubeconfigs/7");
    expect(init.method).toBe("DELETE");
    expect(init.credentials).toBe("include");
  });

  it("treats 404 as already-removed", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response("", { status: 404 })));
    await expect(remove(7)).resolves.toBeUndefined();
  });

  it("throws on other non-ok statuses", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response("", { status: 500 })));
    await expect(remove(7)).rejects.toThrow("delete failed: 500");
  });
});
