import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ExtensionStreamHandlers, ExtensionStreamRequest } from "@srelens/core";

const core = vi.hoisted(() => ({
  tauri: true,
  readExtension: vi.fn(),
  resolveExtensionColumns: vi.fn(),
  listContexts: vi.fn(),
  openExtensionView: vi.fn(),
}));
vi.mock("@srelens/core", async (original) => ({
  ...(await original<typeof import("@srelens/core")>()),
  isTauri: () => core.tauri,
  readExtension: core.readExtension,
  resolveExtensionColumns: core.resolveExtensionColumns,
  listContexts: core.listContexts,
  openExtensionView: core.openExtensionView,
}));

import { ExtensionResults } from "./ExtensionResults";

let handlers: ExtensionStreamHandlers[] = [];
let requests: ExtensionStreamRequest[] = [];
const rows = (...names: string[]) => ({ items: names.map((name) => ({ name, namespace: "team", age: "1d", columns: [] })) });

beforeEach(() => {
  core.tauri = true;
  handlers = [];
  requests = [];
  for (const mock of [core.readExtension, core.resolveExtensionColumns, core.listContexts, core.openExtensionView]) mock.mockReset();
  core.listContexts.mockResolvedValue({ contexts: [] });
  core.resolveExtensionColumns.mockResolvedValue({ columns: [], cells: [] });
  core.openExtensionView.mockImplementation((app: string, label: string) => ({
    view: `${app}/${label}`,
    close: vi.fn(async () => {}),
    open: async (request: ExtensionStreamRequest, h: ExtensionStreamHandlers) => {
      requests.push(request);
      handlers.push(h);
      return { stream: "s-1", cancel: vi.fn(async () => {}) };
    },
  }));
});
afterEach(() => cleanup());

const plugin = {
  manifest: {
    id: "org.test.gitops", name: "GitOps", version: "0.1.0", permissions: ["k8s.listCustomResource"],
    capabilities: [{ name: "list", target: "k8s.listCustomResource", arguments: { kind: "Application", group: "argoproj.io" }, inputs: ["context", "namespace"] }],
    contributions: { pages: [{ id: "apps", title: "Applications", capability: "list" }], detailTabs: [], detailLinks: [] },
  },
  enabled: true, revision: 1, settings: {},
} as never;

const emit = (data: unknown, seq = 1) => act(() => handlers[0].onData(data, seq));

describe("a live app page (#566)", () => {
  it("reads again in place when the watch reports a change", async () => {
    core.readExtension.mockResolvedValueOnce(rows("web"));
    render(<ExtensionResults plugin={plugin} capability="list" context="kind-demo" namespace="team" />);
    expect(await screen.findByText("web")).toBeTruthy();
    await waitFor(() => expect(handlers).toHaveLength(1));
    expect(requests[0]).toMatchObject({ context: "kind-demo", namespace: "team", source: { kind: "watch", capability: "list" } });
    emit({ event: "synced" });
    expect(screen.getByText("Live")).toBeTruthy();
    let finish!: (value: unknown) => void;
    core.readExtension.mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    emit({ event: "changed" }, 2);
    // The list stays on screen while it is read again; no "Refreshing" flash.
    expect(screen.getByText("web")).toBeTruthy();
    expect(screen.queryByText("Refreshing resources…")).toBeNull();
    await act(async () => { finish(rows("web", "api")); });
    expect(screen.getByText("api")).toBeTruthy();
  });

  it("marks the list out of date while reconnecting, and reads afresh after", async () => {
    core.readExtension.mockResolvedValue(rows("web"));
    const { container } = render(<ExtensionResults plugin={plugin} capability="list" context="kind-demo" namespace="team" />);
    await screen.findByText("web");
    await waitFor(() => expect(handlers).toHaveLength(1));
    emit({ event: "synced" });
    const reads = core.readExtension.mock.calls.length;
    emit({ event: "reconnecting", message: "connection reset" }, 2);
    expect(screen.getByText("Reconnecting…")).toBeTruthy();
    expect(screen.getByText(/Reconnecting to the cluster \(connection reset\)\. The list below may be out of date/)).toBeTruthy();
    expect(container.querySelector("[data-stale]")).not.toBeNull();
    core.readExtension.mockResolvedValue(rows("web", "db"));
    emit({ event: "synced" }, 3);
    expect(await screen.findByText("db")).toBeTruthy();
    expect(core.readExtension.mock.calls.length).toBe(reads + 1);
    expect(screen.queryByText(/may be out of date/)).toBeNull();
    expect(container.querySelector("[data-stale]")).toBeNull();
  });

  it("says why it is not live when the host ends the watch", async () => {
    core.readExtension.mockResolvedValue(rows("web"));
    render(<ExtensionResults plugin={plugin} capability="list" context="kind-demo" namespace="team" />);
    await screen.findByText("web");
    await waitFor(() => expect(handlers).toHaveLength(1));
    act(() => handlers[0].onEnd?.({ type: "error", code: "rateLimited", message: "App org.test.gitops sent more than 50 stream messages per second" }));
    expect(screen.getByText("Not live")).toBeTruthy();
    expect(screen.getByText(/Live updates stopped: The host stopped the stream: App org\.test\.gitops sent more than 50/)).toBeTruthy();
    expect(screen.getByText("web")).toBeTruthy();
  });

  it("stays a read-on-Refresh list on the web, and says so", async () => {
    core.tauri = false;
    core.readExtension.mockResolvedValue(rows("web"));
    render(<ExtensionResults plugin={plugin} capability="list" context="kind-demo" namespace="team" />);
    await screen.findByText("web");
    expect(screen.getByText("Not live")).toBeTruthy();
    expect(core.openExtensionView).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "Refresh" })).toBeTruthy();
  });
});

describe("an app limited to chosen clusters (Settings → Apps)", () => {
  const limited = {
    ...(plugin as object),
    manifest: {
      ...(plugin as { manifest: object }).manifest,
      contributions: { pages: [{ id: "apps", title: "Applications", capability: "list" }], detailTabs: [], detailLinks: [],
        tableColumns: [{ id: "sync", title: "Sync", forKinds: ["argoproj.io/Application"], source: { jsonPath: ".name" }, format: "text" }] },
    },
    contexts: ["/kube/a%23b#c"],
  } as never;

  it("joins its columns on its own page, which asks the host by pinned ID (#695)", async () => {
    // `/kube/a` declaring `b#c` and `/kube/a#b` declaring `c` share the stable ID `/kube/a#b#c` (#623).
    core.listContexts.mockResolvedValue({ contexts: [
      { name: "b#c", stableId: "/kube/a#b#c", key: "/kube/a#b%23c", pinnedId: "srelens-context:/kube/a#b%23c" },
      { name: "c", stableId: "/kube/a#b#c", key: "/kube/a%23b#c", pinnedId: "srelens-context:/kube/a%23b#c" },
    ] });
    core.readExtension.mockResolvedValue(rows("web"));
    render(<ExtensionResults plugin={limited} capability="list" context="srelens-context:/kube/a%23b#c" namespace="team" />);
    await screen.findByText("web");
    await waitFor(() => expect(core.resolveExtensionColumns).toHaveBeenCalledWith(
      "org.test.gitops", 1, "srelens-context:/kube/a%23b#c", "team", "argoproj.io/Application", expect.any(Array)));
  });
});
