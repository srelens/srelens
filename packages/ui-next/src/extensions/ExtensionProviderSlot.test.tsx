import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";

vi.mock("@srelens/core", async (original) => ({
  ...(await original<typeof import("@srelens/core")>()),
  listContexts: vi.fn(),
  queryExtensionProvider: vi.fn(),
}));
vi.mock("./inventoryStore", async (original) => ({
  ...(await original<typeof import("./inventoryStore")>()),
  useExtensions: vi.fn(),
}));

if (!("ResizeObserver" in globalThis)) {
  (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}

import { listContexts, queryExtensionProvider, type ExtensionProviderResult, type InstalledExtension } from "@srelens/core";
import { useExtensions } from "./inventoryStore";
import { ExtensionProviderSlot } from "./ExtensionProviderSlot";

const plugin = {
  manifest: {
    id: "org.example.observability", name: "Observability", version: "1.0.0", srelensApiVersion: "^0.6",
    kind: "declarative", permissions: [], capabilities: [],
    contributions: {
      pages: [], detailTabs: [], detailLinks: [],
      metricProviders: [
        { id: "cpu", title: "CPU", capability: "prom", language: "promql", forKinds: ["apps/Deployment"], unit: "cores", query: "up" },
        { id: "podCpu", title: "Pod CPU", capability: "prom", language: "promql", forKinds: ["/Pod"], unit: "cores", query: "up" },
      ],
      traceProviders: [
        { id: "traces", title: "Recent traces", capability: "tempo", language: "traceql", forKinds: ["apps/Deployment"], query: "{}" },
      ],
    },
  },
  enabled: true, revision: 7, grants: [], settings: {}, source: "local", installedAt: 0, history: [],
} as unknown as InstalledExtension;

const deployment = {
  apiVersion: "apps/v1", kind: "Deployment",
  metadata: { name: "web", namespace: "team", uid: "u-1", resourceVersion: "5" },
};

const START = Date.UTC(2026, 8, 27, 9, 0, 0);
const chart: ExtensionProviderResult = {
  kind: "metrics",
  chart: {
    label: "CPU", unit: "cores", range: { start: START, end: START + 3 * 15_000 },
    times: [START, START + 15_000, START + 30_000, START + 45_000],
    series: [
      { name: "pod=\"web-1\"", values: [0.25, 0.5, null, 0.75] },
      { name: "pod=\"web-2\"", values: [0.1, 0.2, 0.3, 0.4] },
    ],
  },
};
const traces: ExtensionProviderResult = {
  kind: "traces",
  truncated: true,
  traces: [
    { traceId: "4bf92f3577b34da6a3ce929d0e0e4736", rootService: "checkout", rootName: "<script>alert(1)</script>", start: START, durationMs: 42 },
    { traceId: "00f067aa0ba902b7" },
  ],
};

function answer(byProvider: Record<string, ExtensionProviderResult | Error>) {
  vi.mocked(queryExtensionProvider).mockImplementation(async (query) => {
    const found = byProvider[query.provider];
    if (found instanceof Error) throw found;
    return found;
  });
}

beforeEach(() => {
  vi.mocked(queryExtensionProvider).mockReset();
  vi.mocked(listContexts).mockResolvedValue({ contexts: [{ name: "prod-eu", key: "prod" }] } as never);
  vi.mocked(useExtensions).mockReturnValue({ status: "ready", data: { schemaVersion: 1, nextRevision: 9, plugins: [plugin] }, reload: vi.fn() } as never);
});

describe("provider panels on an overview (#569)", () => {
  it("asks each provider declared for the kind, for this resource, over the last hour", async () => {
    answer({ cpu: chart, traces });
    render(<ExtensionProviderSlot context="prod-eu" resource={deployment} />);
    await waitFor(() => expect(queryExtensionProvider).toHaveBeenCalledTimes(2));
    expect(vi.mocked(queryExtensionProvider).mock.calls.map(([query]) => query)).toEqual([
      { id: "org.example.observability", revision: 7, provider: "cpu", context: "prod-eu", namespace: "team", resourceKind: "apps/Deployment", name: "web", rangeSeconds: 3600 },
      { id: "org.example.observability", revision: 7, provider: "traces", context: "prod-eu", namespace: "team", resourceKind: "apps/Deployment", name: "web", rangeSeconds: 3600 },
    ]);
  });

  it("draws a metric provider's answer as the host's chart, naming the app it came from", async () => {
    answer({ cpu: chart, traces });
    render(<ExtensionProviderSlot context="prod-eu" resource={deployment} />);
    const panel = await screen.findByRole("region", { name: "CPU from Observability" });
    const plot = await within(panel).findByRole("img");
    expect(plot.getAttribute("aria-label")).toContain("CPU");
    expect(panel.textContent).toContain("pod=\"web-1\"");
    expect(panel.textContent).toContain("pod=\"web-2\"");
    expect(panel.textContent).toContain("Observability");
  });

  it("lists a trace provider's traces as text, in the host's table, and says the list was cut", async () => {
    answer({ cpu: chart, traces });
    const { container } = render(<ExtensionProviderSlot context="prod-eu" resource={deployment} />);
    const panel = await screen.findByRole("region", { name: "Recent traces from Observability" });
    await within(panel).findByText("4bf92f3577b34da6a3ce929d0e0e4736");
    expect(within(panel).getByText("checkout")).toBeTruthy();
    expect(within(panel).getByText("42 ms")).toBeTruthy();
    // What a provider answered is data: a root span called `<script>` is its name, shown.
    expect(within(panel).getByText("<script>alert(1)</script>")).toBeTruthy();
    expect(container.querySelector("script")).toBeNull();
    expect(panel.textContent).toContain("The search found more traces than the 50 listed");
  });

  it("asks again over the range the reader picks, and on Refresh", async () => {
    answer({ cpu: chart, traces });
    render(<ExtensionProviderSlot context="prod-eu" resource={deployment} />);
    await waitFor(() => expect(queryExtensionProvider).toHaveBeenCalledTimes(2));
    fireEvent.change(screen.getByRole("combobox", { name: "range" }), { target: { value: "21600" } });
    await waitFor(() => expect(queryExtensionProvider).toHaveBeenCalledTimes(4));
    expect(vi.mocked(queryExtensionProvider).mock.calls.slice(2).map(([query]) => query.rangeSeconds)).toEqual([21600, 21600]);
    fireEvent.click(screen.getByRole("button", { name: "Refresh app metrics and traces" }));
    await waitFor(() => expect(queryExtensionProvider).toHaveBeenCalledTimes(6));
  });

  it("says what failed and offers a retry, never an empty chart", async () => {
    answer({ cpu: new Error("The query returned 12 series; a chart draws at most 8"), traces });
    render(<ExtensionProviderSlot context="prod-eu" resource={deployment} />);
    const panel = await screen.findByRole("region", { name: "CPU from Observability" });
    expect((await within(panel).findAllByText(/at most 8/)).length).toBeGreaterThan(0);
    expect(panel.textContent).not.toContain("No data reported");
    answer({ cpu: chart, traces });
    await act(async () => {
      fireEvent.click(within(panel).getByRole("button", { name: /Retry/ }));
    });
    await within(panel).findByRole("img");
  });

  it("names the panel whose query matched nothing, rather than a bare 'no data'", async () => {
    const drawn = chart as Extract<ExtensionProviderResult, { kind: "metrics" }>;
    const quiet: ExtensionProviderResult = { kind: "metrics", chart: { ...drawn.chart, series: [{ name: "CPU", values: [null, null, null, null] }] } };
    answer({ cpu: quiet, traces });
    render(<ExtensionProviderSlot context="prod-eu" resource={deployment} />);
    const panel = await screen.findByRole("region", { name: "CPU from Observability" });
    expect((await within(panel).findByText("No data reported for CPU")).textContent).toBe("No data reported for CPU");
    expect(within(panel).queryByRole("img")).toBeNull();
  });

  it("draws nothing, and asks nothing, for a kind no provider is for", () => {
    const service = { apiVersion: "v1", kind: "Service", metadata: { name: "web", namespace: "team" } };
    const { container } = render(<ExtensionProviderSlot context="prod-eu" resource={service} />);
    expect(container.textContent).toBe("");
    expect(queryExtensionProvider).not.toHaveBeenCalled();
  });

  it("offers nothing from an app that is off, quarantined or not enabled for this cluster", async () => {
    vi.mocked(useExtensions).mockReturnValue({
      status: "ready",
      data: { schemaVersion: 1, nextRevision: 9, plugins: [
        { ...plugin, enabled: false },
        { ...plugin, quarantined: "bad" },
        { ...plugin, contexts: ["staging"] },
      ] },
      reload: vi.fn(),
    } as never);
    const { container } = render(<ExtensionProviderSlot context="prod-eu" resource={deployment} />);
    await act(async () => {});
    expect(container.textContent).toBe("");
    expect(queryExtensionProvider).not.toHaveBeenCalled();
  });
});
