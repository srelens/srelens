import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";

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
    { uid: null, name: "api-0", namespace: "team", values: { critical: "3" } },
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

it("keeps resolved cells while a watched snapshot is pending and loads only new rows", async () => {
  let complete!: (result: unknown) => void;
  resolve.mockResolvedValueOnce({ columns: [], cells: [
    { uid: null, name: "api", namespace: "team", values: { critical: "3" } },
  ] }).mockImplementationOnce(() => new Promise((done) => { complete = done; }));
  const first = { name: "api", namespace: "team" };
  const added = { name: "worker", namespace: "team" };
  const props = { plugins: [plugin], context: "prod", namespace: "team", kind: "apps/Deployment", rows: [first] };
  const view = renderHook((next) => useResolvedColumns(next), { initialProps: props });
  await waitFor(() => expect(view.result.current.columns[0].getValue?.(first)).toBe("3"));
  view.rerender({ ...props, rows: [first, added] });
  expect(view.result.current.columns[0].getValue?.(first)).toBe("3");
  expect(renderToStaticMarkup(<>{view.result.current.columns[0].render?.(added)}</>)).toContain("Loading");
  await act(async () => { complete({ columns: [], cells: [
    { uid: null, name: "api", namespace: "team", values: { critical: "4" } },
    { uid: null, name: "worker", namespace: "team", values: { critical: null } },
  ] }); });
  expect(view.result.current.columns[0].getValue?.(first)).toBe("4");
  expect(renderToStaticMarkup(<>{view.result.current.columns[0].render?.(added)}</>)).toContain("—");
});

it("sends no summary fields for join-only columns even when rows have large unused messages", async () => {
  resolve.mockResolvedValue({ columns: [], cells: [] });
  const rows = [{ name: "api", namespace: "team", message: "x".repeat(8_192) }];
  renderHook(() => useResolvedColumns({ plugins: [plugin], context: "prod", namespace: "team", kind: "apps/Deployment", rows }));
  await waitFor(() => expect(resolve).toHaveBeenCalledTimes(1));
  expect(resolve.mock.calls[0][5][0].row).toEqual({});
});

it("projects only the root key read by a row column", async () => {
  resolve.mockResolvedValue({ columns: [], cells: [] });
  const source = { ...plugin, manifest: { ...plugin.manifest, contributions: { tableColumns: [
    { id: "ready", title: "Ready", forKinds: ["apps/Deployment"],
      source: { jsonPath: ".status.ready" }, format: "text" },
  ] } } } as InstalledExtension;
  const rows = [{ name: "api", namespace: "team", status: { ready: "True" }, message: "x".repeat(8_192) }];
  renderHook(() => useResolvedColumns({ plugins: [source], context: "prod", namespace: "team", kind: "apps/Deployment", rows }));
  await waitFor(() => expect(resolve).toHaveBeenCalledTimes(1));
  expect(resolve.mock.calls[0][5][0].row).toEqual({ status: { ready: "True" } });
});

const flux = {
  enabled: true, revision: 4, manifest: { id: "org.srelens.flux", name: "Flux", contributions: {
    badges: [{ id: "flux-managed", forKinds: ["apps/Deployment"], rules: [
      { when: [{ jsonPath: ".metadata.labels['kustomize.toolkit.fluxcd.io/name']", present: true }],
        status: "healthy", label: "Flux", reason: ".metadata.labels['kustomize.toolkit.fluxcd.io/name']" }] }],
  } },
} as InstalledExtension;

it("puts an app's badges on built-in rows from the same one batch, always as words", async () => {
  resolve.mockResolvedValue({ columns: [], badges: flux.manifest.contributions.badges, cells: [
    { uid: null, name: "api", namespace: "team", values: {},
      badges: [{ id: "flux-managed", status: "healthy", label: "Flux", reason: "apps" }] },
    { uid: null, name: "plain", namespace: "team", values: {} },
    { uid: null, name: "gone", namespace: "team", values: {},
      badgeErrors: { "flux-managed": "row is not in the host's metadata read; refresh" } },
  ] });
  const rows = [{ name: "api", namespace: "team" }, { name: "plain", namespace: "team" }, { name: "gone", namespace: "team" }];
  const view = renderHook(() => useResolvedColumns({ plugins: [flux], context: "prod", namespace: "team", kind: "apps/Deployment", rows }));
  await waitFor(() => expect(view.result.current.columns[0]?.getValue?.(rows[0])).toBe("Flux"));
  expect(resolve).toHaveBeenCalledTimes(1);
  const column = view.result.current.columns[0];
  expect(column.header).toBe("Flux");
  expect(column.filterable).toBe(true);
  const html = (row: typeof rows[number]) => renderToStaticMarkup(<>{column.render?.(row)}</>);
  expect(html(rows[0])).toContain(">Flux<");
  expect(html(rows[0])).toContain("apps");
  // No rule held: no badge, which is an answer.
  expect(html(rows[1])).toContain("—");
  // The host could not say: that is not "not managed".
  expect(html(rows[2])).toContain("Couldn’t read");
  expect(html(rows[2])).not.toContain("—");
  expect(column.getValue?.(rows[1])).toBe("");
});

it("keeps an app's badge column apart from a table column whose id is `badges`", async () => {
  // `badges` is a valid column id; the badge column's key must not be one a
  // column id can produce, or the table treats the two as one column.
  const both = { ...flux, manifest: { ...flux.manifest, contributions: { ...flux.manifest.contributions,
    tableColumns: [{ id: "badges", title: "Badges", forKinds: ["apps/Deployment"], source: { jsonPath: ".name" }, format: "text" }],
  } } } as InstalledExtension;
  resolve.mockResolvedValue({ columns: both.manifest.contributions.tableColumns, badges: flux.manifest.contributions.badges, cells: [] });
  const rows = [{ name: "api", namespace: "team" }];
  const view = renderHook(() => useResolvedColumns({ plugins: [both], context: "prod", namespace: "team", kind: "apps/Deployment", rows }));
  await waitFor(() => expect(resolve).toHaveBeenCalledTimes(1));
  const keys = view.result.current.columns.map((column) => column.key);
  expect(keys).toHaveLength(2);
  expect(new Set(keys).size).toBe(2);
});

it("asks for nothing on a kind no badge names", async () => {
  const rows = [{ name: "db", namespace: "team" }];
  const view = renderHook(() => useResolvedColumns({ plugins: [flux], context: "prod", namespace: "team", kind: "apps/StatefulSet", rows }));
  await act(async () => {});
  expect(resolve).not.toHaveBeenCalled();
  expect(view.result.current.columns).toHaveLength(0);
});

it("shows one cell's resolver error without hiding another row's value", async () => {
  resolve.mockResolvedValue({ columns: plugin.manifest.contributions.tableColumns, cells: [
    { uid: null, name: "api", namespace: "team", values: { critical: null },
      errors: { critical: "matched multiple joined resources" } },
    { uid: null, name: "worker", namespace: "team", values: { critical: "7" } },
  ] });
  const rows = [{ name: "api", namespace: "team" }, { name: "worker", namespace: "team" }];
  const view = renderHook(() => useResolvedColumns({ plugins: [plugin], context: "prod", namespace: "team", kind: "apps/Deployment", rows }));
  await waitFor(() => expect(view.result.current.columns[0].getValue?.(rows[1])).toBe("7"));
  expect(renderToStaticMarkup(<>{view.result.current.columns[0].render?.(rows[0])}</>))
    .toContain("matched multiple joined resources");
  expect(view.result.current.columns[0].getValue?.(rows[0])).toBe("");
  expect(view.result.current.errors).toEqual([]);
});
