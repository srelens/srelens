// Performance budgets for the extension platform's client half (#581). The
// host's half — loading the inventory, a call's own overhead, resolving 1,000
// rows, closing a view on the host — is `crates/registry/src/extensions/budget_tests.rs`,
// which explains the scheme: counts are held exactly; times are held to a
// ceiling far above the roadmap's target, because CI runs this suite under
// coverage on a shared runner, and every measurement is reported against the
// target. With SRELENS_PERF_REPORT_DIR set, each is written there as
// `ts-<name>.json`, and CI uploads the directory.
import { act, cleanup, render, renderHook } from "@testing-library/react";
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ExtensionInventory, ExtensionManifest, InstalledExtension } from "@srelens/core";
import argocdExample from "../../../../examples/extensions/argocd.json";
import fluxExample from "../../../../examples/extensions/flux.json";

// A fake host behind the real transport: the real `openExtensionView`,
// `listExtensions` and `resolveExtensionColumns` run, and only the bridge is
// replaced, so what is counted is what the client actually asks of the host.
const host = vi.hoisted(() => ({
  inventory: undefined as unknown,
  listeners: new Map<string, (payload: unknown) => void>(),
  streams: new Map<string, { view: string; channel: string }>(),
  opened: 0,
  closedViews: [] as string[],
  cancelled: [] as string[],
  resolves: [] as Array<{ id: string; uids: unknown[] }>,
}));
vi.mock("@srelens/core/transport", async (original) => ({
  ...(await original<typeof import("@srelens/core/transport")>()),
  subscribe: async (channel: string, handler: (payload: unknown) => void) => {
    host.listeners.set(channel, handler);
    return () => { host.listeners.delete(channel); };
  },
  invokeCapability: async (id: string, input: { id: string; uids: unknown[] }) => {
    if (id === "extensions.list") return host.inventory;
    if (id === "extensions.resolveColumns") {
      host.resolves.push({ id: input.id, uids: input.uids });
      return { columns: [], badges: [], cells: [] };
    }
    throw new Error(`the fake host serves no ${id}`);
  },
  invokeCommand: async (command: string, args: Record<string, never>) => {
    const end = (stream: string, reason: string) => {
      const opened = host.streams.get(stream)!;
      host.streams.delete(stream);
      host.listeners.get(opened.channel)?.({ type: "close", stream, reason });
    };
    if (command === "extension_stream_open") {
      const { view, channel } = args.input as { view: string; channel: string };
      const stream = `s-${++host.opened}`;
      host.streams.set(stream, { view, channel });
      host.listeners.get(channel)?.({ type: "open", stream });
      host.listeners.get(channel)?.({ type: "data", stream, seq: 1, data: { event: "synced" } });
      return { stream, channel };
    }
    if (command === "extension_stream_close_view") {
      host.closedViews.push(args.view);
      const mine = [...host.streams].filter(([, s]) => s.view === args.view).map(([id]) => id);
      for (const stream of mine) end(stream, "viewClosed");
      return mine.length;
    }
    if (command === "extension_stream_cancel") {
      host.cancelled.push(args.stream);
      if (host.streams.has(args.stream)) end(args.stream, "cancelled");
      return true;
    }
    throw new Error(`the fake host serves no ${command}`);
  },
}));
vi.mock("@srelens/core", async (original) => ({
  ...(await original<typeof import("@srelens/core")>()),
  isTauri: () => true,
}));

import { appNavigation } from "./appNavigation";
import { useExtensions } from "./inventoryStore";
import { LiveReaders, useLiveReaders } from "./liveReaders";
import { useResolvedColumns } from "./useResolvedColumns";

// ---------------------------------------------------------------------------
// Measuring and reporting
// ---------------------------------------------------------------------------

const REPORT_DIR = process.env.SRELENS_PERF_REPORT_DIR;

function report(name: string, entry: Record<string, unknown>) {
  if (!REPORT_DIR) return;
  mkdirSync(REPORT_DIR, { recursive: true });
  const out = { suite: "@srelens/ui-next", name, node: process.version, commit: process.env.GITHUB_SHA, ...entry };
  writeFileSync(join(REPORT_DIR, `ts-${name}.json`), `${JSON.stringify(out, null, 2)}\n`);
}

type Budget = { name: string; what: string; targetMs: number | null; ceilingMs: number };

const round = (ms: number) => Math.round(ms * 1000) / 1000;

/** Report `samples` against `budget`, then hold their median to the ceiling. */
function hold(budget: Budget, samples: number[], detail: Record<string, unknown>) {
  const sorted = [...samples].sort((a, b) => a - b);
  const median = sorted[Math.floor(sorted.length / 2)];
  report(budget.name, {
    kind: "timing", what: budget.what, unit: "ms", runs: sorted.length,
    medianMs: round(median), minMs: round(sorted[0]), maxMs: round(sorted[sorted.length - 1]),
    targetMs: budget.targetMs, ceilingMs: budget.ceilingMs,
    withinTarget: budget.targetMs === null ? null : median <= budget.targetMs, detail,
  });
  // The ceiling is far above the target on purpose: a path this far over it
  // has regressed, so find the change rather than raise the number.
  expect(median, `${budget.what}: median of ${sorted.length} runs`).toBeLessThanOrEqual(budget.ceilingMs);
}

/** `run` `warmup` times untimed, then `runs` times timed. */
async function time(warmup: number, runs: number, run: () => unknown): Promise<number[]> {
  for (let i = 0; i < warmup; i++) await run();
  const samples: number[] = [];
  for (let i = 0; i < runs; i++) {
    const started = performance.now();
    await run();
    samples.push(performance.now() - started);
  }
  return samples;
}

// ---------------------------------------------------------------------------
// Fifty apps
// ---------------------------------------------------------------------------

/** As the Rust budgets: the two shipped examples, Flux — the largest — for one in ten. */
const APPS = 50;
const FLUX_APPS = 5;
const FLUX = fluxExample as unknown as ExtensionManifest;
const ARGO = argocdExample as unknown as ExtensionManifest;

function installed(manifest: ExtensionManifest, revision: number, id = manifest.id): InstalledExtension {
  return {
    manifest: { ...manifest, id }, enabled: true, revision, grants: manifest.permissions,
    settings: {}, source: "local", installedAt: 1_767_225_600, history: [],
  };
}

function fiftyApps(): InstalledExtension[] {
  return Array.from({ length: APPS }, (_, index) => index < FLUX_APPS
    ? installed({ ...FLUX, name: `Flux ${index}` }, index + 1, `org.example.flux-${index}`)
    : installed({ ...ARGO, name: `Argo CD ${index}` }, index + 1, `org.example.argocd-${index}`));
}

const flush = () => act(async () => { for (let i = 0; i < 5; i++) await Promise.resolve(); });

beforeEach(() => {
  host.listeners.clear();
  host.streams.clear();
  host.opened = 0;
  host.closedViews = [];
  host.cancelled = [];
  host.resolves = [];
  host.inventory = { schemaVersion: 1, nextRevision: APPS + 1, plugins: fiftyApps() } satisfies ExtensionInventory;
});
afterEach(() => cleanup());

describe("extension performance budgets (#581)", () => {
  it("builds the sidebar's apps from fifty installed apps within budget", async () => {
    const plugins = fiftyApps();
    const nav = appNavigation(plugins, "prod-key");
    // Every app with a page, each under its own node, its grouped pages nested.
    expect(nav?.children).toHaveLength(APPS);
    const flux = nav!.children![0];
    expect(flux.label).toBe("Flux 0");
    expect(flux.children!.map((node) => node.label)).toEqual([
      "Overview", "Kustomizations", "Helm", "Sources", "Image Automation", "Notifications",
    ]);
    expect(flux.children![3].children).toHaveLength(5);
    const pages = FLUX_APPS * FLUX.contributions.pages.length + (APPS - FLUX_APPS) * ARGO.contributions.pages.length;
    const samples = await time(5, 50, () => appNavigation(plugins, "prod-key"));
    hold({
      name: "navigation-build-50-apps",
      what: "the sidebar's Apps group from 50 installed apps",
      targetMs: 10,
      ceilingMs: 50,
    }, samples, { apps: APPS, pages });
  });

  it("takes an inventory of fifty apps from the host to a ready app list within budget", async () => {
    const load = async () => {
      const view = renderHook(() => useExtensions());
      await flush();
      expect(view.result.current.status).toBe("ready");
      expect(view.result.current.data?.plugins).toHaveLength(APPS);
      // The last consumer leaving resets the store, so every run loads afresh.
      view.unmount();
    };
    const samples = await time(3, 20, load);
    hold({
      name: "inventory-store-load-50-apps",
      what: "the app list's own part of loading 50 apps: from the host's answer to a ready list",
      targetMs: 50,
      ceilingMs: 250,
    }, samples, { apps: APPS, inventoryBytes: JSON.stringify(host.inventory).length });
  });

  it("releases every watch and stream a view opened when the view closes", async () => {
    // A Flux page following three parts on two readers, over a Deployment table
    // two other apps add joined columns to.
    const flux = installed(FLUX, 7);
    const joins = (id: string, readers: string[]): InstalledExtension => installed({
      ...ARGO, id, name: id,
      contributions: {
        ...ARGO.contributions,
        joins: readers.map((capability) => ({ id: capability, capability, match: { name: true } })),
        tableColumns: readers.map((capability) => ({
          id: capability, title: capability, forKinds: ["apps/Deployment"],
          source: { join: capability, jsonPath: ".status.phase" }, format: "text",
        })),
      },
    }, 3, id);
    const security = joins("org.example.security", ["vulnerabilityreports", "configauditreports"]);
    const argo = joins("org.example.argo", ["applications"]);
    const rows = Array.from({ length: 1_000 }, (_, index) => ({ uid: `uid-${index}`, name: `api-${index}`, namespace: "team" }));
    function Part({ capability }: { capability: string }) {
      useLiveReaders({ plugin: flux, capabilities: [capability], context: "kind-demo", namespace: "team", label: "part", onChange: () => {} });
      return null;
    }
    function Table() {
      useResolvedColumns({ plugins: [security, argo], context: "kind-demo", namespace: "team", kind: "apps/Deployment", rows });
      return null;
    }
    const page = render(
      <LiveReaders plugin={flux} label="page:overview">
        <Part capability="kustomizations" />
        <Part capability="kustomizations" />
        <Part capability="helmreleases" />
        <Table />
      </LiveReaders>,
    );
    await flush();
    // One watch per reader per app, however many parts show it: 2 + 2 + 1.
    const views = new Set([...host.streams.values()].map((stream) => stream.view));
    expect(host.streams.size).toBe(5);
    expect(views.size).toBe(3);
    expect(host.listeners.size).toBe(5);
    // One resolve per contributing app for the whole table, never per row —
    // and one more once its watch has listed: the host drops the reader's
    // snapshot on `synced`, so that second read closes the gap between the
    // table's first read and the watch starting. Two per app, whatever the rows.
    expect(host.resolves.map((call) => [call.id, call.uids.length]).sort()).toEqual([
      ["org.example.argo", 1_000], ["org.example.argo", 1_000],
      ["org.example.security", 1_000], ["org.example.security", 1_000],
    ]);

    page.unmount();
    await flush();
    // Every view the page opened is closed on the host, once; nothing is left
    // open there, and nothing is left listening here.
    expect([...host.closedViews].sort()).toEqual([...views].sort());
    expect(host.streams.size).toBe(0);
    expect(host.listeners.size).toBe(0);
    report("client-view-close-releases", {
      kind: "count",
      what: "streams open on the host and channel listeners left in the client after a view with three apps' watches unmounts",
      streamsOpened: host.opened,
      viewsOpened: views.size,
      viewsClosed: host.closedViews.length,
      streamsCancelledOneByOne: host.cancelled.length,
      streamsOpenAfterClose: host.streams.size,
      listenersAfterClose: host.listeners.size,
      resolveCalls: host.resolves.length,
      rows: rows.length,
    });
  });
});
