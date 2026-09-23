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
import { ExtensionWorkspace, resourceStatus } from "./ExtensionWorkspace";
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
it("classifies statuses without counting suspended or reconciling as ready", () => {
  const columns = { ready: 0, suspended: 1, progressing: 2 };
  expect(resourceStatus(["True", "true", "False"], columns)).toBe("Suspended");
  expect(resourceStatus(["True", "false", "True"], columns)).toBe(
    "In progress",
  );
  expect(resourceStatus(["True", "false", "False"], columns)).toBe("Ready");
  expect(resourceStatus(["False"], columns)).toBe("Not ready");
  expect(resourceStatus([], columns)).toBe("Unknown");
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
  expect(await screen.findByText("Ready: 1")).toBeTruthy();
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
  expect(await screen.findByText("Ready: 1")).toBeTruthy();
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
  expect(screen.queryByText("Ready: 0")).toBeNull();
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
