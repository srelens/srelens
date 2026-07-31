import { afterEach, describe, expect, it, vi } from "vitest";

// isWeb is true under jsdom (no __TAURI_INTERNALS__) by default; these tests
// exercise the desktop (localStorage) branch, so flip the Tauri marker before
// importing (isWeb is a module-level const captured at import time).
function setDesktop(on: boolean) {
  if (on) {
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  } else {
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  }
}

describe("savedForwards on desktop (localStorage)", () => {
  afterEach(() => {
    setDesktop(false);
    vi.resetModules();
    localStorage.clear();
  });

  it("round-trips save -> list -> delete", async () => {
    setDesktop(true);
    vi.resetModules();
    const { listSavedForwards, saveForward, deleteSavedForward } = await import("./savedForwards");

    expect(await listSavedForwards("kind-dev")).toEqual([]);

    const sf = {
      id: "sf-1",
      name: "web console",
      namespace: "default",
      kind: "Service",
      target: "web",
      remotePort: 80,
      localPort: 8080,
    };
    await saveForward("kind-dev", sf);
    expect(await listSavedForwards("kind-dev")).toEqual([sf]);

    await deleteSavedForward("kind-dev", "sf-1");
    expect(await listSavedForwards("kind-dev")).toEqual([]);
  });

  it("upserts by id instead of duplicating", async () => {
    setDesktop(true);
    vi.resetModules();
    const { listSavedForwards, saveForward } = await import("./savedForwards");

    const sf = { id: "sf-1", name: "a", namespace: "default", kind: "Pod", target: "web-1", remotePort: 80 };
    await saveForward("kind-dev", sf);
    await saveForward("kind-dev", { ...sf, name: "renamed" });

    const rows = await listSavedForwards("kind-dev");
    expect(rows).toHaveLength(1);
    expect(rows[0].name).toBe("renamed");
  });

  it("isolates saved forwards per context", async () => {
    setDesktop(true);
    vi.resetModules();
    const { listSavedForwards, saveForward } = await import("./savedForwards");

    const sf = { id: "sf-1", name: "a", namespace: "default", kind: "Pod", target: "web-1", remotePort: 80 };
    await saveForward("kind-dev", sf);

    expect(await listSavedForwards("kind-dev")).toEqual([sf]);
    expect(await listSavedForwards("kind-prod")).toEqual([]);
  });

  it("persists under a single localStorage key holding a per-context map", async () => {
    setDesktop(true);
    vi.resetModules();
    const { saveForward } = await import("./savedForwards");

    const sf = { id: "sf-1", name: "a", namespace: "default", kind: "Pod", target: "web-1", remotePort: 80 };
    await saveForward("kind-dev", sf);

    const raw = localStorage.getItem("srelens.savedForwards");
    expect(raw).toBeTruthy();
    expect(JSON.parse(raw as string)).toEqual({ "kind-dev": [sf] });
  });

  it("tolerates corrupt storage", async () => {
    setDesktop(true);
    localStorage.setItem("srelens.savedForwards", "{not json");
    vi.resetModules();
    const { listSavedForwards } = await import("./savedForwards");
    expect(await listSavedForwards("kind-dev")).toEqual([]);
  });
});

describe("savedForwards on web (settings API)", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    vi.resetModules();
  });

  it("GETs /api/settings/savedForwards:<context> and returns the stored array", async () => {
    // isWeb is already true by default under jsdom in this describe block.
    const { listSavedForwards } = await import("./savedForwards");
    const sf = { id: "sf-1", name: "a", namespace: "default", kind: "Pod", target: "web-1", remotePort: 80 };
    const fetchMock = vi.fn().mockResolvedValue(new Response(JSON.stringify({ value: [sf] }), { status: 200 }));
    vi.stubGlobal("fetch", fetchMock);

    expect(await listSavedForwards("kind-dev")).toEqual([sf]);
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe(`/api/settings/${encodeURIComponent("savedForwards:kind-dev")}`);
    expect(init.credentials).toBe("include");
    expect(init.headers["X-Srelens-Csrf"]).toBeTruthy();
  });

  it("treats a null stored value as an empty list", async () => {
    const { listSavedForwards } = await import("./savedForwards");
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(JSON.stringify({ value: null }), { status: 200 })));
    expect(await listSavedForwards("kind-dev")).toEqual([]);
  });

  it("saveForward reads the current list, appends, and PUTs the whole array", async () => {
    const { saveForward } = await import("./savedForwards");
    const existing = { id: "sf-1", name: "a", namespace: "default", kind: "Pod", target: "web-1", remotePort: 80 };
    const added = { id: "sf-2", name: "b", namespace: "default", kind: "Pod", target: "web-2", remotePort: 81 };
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(new Response(JSON.stringify({ value: [existing] }), { status: 200 }))
      .mockResolvedValueOnce(new Response(null, { status: 204 }));
    vi.stubGlobal("fetch", fetchMock);

    await saveForward("kind-dev", added);

    const [putUrl, putInit] = fetchMock.mock.calls[1];
    expect(putUrl).toBe(`/api/settings/${encodeURIComponent("savedForwards:kind-dev")}`);
    expect(putInit.method).toBe("PUT");
    expect(putInit.headers["Content-Type"]).toBe("application/json");
    expect(JSON.parse(putInit.body)).toEqual([existing, added]);
  });

  it("deleteSavedForward PUTs the array filtered by id", async () => {
    const { deleteSavedForward } = await import("./savedForwards");
    const keep = { id: "sf-1", name: "a", namespace: "default", kind: "Pod", target: "web-1", remotePort: 80 };
    const drop = { id: "sf-2", name: "b", namespace: "default", kind: "Pod", target: "web-2", remotePort: 81 };
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(new Response(JSON.stringify({ value: [keep, drop] }), { status: 200 }))
      .mockResolvedValueOnce(new Response(null, { status: 204 }));
    vi.stubGlobal("fetch", fetchMock);

    await deleteSavedForward("kind-dev", "sf-2");

    const [, putInit] = fetchMock.mock.calls[1];
    expect(JSON.parse(putInit.body)).toEqual([keep]);
  });
});
