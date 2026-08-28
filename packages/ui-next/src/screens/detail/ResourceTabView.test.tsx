import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, within, type RenderOptions } from "@testing-library/react";
import { render as renderBare } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactElement, ReactNode } from "react";
import type { K8sObject, PodMetric } from "@srelens/core";
import type { KindDescriptor, ListRow } from "../../lib/kinds/types";

// The reads behind the tab: the object itself, the pod usage its CPU and
// Memory tiles show, and the two pane fetches it inherits from the shared
// pane machinery.
const { getObject, getManifest, listEvents, listCrds, podMetrics, podsForSelector } = vi.hoisted(() => ({
  getObject: vi.fn(async (): Promise<{ object?: K8sObject; error?: string }> => ({})),
  getManifest: vi.fn(async (): Promise<{ yaml?: string; error?: string }> => ({ yaml: "" })),
  listEvents: vi.fn(async () => ({ events: [] })),
  listCrds: vi.fn(async () => ({ crds: [] })),
  podMetrics: vi.fn(async (): Promise<{ metrics?: PodMetric[]; error?: string }> => ({ metrics: [] })),
  podsForSelector: vi.fn(async () => ({ pods: [] })),
}));

vi.mock("@srelens/core", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@srelens/core")>()),
  getObject,
  getManifest,
  listEvents,
  listCrds,
  podMetrics,
  podsForSelector,
}));

const { descriptorFor } = vi.hoisted(() => ({
  descriptorFor: vi.fn((_slug: string): KindDescriptor<ListRow> | undefined => undefined),
}));

vi.mock("../../lib/kinds/descriptors", () => ({ descriptorFor }));

import { ConsoleProvider } from "../../console";
import { loadSectionFolds, setSectionOpen } from "../../lib/sectionFolds";
import { detailFacts } from "./detailData";
import { ResourceTabView } from "./ResourceTabView";

function Wrapper({ children }: { children: ReactNode }) {
  return <ConsoleProvider>{children}</ConsoleProvider>;
}

function render_(ui: ReactElement, options?: Omit<RenderOptions, "wrapper">) {
  return renderBare(ui, { wrapper: Wrapper, ...options });
}

const DAY_MS = 24 * 60 * 60 * 1000;
const daysAgo = (days: number) => new Date(Date.now() - days * DAY_MS).toISOString();

/** The mock's own subject: a healthy Pod with two containers. */
const POD: K8sObject = {
  kind: "Pod",
  apiVersion: "v1",
  metadata: {
    name: "cart-session-store-1",
    namespace: "checkout",
    creationTimestamp: daysAgo(211),
    labels: { app: "cart-session-store" },
    annotations: { "srelens.io/last-applied-by": "dana@acme.io" },
  },
  spec: {
    nodeName: "eu-w4-c3-standard-a3",
    serviceAccountName: "cart-session-store",
    containers: [
      {
        name: "app",
        image: "redis:7.4-alpine",
        ports: [{ containerPort: 8080 }, { containerPort: 9090 }],
        resources: { requests: { cpu: "250m", memory: "512Mi" }, limits: { cpu: "2", memory: "1Gi" } },
        readinessProbe: { httpGet: { path: "/healthz", port: 8080 }, periodSeconds: 5 },
      },
      {
        name: "otel-sidecar",
        image: "otel/opentelemetry-collector:0.112.0",
        ports: [{ containerPort: 4317 }],
        resources: { requests: { cpu: "50m", memory: "96Mi" }, limits: { cpu: "200m", memory: "256Mi" } },
      },
    ],
  },
  status: {
    phase: "Running",
    podIP: "10.44.21.4",
    qosClass: "Burstable",
    containerStatuses: [
      { name: "app", ready: true, restartCount: 0, state: { running: { startedAt: daysAgo(1) } } },
      { name: "otel-sidecar", ready: true, restartCount: 0, state: { running: { startedAt: daysAgo(1) } } },
    ],
  },
};

/** A kind `resourceStatusLine` has no verdict for, and which has no containers. */
const CONFIGMAP: K8sObject = {
  kind: "ConfigMap",
  apiVersion: "v1",
  metadata: { name: "cm-1", namespace: "default", creationTimestamp: daysAgo(30) },
};

function podDescriptor(overrides: Partial<KindDescriptor<ListRow>> = {}): KindDescriptor<ListRow> {
  return {
    k8sKind: "Pod",
    columns: [],
    source: "watch",
    scope: "namespaced",
    actions: { logs: true, shell: true, forward: true, evict: true },
    panes: { containers: true },
    ...overrides,
  };
}

/** One metric tile, read the way the design draws it: label, figure, caption. */
function tiles() {
  return Array.from(document.querySelectorAll<HTMLElement>("[data-slot='metric-strip'] .stat")).map((tile) => ({
    label: tile.querySelector(".eyebrow")?.textContent ?? "",
    value: tile.querySelector(".stat-value")?.textContent ?? "",
    caption: tile.querySelector(".num")?.textContent ?? "",
  }));
}

const tabNames = () => screen.getAllByRole("tab").map((t) => t.textContent);

async function openPod(props: Partial<{ kind: string; namespace: string | null; name: string }> = {}) {
  const view = render_(
    <ResourceTabView
      context="prod-eu"
      kind={props.kind ?? "Pod"}
      namespace={props.namespace === undefined ? "checkout" : props.namespace}
      name={props.name ?? "cart-session-store-1"}
    />,
  );
  await waitFor(() => expect(screen.getAllByRole("tab").length).toBeGreaterThan(0));
  return view;
}

/**
 * Open a titled block, the way a reader does. Every one of them opens shut on
 * a first visit — the reader asked for that — so a test reading what is inside
 * one asks for it first.
 */
async function expand(name: string) {
  await userEvent.click(screen.getByRole("button", { name }));
}

describe("ResourceTabView — the full tab the design draws", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    getObject.mockResolvedValue({ object: POD });
    getManifest.mockResolvedValue({ yaml: "kind: Pod\n" });
    listEvents.mockResolvedValue({ events: [] });
    listCrds.mockResolvedValue({ crds: [] });
    podMetrics.mockResolvedValue({ metrics: [] });
    podsForSelector.mockResolvedValue({ pods: [] });
    descriptorFor.mockReturnValue(podDescriptor());
    // The fold memory is a module-level store, so a block one test opens
    // would still be open in the next one.
    localStorage.clear();
    loadSectionFolds();
  });

  describe("the breadcrumb header", () => {
    it("heads the page with the name, then cluster / namespace / kind", async () => {
      await openPod();
      // The name is the page's heading, not a pane's: this IS the tab.
      expect(screen.getByRole("heading", { level: 1, name: "cart-session-store-1" })).toBeDefined();
      const crumb = screen.getByRole("navigation", { name: "Breadcrumb" });
      expect(Array.from(crumb.querySelectorAll("li")).map((li) => li.textContent?.replace("/", ""))).toEqual([
        "prod-eu",
        "checkout",
        "Pod",
      ]);
    });

    it("drops the namespace from the trail for a cluster-scoped subject", async () => {
      getObject.mockResolvedValue({
        object: { kind: "Node", apiVersion: "v1", metadata: { name: "worker-1" } },
      });
      descriptorFor.mockReturnValue(podDescriptor({ k8sKind: "Node", panes: {} }));
      await openPod({ kind: "Node", namespace: null, name: "worker-1" });
      const crumb = screen.getByRole("navigation", { name: "Breadcrumb" });
      expect(Array.from(crumb.querySelectorAll("li")).map((li) => li.textContent?.replace("/", ""))).toEqual([
        "prod-eu",
        "Node",
      ]);
    });

    it("puts the actions in the header row, not in a footer bar", async () => {
      await openPod();
      const header = document.querySelector("header")!;
      const words = Array.from(header.querySelectorAll("button")).map((b) => b.textContent);
      // The design's row: Ask first, then the kind's own, then the overflow.
      expect(words[0]).toBe("Ask");
      expect(words).toContain("Logs");
      expect(words).toContain("Shell");
      expect(words).toContain("Edit");
      expect(document.querySelector("footer")).toBeNull();
    });
  });

  describe("the tab strip", () => {
    it("names Overview rather than the peek's Details, and shows only the panes that exist", async () => {
      await openPod();
      // Relations, Drill and Metrics are deferred and have no body; the strip
      // names a pane only when there is something behind it.
      expect(tabNames()).toEqual(["Overview", "YAML", "Events"]);
    });

    it("keeps Containers off the strip — its table is inline on Overview", async () => {
      await openPod();
      expect(screen.queryByRole("tab", { name: "Containers" })).toBeNull();
    });

    it("rules the active tab rather than filling it", async () => {
      await openPod();
      expect(document.querySelector(".utabs")).toBeTruthy();
    });
  });

  describe("the metric strip", () => {
    it("reads the ready figure, restarts and age off the object", async () => {
      podMetrics.mockResolvedValue({
        metrics: [{ name: "cart-session-store-1", cpuMillicores: 101, memoryMiB: 962 }] as PodMetric[],
      });
      await openPod();
      await waitFor(() => expect(tiles().length).toBe(5));
      const strip = tiles();
      expect(strip.map((t) => t.label)).toEqual(["Ready", "Restarts", "CPU", "Memory", "Age"]);
      expect(strip[0].value).toBe("2/2 ready");
      // The caption is core's own verdict, never a phrase paired with a tone
      // here — that pairing has been found and removed six times on this
      // branch.
      expect(strip[0].caption).toBe("Running");
      expect(strip[1].value).toBe("0");
      expect(strip[1].caption).toBe("none");
      expect(strip[4].value).toBe("211d");
    });

    it("shows the usage the metrics server reports, and says what each figure is", async () => {
      podMetrics.mockResolvedValue({
        metrics: [{ name: "cart-session-store-1", cpuMillicores: 101, memoryMiB: 962 }] as PodMetric[],
      });
      await openPod();
      await waitFor(() => expect(tiles()[2].value).not.toBe("—"));
      const strip = tiles();
      expect(strip[2].value).toBe("101m");
      expect(strip[2].caption).toBe("current");
      expect(strip[3].caption).toBe("working set");
    });

    it("omits the usage tiles rather than drawing em dashes when nothing reports any", async () => {
      // A cluster with no metrics-server costs the reader two tiles, not the
      // page — the same best-effort treatment the related-pods table gives
      // this very call.
      await openPod();
      expect(tiles().map((t) => t.label)).toEqual(["Ready", "Restarts", "Age"]);
    });

    it("draws only the tiles a kind actually has", async () => {
      getObject.mockResolvedValue({ object: CONFIGMAP });
      descriptorFor.mockReturnValue(podDescriptor({ k8sKind: "ConfigMap", panes: {} }));
      await openPod({ kind: "ConfigMap", namespace: "default", name: "cm-1" });
      // A ConfigMap has no health, no containers and no usage — one tile, not
      // four em dashes pretending to be figures.
      expect(tiles().map((t) => t.label)).toEqual(["Age"]);
    });

    it("does not ask the metrics server about a kind that has no pod usage", async () => {
      getObject.mockResolvedValue({ object: CONFIGMAP });
      descriptorFor.mockReturnValue(podDescriptor({ k8sKind: "ConfigMap", panes: {} }));
      await openPod({ kind: "ConfigMap", namespace: "default", name: "cm-1" });
      expect(podMetrics).not.toHaveBeenCalled();
    });
  });

  describe("Overview", () => {
    it("lays the facts out as three columns of label-above-value, in a grid of its own", async () => {
      await openPod();
      // THIS SCREEN'S grid, built here — not the peek's rows restyled from
      // above, which is what `FactGrid` did. The rows say so themselves:
      // `stacked` is the form a row takes, so the label sits over the value
      // and the pair is ruled off beneath.
      const grid = document.querySelector<HTMLElement>("[data-slot='fact-grid']")!;
      expect(grid).toBeTruthy();
      expect(grid.className).toContain("grid-cols-3");
      const rows = [...grid.querySelectorAll<HTMLElement>(".kv")];
      expect(rows.length).toBeGreaterThan(0);
      expect(rows.every((row) => row.dataset.stacked === "true")).toBe(true);
      // The very facts the peek reads down a column — one derivation, two
      // layouts.
      expect(within(grid).getByText("QoS class")).toBeDefined();
      expect(within(grid).getByText("Burstable")).toBeDefined();
    });

    it("draws every fact it was handed, not a selection of them", async () => {
      await openPod();
      // EQUALS the derived list, in its order. A screen that filtered three
      // facts out of the list it shares with the peek would pass every other
      // assertion in this file, and the two screens would silently disagree
      // about what a pod is — the drift the retired both-hosts comparison
      // used to catch, caught here without either screen's markup standing in
      // for the other's.
      const derived = detailFacts({ kind: "Pod", object: POD }).map((f) => f.label);
      expect(derived.length).toBeGreaterThan(3);
      const drawn = [...document.querySelectorAll("[data-slot='fact-grid'] .kv-k")].map(
        (el) => el.textContent ?? "",
      );
      expect(drawn).toEqual(derived);
    });

    it("draws no fact of the peek's own form, so nothing here is the peek's markup", async () => {
      await openPod();
      const grid = document.querySelector<HTMLElement>("[data-slot='fact-grid']")!;
      // Every row in the grid is this screen's form. A row here in the peek's
      // form would mean the tab was showing something the peek built.
      expect(grid.querySelectorAll(".kv:not([data-stacked])")).toHaveLength(0);
    });

    it("puts the containers table on Overview, with the design's columns", async () => {
      await openPod();
      await expand("Containers");
      // Scoped by the block's own heading: `Table` names no table, and the
      // Overview holds more than one.
      const block = screen
        .getByRole("heading", { name: "Containers" })
        .closest("section.section") as HTMLElement;
      const table = within(block).getByRole("table");
      expect(Array.from(table.querySelectorAll("thead th")).map((th) => th.textContent?.trim())).toEqual([
        "Name",
        "Image",
        "Ports",
        "Requests",
        "Limits",
        "Probe",
        "State",
      ]);
      const rows = Array.from(table.querySelectorAll("tbody tr.tbl-row"));
      expect(rows.length).toBe(2);
      const cells = Array.from(rows[0].querySelectorAll("td")).map((td) => td.textContent);
      expect(cells[0]).toBe("app");
      expect(cells[1]).toBe("redis:7.4-alpine");
      expect(cells[3]).toBe("250m · 512Mi");
      expect(cells[4]).toBe("2 · 1Gi");
      // The state word and its tone are `containerStateText`'s, the one rule
      // every container state in srelens is read through.
      expect(cells[6]).toContain("running");
    });

    it("compensates by hand for the one hairline it breaks on purpose", async () => {
      await openPod();
      const body = document.querySelector<HTMLElement>(".pane-body")!;
      // Every block of Overview is a `.section` sibling of every other, which
      // is what `.section + .section` draws the hairlines between — with one
      // exception, and the exception is the point: Labels and Annotations read
      // side by side here, so neither can be the other's adjacent sibling and
      // the rule between them would run down the middle of the row instead of
      // across it.
      const breaks = [...body.children].filter((el) => !el.matches("section.section"));
      expect(breaks.map((el) => el.getAttribute("data-slot"))).toEqual(["metadata-pair"]);

      // Which means this block owes the run the two rules it took away, drawn
      // by hand: one above the row, where the sibling rule would have drawn
      // against the block before it, and one down the middle, between the two
      // columns. The peek needs neither and has neither — it stacks them.
      const pair = breaks[0];
      expect(pair.className).toContain("rule-t");
      expect([...pair.children].map((column) => column.className)).toEqual(["", "rule-l"]);
    });

    it("reads Labels and Annotations side by side", async () => {
      await openPod();
      const pair = document.querySelector<HTMLElement>("[data-slot='metadata-pair']")!;
      expect(pair).toBeTruthy();
      expect(pair.className).toContain("grid-cols-2");
      expect(within(pair).getByRole("heading", { name: "Labels" })).toBeDefined();
      expect(within(pair).getByRole("heading", { name: "Annotations" })).toBeDefined();
      // Two columns, so neither is the other's `.section` sibling — an
      // adjacent-sibling rule would draw a hairline down the middle of a row.
      expect(pair.querySelectorAll(":scope > .section").length).toBe(0);
    });
  });

  describe("what it shares with the peek", () => {
    it("reads the subject once, however many panes are opened", async () => {
      await openPod();
      expect(getObject).toHaveBeenCalledTimes(1);
      expect(getObject).toHaveBeenCalledWith("prod-eu", "Pod", "checkout", "cart-session-store-1");
    });

    it("fetches a pane's own data only once that pane is opened", async () => {
      const view = await openPod();
      expect(getManifest).not.toHaveBeenCalled();
      await view.rerender(
        <ResourceTabView context="prod-eu" kind="Pod" namespace="checkout" name="cart-session-store-1" />,
      );
      expect(getManifest).not.toHaveBeenCalled();
    });

    it("opens every titled block shut, here as in the peek", async () => {
      await openPod();
      for (const name of ["Containers", "Labels", "Annotations"]) {
        expect(screen.getByRole("button", { name }).getAttribute("aria-expanded")).toBe("false");
      }
      // The lead fact grid has no heading, so it has no control and stays
      // open — a tab that opened showing nothing at all is hostile.
      expect(screen.getByText("Burstable")).toBeDefined();
    });

    it("reads the same memory the peek writes, since a block is the same block in both", async () => {
      // The two hosts lay a subject out differently and remember it once. The
      // memory is per KIND, so this is what a reader who opened Annotations
      // on some other Pod sees here.
      setSectionOpen("Pod", "Annotations", true);
      await openPod();
      expect(screen.getByRole("button", { name: "Annotations" }).getAttribute("aria-expanded")).toBe("true");
      expect(screen.getByText("dana@acme.io")).toBeDefined();
      expect(screen.getByRole("button", { name: "Labels" }).getAttribute("aria-expanded")).toBe("false");
    });
  });
});
