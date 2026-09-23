import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { it, expect, vi, beforeEach } from "vitest";
vi.mock("@srelens/core", async (original) => ({
  ...(await original<typeof import("@srelens/core")>()),
  listCrds: vi.fn(),
  readExtension: vi.fn(),
  listNamespaces: vi.fn(),
}));
import {
  listCrds,
  readExtension,
  listNamespaces,
  type InstalledExtension,
} from "@srelens/core";
import { ExtensionWorkspace, DONUT_COLORS, donutBackground, EMPTY_DONUT } from "./ExtensionWorkspace";

it("draws Unknown in a readable ink, never the hairline an empty ring uses", () => {
  // `--rule` is the subtle hairline role: an all-Unknown ring drawn in it
  // looked exactly like an empty one.
  expect(DONUT_COLORS.unknown).toMatch(/^var\(--ink-faint\b/);
  expect(DONUT_COLORS.unknown).not.toContain("--rule");
  expect(EMPTY_DONUT).toContain("--rule");
  expect(new Set(Object.values(DONUT_COLORS)).size).toBe(6);
  const allUnknown = donutBackground({ healthy: 0, warning: 0, error: 0, progressing: 0, suspended: 0, unknown: 4 });
  expect(allUnknown).toContain(DONUT_COLORS.unknown);
  expect(allUnknown).not.toBe(EMPTY_DONUT);
  expect(donutBackground({ healthy: 0, warning: 0, error: 0, progressing: 0, suspended: 0, unknown: 0 })).toBe(EMPTY_DONUT);
});

/** The ring's stops as `{ colour, from, to }`, in order. */
function ringStops(ring: string) {
  return ring.replace(/^conic-gradient\(/, "").replace(/\)$/, "").split(/(?<=%),/).map((stop) => {
    const match = /^(.*) (-?[\d.]+)% (-?[\d.]+)%$/.exec(stop.trim())!;
    return { colour: match[1], from: Number(match[2]), to: Number(match[3]) };
  });
}

it.each([
  ["1 in 100", 99],
  ["1 in 1000", 999],
])("never lets a rare status vanish from the ring: %s", (_name, healthy) => {
  // One error among many rows is exactly what an operator scans the ring
  // for; a gap wider than its share used to leave it a zero-width stop.
  const stops = ringStops(donutBackground({ healthy, warning: 0, error: 1, progressing: 0, suspended: 0, unknown: 0 }));
  const error = stops.findIndex((stop) => stop.colour === DONUT_COLORS.error);
  expect(error).toBeGreaterThanOrEqual(0);
  expect(stops[error].to - stops[error].from).toBeGreaterThan(1);
  // A mark the eye finds: its colour is wider than the gap beside it.
  const gap = stops[error + 1] ? stops[error + 1].to - stops[error + 1].from : 0;
  expect(stops[error].to - stops[error].from).toBeGreaterThanOrEqual(3 * gap - 1e-9);
  // And it keeps its gap, so it is not merged into its neighbour.
  expect(stops[error + 1]?.colour ?? stops[0].colour).toMatch(/^var\(--surface/);
  // The ring still closes: every stop in order, ending at 100%.
  stops.forEach((stop, i) => {
    expect(stop.to).toBeGreaterThanOrEqual(stop.from);
    if (i) expect(stop.from).toBeCloseTo(stops[i - 1].to, 6);
  });
  expect(stops[0].from).toBe(0);
  expect(stops.at(-1)!.to).toBeCloseTo(100, 6);
  // Every non-zero colour segment is visible.
  for (const stop of stops.filter((s) => !s.colour.startsWith("var(--surface"))) expect(stop.to - stop.from).toBeGreaterThan(1);
});

it("keeps every one of six rare statuses visible beside a dominant one", () => {
  const stops = ringStops(donutBackground({ healthy: 10_000, warning: 1, error: 1, progressing: 1, suspended: 1, unknown: 1 }));
  const colours = stops.filter((stop) => !stop.colour.startsWith("var(--surface"));
  expect(colours.map((stop) => stop.colour)).toEqual(Object.values(DONUT_COLORS));
  for (const stop of colours) expect(stop.to - stop.from).toBeGreaterThan(1);
  expect(stops.at(-1)!.to).toBeCloseTo(100, 6);
});

it("keeps the legend's counts exact when the ring exaggerates a sliver", async () => {
  const items = [
    ...Array.from({ length: 999 }, (_, i) => ({ name: `ok-${i}`, namespace: "flux-system", age: "1d", columns: [], status: { status: "healthy", label: "Ready" } })),
    { name: "broken", namespace: "flux-system", age: "1d", columns: [], status: { status: "error", label: "Not ready" } },
  ];
  const migrated = {
    ...plugin,
    manifest: { ...plugin.manifest,
      capabilities: [{ name: "apps", target: "k8s.listCustomResource", arguments: { group: "kustomize.toolkit.fluxcd.io", kind: "Kustomization" } }],
      contributions: { ...plugin.manifest.contributions,
        pages: plugin.manifest.contributions.pages.map(({ statusColumns: _unused, ...page }) => page),
        statusResolvers: [{ forKinds: ["kustomize.toolkit.fluxcd.io/Kustomization"], rules: [] }] } },
  } as unknown as InstalledExtension;
  vi.mocked(readExtension).mockResolvedValue({ items } as never);
  render(<ExtensionWorkspace plugin={migrated} page={migrated.manifest.contributions.pages[0]} context="staging" />);
  expect(await screen.findByText("Healthy: 999")).toBeTruthy();
  expect(screen.getByText("Error: 1")).toBeTruthy();
});

it("parts adjacent segments with the surface, so two neutrals never meet edge to edge", () => {
  // Suspended and Unknown are both neutral inks, too close to tell apart by
  // colour; a surface gap between them reads at 3:1 or better in every theme.
  const ring = donutBackground({ healthy: 0, warning: 0, error: 0, progressing: 0, suspended: 2, unknown: 2 });
  const stops = ring.replace(/^conic-gradient\(/, "").replace(/\)$/, "").split(/(?<=%),/).map((stop) => stop.trim());
  const suspended = stops.findIndex((stop) => stop.startsWith(DONUT_COLORS.suspended));
  const unknown = stops.findIndex((stop) => stop.startsWith(DONUT_COLORS.unknown));
  expect(suspended).toBeGreaterThanOrEqual(0);
  expect(stops.slice(suspended + 1, unknown).some((stop) => stop.startsWith("var(--surface"))).toBe(true);
  // A single segment is a whole ring, with no gap cut out of it.
  expect(donutBackground({ healthy: 3, warning: 0, error: 0, progressing: 0, suspended: 0, unknown: 0 })).not.toContain("--surface");
});
// jsdom omits the browser layout APIs used by the shared searchable picker.
if (!("ResizeObserver" in globalThis)) {
  (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver =
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    };
}
HTMLElement.prototype.scrollIntoView ??= () => {};
const plugin = {
  manifest: {
    id: "org.test.flux",
    name: "Flux",
    capabilities: [
      {
        name: "apps",
        arguments: {
          printerColumns: [
            { name: "Ready" },
            { name: "Suspended" },
            { name: "Reconciling" },
          ],
        },
      },
    ],
    contributions: {
      pages: [
        {
          id: "overview",
          title: "Overview",
          capability: "apps",
          dashboard: { pages: ["apps"] },
        },
        {
          id: "apps",
          title: "Kustomizations",
          capability: "apps",
          statusColumns: { ready: 0, suspended: 1, progressing: 2 },
        },
        {
          id: "repos",
          title: "Git repositories",
          capability: "apps",
          group: "Sources",
        },
      ],
    },
  },
  revision: 3,
} as unknown as InstalledExtension;
beforeEach(async () => {
  vi.resetAllMocks();
  vi.spyOn(await import("@srelens/core/lib/workloads"),"listNamespaces").mockImplementation((...args)=>listNamespaces(...args));
  vi.mocked(listNamespaces).mockResolvedValue({ namespaces: [] } as never);
  vi.mocked(readExtension).mockResolvedValue({
    items: [
      {
        name: "apps",
        namespace: "flux-system",
        age: "1d",
        columns: ["True", "false", "False"],
      },
    ],
  });
});
it("counts a resolver-backed page by the host's resolved statuses, in words", async () => {
  // Flux after #541: no statusColumns; the host resolves each row's status.
  const migrated = {
    ...plugin,
    manifest: {
      ...plugin.manifest,
      capabilities: [{ name: "apps", target: "k8s.listCustomResource",
        arguments: { group: "kustomize.toolkit.fluxcd.io", kind: "Kustomization" } }],
      contributions: {
        ...plugin.manifest.contributions,
        pages: plugin.manifest.contributions.pages.map(({ statusColumns: _unused, ...page }) => page),
        statusResolvers: [{ forKinds: ["kustomize.toolkit.fluxcd.io/Kustomization"], rules: [] }],
      },
    },
  } as unknown as InstalledExtension;
  vi.mocked(readExtension).mockResolvedValue({ items: [
    { name: "a", namespace: "flux-system", age: "1d", columns: [], status: { status: "healthy", label: "Ready" } },
    { name: "b", namespace: "flux-system", age: "1d", columns: [], status: { status: "suspended", label: "Suspended" } },
    { name: "c", namespace: "flux-system", age: "1d", columns: [], status: { status: "error", label: "Not ready" } },
    { name: "d", namespace: "flux-system", age: "1d", columns: [], status: { status: "error", label: "Not ready" } },
  ] });
  render(<ExtensionWorkspace plugin={migrated} page={migrated.manifest.contributions.pages[0]} context="staging" />);
  expect(await screen.findByText("Healthy: 1")).toBeTruthy();
  expect(screen.getByText("Error: 2")).toBeTruthy();
  expect(screen.getByText("Suspended: 1")).toBeTruthy();
  // Every status is listed, zero included, so an absent colour is never the answer.
  for (const word of ["Warning: 0", "Progressing: 0", "Unknown: 0"]) expect(screen.getByText(word)).toBeTruthy();
});
it("shows dashboard counts and navigates to grouped resource pages on the pinned cluster", async () => {
  const onPage = vi.fn();
  render(
    <ExtensionWorkspace
      plugin={plugin}
      page={plugin.manifest.contributions.pages[0]}
      context="staging"
      onPage={onPage}
    />,
  );
  expect(await screen.findByText("Healthy: 1")).toBeTruthy();
  expect(readExtension).toHaveBeenCalledWith(
    "org.test.flux",
    3,
    "apps",
    "staging",
    "",
  );
  fireEvent.click(screen.getByRole("button", { name: "Sources" }));
  expect(onPage).toHaveBeenCalledWith("repos", "");
});
it("refreshes dashboard counts when an action on one of their resources is accepted", async () => {
  const { EXTENSION_RESOURCE_CHANGED } = await import("@srelens/core");
  render(
    <ExtensionWorkspace
      plugin={plugin}
      page={plugin.manifest.contributions.pages[0]}
      context="staging"
    />,
  );
  expect(await screen.findByText("Healthy: 1")).toBeTruthy();
  const before = vi.mocked(readExtension).mock.calls.length;
  const changed = (detail: object) =>
    window.dispatchEvent(new CustomEvent(EXTENSION_RESOURCE_CHANGED, { detail }));
  const resource = { id: "org.test.flux", revision: 3, capability: "apps", context: "staging", namespace: "flux-system", name: "apps" };
  changed({ ...resource, context: "prod" });
  changed({ ...resource, id: "org.other.app" });
  changed({ ...resource, capability: "other" });
  expect(readExtension).toHaveBeenCalledTimes(before);
  changed(resource);
  await waitFor(() =>
    expect(vi.mocked(readExtension).mock.calls.length).toBeGreaterThan(before),
  );
});
it("reports failed summaries instead of displaying zero healthy resources", async () => {
  vi.mocked(readExtension).mockRejectedValue(new Error("Forbidden"));
  render(
    <ExtensionWorkspace
      plugin={plugin}
      page={plugin.manifest.contributions.pages[0]}
      context="staging"
    />,
  );
  expect((await screen.findByRole("alert")).textContent).toContain("Forbidden");
  expect(screen.queryByText("Healthy: 0")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Retry" }));
  await waitFor(() => expect(readExtension).toHaveBeenCalledTimes(2));
});

it("says when event reads were capped and pages matching rows", async () => {
  const eventPlugin = structuredClone(plugin);
  eventPlugin.manifest.contributions.pages[0].dashboard!.events = {
    capability: "events",
    apiGroups: ["kustomize.toolkit.fluxcd.io"],
  };
  const events = Array.from({ length: 120 }, (_, i) => ({
    name: `ev-${i}`,
    namespace: "flux-system",
    object: "Kustomization/apps",
    objectApiVersion: "kustomize.toolkit.fluxcd.io/v1",
    message: `Event ${i}`,
    type: "Normal",
    count: 1,
    age: "1m",
  }));
  vi.mocked(readExtension).mockImplementation(
    async (_id, _revision, capability) =>
      (capability === "events"
        ? { events, truncated: true }
        : { items: [] }) as never,
  );
  render(
    <ExtensionWorkspace
      plugin={eventPlugin}
      page={eventPlugin.manifest.contributions.pages[0]}
      context="staging"
    />,
  );
  expect(
    await screen.findByText(
      "Showing the first 120 events; more remain on the cluster.",
    ),
  ).toBeTruthy();
  expect(screen.getByText("Events")).toBeTruthy();
  expect(screen.getByText(/\(120\+\)/)).toBeTruthy();
  expect(screen.getByText("Event 0")).toBeTruthy();
  expect(screen.queryByText("Event 100")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: /Show 20 more/ }));
  expect(screen.getByText("Event 100")).toBeTruthy();
});

it("resets event paging when the extension revision changes", async () => {
  const eventPlugin = structuredClone(plugin);
  eventPlugin.manifest.contributions.pages[0].dashboard!.events = {
    capability: "events",
    apiGroups: ["kustomize.toolkit.fluxcd.io"],
  };
  const events = Array.from({ length: 120 }, (_, i) => ({
    name: `ev-${i}`,
    namespace: "flux-system",
    object: "Kustomization/apps",
    objectApiVersion: "kustomize.toolkit.fluxcd.io/v1",
    message: `Event ${i}`,
    type: "Normal",
    count: 1,
    age: "1m",
  }));
  vi.mocked(readExtension).mockImplementation(
    async (_id, _revision, capability) =>
      (capability === "events" ? { events } : { items: [] }) as never,
  );
  const { rerender } = render(
    <ExtensionWorkspace
      plugin={eventPlugin}
      page={eventPlugin.manifest.contributions.pages[0]}
      context="staging"
    />,
  );
  await screen.findByText("Event 0");
  fireEvent.click(screen.getByRole("button", { name: /Show 20 more/ }));
  expect(screen.getByText("Event 100")).toBeTruthy();
  rerender(
    <ExtensionWorkspace
      plugin={{ ...eventPlugin, revision: eventPlugin.revision + 1 }}
      page={eventPlugin.manifest.contributions.pages[0]}
      context="staging"
    />,
  );
  await waitFor(() => expect(screen.queryByText("Event 100")).toBeNull());
  expect(screen.getByText("Event 0")).toBeTruthy();
});

it("filters events by API group and search, not just a matching kind name", async () => {
  const eventPlugin = structuredClone(plugin);
  eventPlugin.manifest.contributions.pages[0].dashboard!.events = {
    capability: "events",
    apiGroups: ["kustomize.toolkit.fluxcd.io"],
  };
  vi.mocked(readExtension).mockImplementation(
    async (_id, _revision, capability) =>
      (capability === "events"
        ? {
            events: [
              {
                name: "one",
                namespace: "flux-system",
                object: "Kustomization/apps",
                objectApiVersion: "kustomize.toolkit.fluxcd.io/v1",
                message: "Reconciliation succeeded",
                type: "Normal",
                count: 2,
                source: "kustomize-controller",
                age: "1m",
              },
              {
                name: "two",
                namespace: "flux-system",
                object: "Kustomization/other",
                objectApiVersion: "example.org/v1",
                message: "Unrelated event",
                type: "Normal",
                count: 1,
                age: "2m",
              },
            ],
          }
        : { items: [] }) as never,
  );
  render(
    <ExtensionWorkspace
      plugin={eventPlugin}
      page={eventPlugin.manifest.contributions.pages[0]}
      context="staging"
    />,
  );
  expect(await screen.findByText("Reconciliation succeeded")).toBeTruthy();
  expect(screen.queryByText("Unrelated event")).toBeNull();
  fireEvent.change(
    screen.getByRole("textbox", { name: "Search app resources" }),
    { target: { value: "does not match" } },
  );
  expect(await screen.findByText("No matching events.")).toBeTruthy();
  expect(readExtension).toHaveBeenCalledTimes(2);
});

it("retains a namespace discovery error and offers retry", async () => {
  vi.mocked(listNamespaces).mockResolvedValue({
    error: "namespace request timed out",
  });
  render(
    <ExtensionWorkspace
      plugin={plugin}
      page={plugin.manifest.contributions.pages[0]}
      context="staging"
    />,
  );
  expect((await screen.findByRole("alert")).textContent).toContain(
    "namespace request timed out",
  );
  fireEvent.click(screen.getByRole("button", { name: "Retry" }));
  await waitFor(() => expect(listNamespaces).toHaveBeenCalledTimes(2));
});

it("asks the host for only a dashboard card's rows on its target page", async () => {
  render(
    <ExtensionWorkspace
      plugin={plugin}
      page={plugin.manifest.contributions.pages[1]}
      context="staging"
      namespace="flux-system"
      card="suspended"
    />,
  );
  expect(await screen.findByRole("cell", { name: "apps" })).toBeTruthy();
  expect(readExtension).toHaveBeenCalledWith("org.test.flux", 3, "apps", "staging", "flux-system", true, "suspended");
});

it("reads a card's rows over the several namespaces it counted in", async () => {
  // Narrows like the host: only rows in the namespaces the read names.
  const rows = [
    { name: "in-prod", namespace: "prod", age: "1d", columns: [] },
    { name: "in-team", namespace: "team", age: "1d", columns: [] },
    { name: "in-other", namespace: "other", age: "1d", columns: [] },
  ];
  vi.mocked(readExtension).mockImplementation((async (...args: unknown[]) => {
    const scope = (args[7] as string[] | undefined) ?? [];
    return { items: rows.filter((row) => !scope.length || scope.includes(row.namespace)) };
  }) as never);
  render(
    <ExtensionWorkspace
      plugin={plugin}
      page={plugin.manifest.contributions.pages[1]}
      context="staging"
      card="suspended"
      cardNamespaces={["prod", "team"]}
    />,
  );
  expect(await screen.findByRole("cell", { name: "in-prod" })).toBeTruthy();
  expect(screen.getByRole("cell", { name: "in-team" })).toBeTruthy();
  expect(screen.queryByRole("cell", { name: "in-other" })).toBeNull();
  expect(readExtension).toHaveBeenCalledWith("org.test.flux", 3, "apps", "staging", "", true, "suspended", ["prod", "team"]);
});

it("filters resource rows without a second cluster read", async () => {
  render(
    <ExtensionWorkspace
      plugin={plugin}
      page={plugin.manifest.contributions.pages[1]}
      context="staging"
    />,
  );
  expect(await screen.findByRole("cell", { name: "apps" })).toBeTruthy();
  fireEvent.change(
    screen.getByRole("textbox", { name: "Search app resources" }),
    { target: { value: "missing" } },
  );
  expect(screen.queryByRole("cell", { name: "apps" })).toBeNull();
  expect(readExtension).toHaveBeenCalledTimes(1);
});

it("reloads the selected namespace without changing the pinned cluster", async () => {
  vi.mocked(listNamespaces).mockResolvedValue({
    namespaces: ["flux-system"],
    summaries: [],
  });
  render(
    <ExtensionWorkspace
      plugin={plugin}
      page={plugin.manifest.contributions.pages[1]}
      context="staging"
    />,
  );
  await screen.findByRole("cell", { name: "apps" });
  fireEvent.click(
    screen.getByRole("combobox", { name: "App namespace" }),
  );
  fireEvent.click(await screen.findByRole("option", { name: "flux-system" }));
  await waitFor(() =>
    expect(readExtension).toHaveBeenLastCalledWith(
      "org.test.flux",
      3,
      "apps",
      "staging",
      "flux-system",
      true,
    ),
  );
});

it("uses the restricted namespace instead of an all-namespace resource read", async () => {
  vi.mocked(listNamespaces).mockResolvedValue({error:'Forbidden: User "system:serviceaccount:team:reader" cannot list namespaces'} as any);
  vi.spyOn(await import("@srelens/core/lib/clusters"),"listContexts").mockResolvedValue({contexts:[{name:"staging",namespace:"team"}]} as any);
  render(<ExtensionWorkspace plugin={plugin} page={plugin.manifest.contributions.pages[1]} context="staging" />);
  await waitFor(()=>expect(readExtension).toHaveBeenLastCalledWith(plugin.manifest.id,plugin.revision,"apps","staging","team",true));
  expect(vi.mocked(readExtension).mock.calls.every(call=>call[4]==="team")).toBe(true);
});

it("reads a card target's own namespaces even under a namespace-restricted credential", async () => {
  // The route says what the card counted; the credential's one namespace must not
  // quietly narrow it. If the credential cannot read them, the read says so.
  vi.mocked(listNamespaces).mockResolvedValue({error:'Forbidden: User "system:serviceaccount:team:reader" cannot list namespaces'} as any);
  vi.spyOn(await import("@srelens/core/lib/clusters"),"listContexts").mockResolvedValue({contexts:[{name:"staging",namespace:"team"}]} as any);
  render(<ExtensionWorkspace plugin={plugin} page={plugin.manifest.contributions.pages[1]} context="staging" card="suspended" cardNamespaces={["prod","team"]} />);
  await waitFor(()=>expect(readExtension).toHaveBeenCalled());
  await waitFor(()=>expect(screen.getByRole("combobox",{name:"App namespace"}).textContent).toContain("prod, team"));
  const cardReads = vi.mocked(readExtension).mock.calls.filter(call=>call[6]==="suspended");
  expect(cardReads.length).toBeGreaterThan(0);
  for (const call of cardReads) expect(call.slice(4)).toEqual(["",true,"suspended",["prod","team"]]);
});

it("carries the selected namespace into group navigation", async () => {
  vi.mocked(listNamespaces).mockResolvedValue({namespaces:["team"]} as any);
  const onPage=vi.fn();
  const onNamespace=vi.fn();
  render(<ExtensionWorkspace plugin={plugin} page={plugin.manifest.contributions.pages[1]} context="staging" onPage={onPage} onNamespace={onNamespace}/>);
  await screen.findByRole("cell",{name:"apps"});
  fireEvent.click(screen.getByRole("combobox",{name:"App namespace"}));
  fireEvent.click(await screen.findByRole("option",{name:"team"}));
  fireEvent.click(screen.getByRole("button",{name:"Sources"}));
  expect(onPage).toHaveBeenCalledWith("repos","team");
  expect(onNamespace).toHaveBeenCalledWith("team");
});

it("ticks event first and last occurrence ages without reloading events", async () => {
  const {act}=await import("@testing-library/react");
  const eventPlugin=structuredClone(plugin);
  eventPlugin.manifest.contributions.pages[0].dashboard!.events={capability:"events",apiGroups:["source.toolkit.fluxcd.io"]};
  vi.mocked(readExtension).mockImplementation(async (_id,_revision,capability)=>(capability==="events"?{events:[{name:"event",objectApiVersion:"source.toolkit.fluxcd.io/v1",object:"GitRepository/apps",message:"Fetched",age:"0s",firstAge:"0s",created:"2026-09-13T12:00:10Z",firstCreated:"2026-09-13T12:00:00Z"}]}:{items:[]}) as any);
  vi.useFakeTimers();vi.setSystemTime(new Date("2026-09-13T12:00:10Z"));
  let view:ReturnType<typeof render>;
  try {
    await act(async()=>{view=render(<ExtensionWorkspace plugin={eventPlugin} page={eventPlugin.manifest.contributions.pages[0]} context="staging"/>);});
    expect(screen.getByText("10s")).toBeTruthy();
    act(()=>{vi.advanceTimersByTime(30000);});
    expect(screen.getByText("40s")).toBeTruthy();
    expect(screen.getByText("30s")).toBeTruthy();
    expect(readExtension).toHaveBeenCalledTimes(2);
  } finally {view!?.unmount();vi.useRealTimers();}
});

it("shows unsupported-cluster requirements without reading app resources", async () => {
  const requiring = structuredClone(plugin);
  requiring.manifest.capabilities[0].target = "k8s.listCustomResource";
  Object.assign(requiring.manifest.capabilities[0].arguments, { group: "kustomize.toolkit.fluxcd.io", version: "v1", plural: "kustomizations", kind: "Kustomization" });
  vi.mocked(listCrds).mockResolvedValue({ crds: [] });
  render(<ExtensionWorkspace plugin={requiring} page={requiring.manifest.contributions.pages[0]} context="staging" />);
  expect(await screen.findByText("Missing requirements")).toBeTruthy();
  expect(readExtension).not.toHaveBeenCalled();
  expect(screen.getByRole("navigation", { name: "Flux pages" })).toBeTruthy();
});
