import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";

const { resolve } = vi.hoisted(() => ({ resolve: vi.fn() }));
vi.mock("@srelens/core", async (original) => ({
  ...(await original<typeof import("@srelens/core")>()),
  resolveExtensionColumns: resolve,
}));

import { useResolvedColumns } from "./useResolvedColumns";
import type { InstalledExtension } from "@srelens/core";

const plugin = {
  enabled: true, revision: 2, manifest: { id: "org.example.security", name: "Security", contributions: {
    tableColumns: [{ id: "critical", title: "Critical CVEs", forKinds: ["apps/Deployment"],
      source: { join: "vulns", jsonPath: ".report.summary.criticalCount" }, format: "number", sortable: true }],
  } },
} as InstalledExtension;

beforeEach(() => { vi.clearAllMocks(); });

it("resolves 1,000 deployment rows with one call, then removes the column on disable", async () => {
  resolve.mockResolvedValue({ columns: plugin.manifest.contributions.tableColumns, cells: [
    { name: "api-0", namespace: "team", values: { critical: "3" } },
  ] });
  const rows = Array.from({ length: 1_000 }, (_, index) => ({ name: `api-${index}`, namespace: "team" }));
  const props = { plugins: [plugin], context: "prod", contextId: "context-key", namespace: "team", kind: "apps/Deployment", rows };
  const view = renderHook((next) => useResolvedColumns(next), { initialProps: props });
  await waitFor(() => expect(view.result.current.columns[0].getValue?.(rows[0])).toBe("3"));
  expect(resolve).toHaveBeenCalledTimes(1);
  expect(resolve.mock.calls[0][5]).toHaveLength(1_000);
  expect(view.result.current.columns[0].getValue?.(rows[0])).toBe("3");
  expect(view.result.current.columns[0].getValue?.(rows[1])).toBe("");
  view.rerender({ ...props, plugins: [{ ...plugin, enabled: false }] });
  expect(view.result.current.columns).toHaveLength(0);
});

it("keeps a failed read explicit and retries one batch", async () => {
  resolve.mockRejectedValueOnce(new Error("RBAC denied")).mockResolvedValueOnce({ columns: [], cells: [] });
  const rows = [{ name: "api", namespace: "team" }];
  const view = renderHook(() => useResolvedColumns({ plugins: [plugin], context: "prod", namespace: "team", kind: "apps/Deployment", rows }));
  await waitFor(() => expect(view.result.current.errors[0]?.message).toBe("RBAC denied"));
  act(() => view.result.current.reload());
  await waitFor(() => expect(resolve).toHaveBeenCalledTimes(2));
});
