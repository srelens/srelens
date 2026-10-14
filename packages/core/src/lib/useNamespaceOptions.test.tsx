import { describe, expect, it, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";

const { listNamespaces, listContexts } = vi.hoisted(() => ({ listNamespaces: vi.fn(), listContexts: vi.fn() }));
vi.mock("./workloads", () => ({ listNamespaces }));
vi.mock("./clusters", () => ({ listContexts }));

import { useNamespaceOptions } from "./useNamespaceOptions";

beforeEach(() => {
  listNamespaces.mockReset();
  listContexts.mockReset().mockResolvedValue({ contexts: [] });
});

/**
 * A namespace collection selects by label (#881), so the picker needs the
 * labels the listing already carries, and needs to know whether the listing
 * is every namespace or only what restricted credentials could see.
 */
describe("useNamespaceOptions, for namespace collections", () => {
  it("hands over each namespace's labels, and says the listing is complete", async () => {
    listNamespaces.mockResolvedValue({
      namespaces: ["prod-eu", "dev"],
      summaries: [
        { name: "prod-eu", phase: "Active", labels: { env: "prod" }, age: "1d" },
        { name: "dev", phase: "Active", labels: {}, age: "1d" },
      ],
    });
    const { result } = renderHook(() => useNamespaceOptions("c", []));
    expect(result.current.items).toBeNull();
    await waitFor(() => expect(result.current.items).not.toBeNull());
    expect(result.current.items).toEqual([
      { name: "prod-eu", labels: { env: "prod" } },
      { name: "dev", labels: {} },
    ]);
    expect(result.current.complete).toBe(true);
  });

  it("says the listing is partial when restricted credentials scoped it to one namespace", async () => {
    listNamespaces.mockResolvedValue({ error: 'namespaces is forbidden: User "system:serviceaccount:team-a:bot" cannot list' });
    listContexts.mockResolvedValue({ contexts: [{ name: "c", namespace: "team-a" }] });
    const { result } = renderHook(() => useNamespaceOptions("c", []));
    await waitFor(() => expect(result.current.namespaces).toEqual(["team-a"]));
    expect(result.current.items).toEqual([{ name: "team-a" }]);
    expect(result.current.complete).toBe(false);
  });

  it("says the listing is partial, and empty, when it failed", async () => {
    listNamespaces.mockResolvedValue({ error: "connection refused" });
    const { result } = renderHook(() => useNamespaceOptions("c", []));
    await waitFor(() => expect(result.current.items).toEqual([]));
    expect(result.current.complete).toBe(false);
    expect(result.current.error).toBe("connection refused");
  });
});
