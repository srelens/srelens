import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
vi.mock("@srelens/core", async original => ({ ...await original<typeof import("@srelens/core")>(), listContexts:vi.fn() }));
import { listContexts } from "@srelens/core";
import { useContextId, useContextLookup } from "./contextIds";

// `/kube/a` declaring `b#c` and `/kube/a#b` declaring `c` share the stable ID `/kube/a#b#c` (#623).
const shared = [
  { name:"b#c", stableId:"/kube/a#b#c", key:"/kube/a#b%23c", pinnedId:"srelens-context:/kube/a#b%23c" },
  { name:"c", stableId:"/kube/a#b#c", key:"/kube/a%23b#c", pinnedId:"srelens-context:/kube/a%23b#c" },
];

beforeEach(() => {
  vi.mocked(listContexts).mockReset().mockResolvedValue({ contexts:shared } as never);
});

async function lookup(context: string) {
  const { result } = renderHook(() => useContextLookup(context));
  await waitFor(() => expect(result.current.status).not.toBe("loading"));
  return result.current;
}

it("finds a context by the display name the Inspector names it by", async () => {
  expect(await lookup("c")).toEqual({ status:"found", id:"/kube/a%23b#c" });
});

it("finds a context by the pinned ID an app page asks the host by (#695)", async () => {
  expect(await lookup("srelens-context:/kube/a%23b#c")).toEqual({ status:"found", id:"/kube/a%23b#c" });
  expect(await lookup("srelens-context:/kube/a#b%23c")).toEqual({ status:"found", id:"/kube/a#b%23c" });
});

it("never finds a context by the stable ID two contexts share", async () => {
  expect(await lookup("/kube/a#b#c")).toEqual({ status:"missing" });
});

it("finds neither context when a string is one context's name and another's pinned ID", async () => {
  // A context literally named after another's pinned ID. Which one the caller meant is not
  // known, so neither is chosen; the host's find_context refuses the string too.
  vi.mocked(listContexts).mockResolvedValue({ contexts:[...shared, { name:"srelens-context:/kube/a%23b#c",
    stableId:"/kube/x#srelens-context:/kube/a%23b#c", key:"/kube/x#srelens-context:/kube/a%2523b%23c" }] } as never);
  expect(await lookup("srelens-context:/kube/a%23b#c")).toEqual({ status:"ambiguous" });
});

it("gives a context's key for its display name or its pinned ID, and nothing for a stable ID", async () => {
  const { result } = renderHook(() => ["c", "srelens-context:/kube/a%23b#c", "/kube/a#b#c"].map(useContextId));
  await waitFor(() => expect(result.current[0]).toBeDefined());
  expect(result.current).toEqual(["/kube/a%23b#c", "/kube/a%23b#c", undefined]);
});
