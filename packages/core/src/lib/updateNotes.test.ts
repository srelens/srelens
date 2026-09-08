import { beforeEach, afterEach, expect, it, vi } from "vitest";
import { loadUpdateNotes } from "./updateNotes";
beforeEach(() => localStorage.clear());
afterEach(() => vi.unstubAllGlobals());
it("uses embedded notes without a network request", async () => {
  const fetcher = vi.fn(); vi.stubGlobal("fetch", fetcher);
  expect(await loadUpdateNotes({ version: "0.10.1-159", notes: "Changes" })).toBe("Changes");
  expect(fetcher).not.toHaveBeenCalled();
});
it("loads missing notes from the exact release tag", async () => {
  const fetcher = vi.fn().mockResolvedValue({ ok: true, json: async () => ({ body: "### Recent changes\n- Fixed settings" }) });
  vi.stubGlobal("fetch", fetcher);
  expect(await loadUpdateNotes({ version: "0.10.1-159", notes: "" })).toContain("Fixed settings");
  expect(fetcher.mock.calls[0][0]).toBe("https://api.github.com/repos/srelens/srelens/releases/tags/srelens-v0.10.1-159");
});
it("distinguishes a refused read from a release without notes", async () => {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue({ ok: false, status: 403 }));
  await expect(loadUpdateNotes({ version: "0.10.1", notes: "" })).rejects.toThrow("403");
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue({ ok: true, json: async () => ({ body: null }) }));
  expect(await loadUpdateNotes({ version: "0.10.1", notes: "" })).toBe("");
});
it("caches release notes by version and keeps releases separate", async () => {
  localStorage.clear();
  const fetcher = vi.fn().mockResolvedValue({ ok: true, json: async () => ({ body: "Cached current notes" }) });
  vi.stubGlobal("fetch", fetcher);
  await loadUpdateNotes({ version: "0.10.0", notes: "" });
  expect(await loadUpdateNotes({ version: "0.10.0", notes: "" })).toBe("Cached current notes");
  expect(fetcher).toHaveBeenCalledOnce();
  fetcher.mockResolvedValue({ ok: true, json: async () => ({ body: "New release notes" }) });
  expect(await loadUpdateNotes({ version: "0.10.1", notes: "" })).toBe("New release notes");
  expect(fetcher).toHaveBeenCalledTimes(2);
});
it("refreshes cached notes on an explicit retry", async () => {
  localStorage.clear();
  await loadUpdateNotes({ version: "0.10.0", notes: "Old notes" });
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue({ ok: true, json: async () => ({ body: "Corrected notes" }) }));
  expect(await loadUpdateNotes({ version: "0.10.0", notes: "" }, { refresh: true })).toBe("Corrected notes");
});
it("ignores a corrupt cache and bounds saved history", async () => {
  localStorage.setItem("srelens.releaseNotes", "not-json");
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue({ ok: true, json: async () => ({ body: "Recovered" }) }));
  expect(await loadUpdateNotes({ version: "0.10.0", notes: "" })).toBe("Recovered");
  for (let i = 0; i < 12; i++) await loadUpdateNotes({ version: `0.11.${i}`, notes: `Notes ${i}` });
  const cache = JSON.parse(localStorage.getItem("srelens.releaseNotes")!);
  expect(cache).toHaveLength(10);
  expect(cache[0].version).toBe("0.11.11");
});
it("does not cache a malformed release response as an empty release", async () => {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue({ ok: true, json: async () => ({ message: "error" }) }));
  await expect(loadUpdateNotes({ version: "0.10.0", notes: "" })).rejects.toThrow("invalid release response");
  expect(localStorage.getItem("srelens.releaseNotes")).toBeNull();
});
