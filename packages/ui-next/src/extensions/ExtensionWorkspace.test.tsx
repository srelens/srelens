import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { it, expect, vi, beforeEach } from "vitest";
vi.mock("@srelens/core", async (original) => ({
  ...(await original<typeof import("@srelens/core")>()),
  readExtension: vi.fn(),
  listNamespaces: vi.fn(),
}));
import {
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
beforeEach(() => {
  vi.resetAllMocks();
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
  expect(onPage).toHaveBeenCalledWith("repos");
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
    screen.getByRole("textbox", { name: "Search extension resources" }),
    { target: { value: "does not match" } },
  );
  expect(await screen.findByText("No matching events.")).toBeTruthy();
  expect(readExtension).toHaveBeenCalledTimes(2);
});

it("retains a namespace discovery error and offers retry", async () => {
  vi.mocked(listNamespaces).mockResolvedValue({
    error: "namespace list forbidden",
  });
  render(
    <ExtensionWorkspace
      plugin={plugin}
      page={plugin.manifest.contributions.pages[0]}
      context="staging"
    />,
  );
  expect((await screen.findByRole("alert")).textContent).toContain(
    "namespace list forbidden",
  );
  fireEvent.click(screen.getByRole("button", { name: "Retry" }));
  await waitFor(() => expect(listNamespaces).toHaveBeenCalledTimes(2));
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
    screen.getByRole("textbox", { name: "Search extension resources" }),
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
    screen.getByRole("combobox", { name: "Extension namespace" }),
  );
  fireEvent.click(await screen.findByRole("option", { name: "flux-system" }));
  await waitFor(() =>
    expect(readExtension).toHaveBeenLastCalledWith(
      "org.test.flux",
      3,
      "apps",
      "staging",
      "flux-system",
    ),
  );
});
