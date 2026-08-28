import { describe, it, expect, vi, beforeEach } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const { watchResource, useNamespaceOptions, cronjobSetSuspend } = vi.hoisted(() => ({
  watchResource: vi.fn(),
  useNamespaceOptions: vi.fn(),
  cronjobSetSuspend: vi.fn(),
}));

vi.mock("@srelens/core", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@srelens/core")>()),
  watchResource: (...a: unknown[]) => watchResource(...a),
  cronjobSetSuspend: (...a: unknown[]) => cronjobSetSuspend(...a),
}));

vi.mock("@srelens/core/react", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@srelens/core/react")>()),
  useNamespaceOptions: (...a: unknown[]) => useNamespaceOptions(...a),
}));

if (!("ResizeObserver" in globalThis)) {
  (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}
const proto = window.HTMLElement.prototype as unknown as Record<string, unknown>;
proto.scrollIntoView ??= () => {};
proto.hasPointerCapture ??= () => false;
proto.setPointerCapture ??= () => {};
proto.releasePointerCapture ??= () => {};

import { resourceStatusLine, type ClusterContext, type K8sObject } from "@srelens/core";
import { Workloads } from "./Workloads";
import { ConsoleProvider } from "../console";
import * as store from "../lib/tabsStore";
import { defaultState } from "../lib/tabs";
import { resetContexts, setContexts, setKubeconfigFiles } from "../lib/clusters";
import { loadColumnPrefs } from "../lib/columnPrefs";
import { resetListCache } from "../lib/resourceList";
import { resetView } from "../lib/workspace";

const CTX: ClusterContext = {
  name: "prod-eu",
  stableId: "prod",
  cluster: "prod",
  server: "https://prod",
  isCurrent: true,
  sourceFile: "/home/dana/.kube/config",
  authKind: "client certificate",
};

// One row per kind, deliberately with ages that interleave across kinds
// rather than falling neatly kind-by-kind — the fixture the cross-kind sort
// test depends on.
const DEPLOYMENTS = [
  { name: "checkout", namespace: "default", ready: "2/2", upToDate: 2, available: 2, age: "10d" },
];
const STATEFULSETS = [
  { name: "db", namespace: "default", ready: "1/1", updated: 1, service: "db-svc", age: "30d" },
];
const DAEMONSETS = [
  { name: "node-exporter", namespace: "kube-system", desired: 3, current: 3, ready: 3, upToDate: 3, available: 3, age: "1d" },
];
const PODS = [
  { name: "web-1", namespace: "default", phase: "Running", ready: "1/1", restarts: 0, node: "n1", age: "2d", image: "acme/web:1" },
];
const CRONJOBS = [
  { name: "nightly-backup", namespace: "default", schedule: "0 2 * * *", suspended: false, active: 0, lastSchedule: "2h ago", age: "120d" },
];

const FIXTURES: Record<string, unknown[]> = {
  deployments: DEPLOYMENTS,
  statefulsets: STATEFULSETS,
  daemonsets: DAEMONSETS,
  pods: PODS,
  cronjobs: CRONJOBS,
};

let stop: ReturnType<typeof vi.fn>;

beforeEach(() => {
  vi.clearAllMocks();
  stop = vi.fn();
  watchResource.mockImplementation(
    async (
      _context: string,
      _namespace: string,
      kind: string,
      onRows: (rows: unknown[]) => void,
    ) => {
      onRows(FIXTURES[kind] ?? []);
      return { stop };
    },
  );
  useNamespaceOptions.mockReturnValue({ namespaces: ["default", "kube-system"], scope: "", error: "" });
  cronjobSetSuspend.mockResolvedValue({ ok: true });

  resetContexts();
  setContexts([CTX]);
  setKubeconfigFiles(["/home/u/.kube/config"]);
  store.setState(defaultState([CTX]));
  resetView();
  resetListCache();
  loadColumnPrefs();
});

/** The name cell of every rendered row, in table order. */
const rowNames = () =>
  Array.from(document.querySelectorAll("tbody tr.tbl-row")).map(
    (row) => row.querySelector("td:not(.tbl-check)")?.textContent ?? null,
  );

/** The tab a route is open in — the one the screen under test is bound to. */
const tabFor = (route: string) => store.currentWorkspace().tabs.find((t) => t.route === route)!;

function open() {
  store.openTab("/resources");
  return render(
    <ConsoleProvider>
      <Workloads route="/resources" />
    </ConsoleProvider>,
  );
}

describe("Workloads", () => {
  it("lists every workload kind at once, each row saying which it is", async () => {
    open();

    await waitFor(() => expect(rowNames()).toHaveLength(5));

    expect(within(screen.getByText("checkout").closest("tr")!).getByText("Deployment")).toBeTruthy();
    expect(within(screen.getByText("db").closest("tr")!).getByText("StatefulSet")).toBeTruthy();
    expect(within(screen.getByText("node-exporter").closest("tr")!).getByText("DaemonSet")).toBeTruthy();
    expect(within(screen.getByText("web-1").closest("tr")!).getByText("Pod")).toBeTruthy();
    expect(within(screen.getByText("nightly-backup").closest("tr")!).getByText("CronJob")).toBeTruthy();

    // Five kinds, five watches — never one list re-fetched as five rows.
    expect(watchResource).toHaveBeenCalledTimes(5);
  });

  it("reads a crash-looping pod's waiting reason in the row, not the phase that hides it", async () => {
    // The row already got its unhealthy dot from `podFlagged`, which asks
    // core. The label asked `row.phase` instead — and a pod whose container
    // is in a back-off loop still reports "Running", so the same row said
    // both "needs attention" and "Running". Both now read `podStatus`.
    watchResource.mockImplementation(
      async (_c: string, _n: string, kind: string, onRows: (rows: unknown[]) => void) => {
        onRows(
          kind === "pods"
            ? [{ name: "web-1", namespace: "default", phase: "Running", ready: "0/1", restarts: 7, node: "n1", age: "2d", image: "acme/web:1", waitingReason: "CrashLoopBackOff" }]
            : [],
        );
        return { stop };
      },
    );
    open();

    await waitFor(() => expect(rowNames()).toEqual(["Needs attentionweb-1"]));
    const row = screen.getByText("web-1").closest("tr")!;
    // The dot and the word now agree: the row says "needs attention" AND says
    // what is the matter, instead of saying "Running" beside its own dot.
    expect(within(row).getByText("Needs attention")).toBeTruthy();
    expect(within(row).getByText("CrashLoopBackOff")).toBeTruthy();
    expect(within(row).queryByText("Running")).toBeNull();
  });

  it("keeps that same pod flagged in the instant it is between restarts", async () => {
    // The moment the row above never modelled: no waiting reason, phase still
    // "Running", ready still 0/1. The row used to drop its dot and read a
    // plain green "Running" until the container failed again.
    watchResource.mockImplementation(
      async (_c: string, _n: string, kind: string, onRows: (rows: unknown[]) => void) => {
        onRows(
          kind === "pods"
            ? [{ name: "web-1", namespace: "default", phase: "Running", ready: "0/1", restarts: 7, node: "n1", age: "2d", image: "acme/web:1", waitingReason: "" }]
            : [],
        );
        return { stop };
      },
    );
    open();

    await waitFor(() => expect(rowNames()).toEqual(["Needs attentionweb-1"]));
    const row = screen.getByText("web-1").closest("tr")!;
    expect(within(row).getByText("Needs attention")).toBeTruthy();
    expect(within(row).getByText("NotReady")).toBeTruthy();
    expect(within(row).queryByText("Running")).toBeNull();
  });

  it("leaves a healthy pod reading its phase", async () => {
    open();
    await waitFor(() => expect(rowNames()).toHaveLength(5));
    const row = screen.getByText("web-1").closest("tr")!;
    expect(within(row).getByText("Running")).toBeTruthy();
  });

  it("narrows to one kind from the segment control", async () => {
    open();
    await waitFor(() => expect(rowNames()).toHaveLength(5));

    const callsBefore = watchResource.mock.calls.length;
    await userEvent.click(screen.getByRole("tab", { name: "Pod" }));

    await waitFor(() => expect(rowNames()).toEqual(["web-1"]));
    // Switching segments filters what's already in memory — it must not
    // reopen any of the five watches.
    expect(watchResource.mock.calls.length).toBe(callsBefore);

    await userEvent.click(screen.getByRole("tab", { name: "All" }));
    await waitFor(() => expect(rowNames()).toHaveLength(5));
  });

  it("sorts across kinds, not within them", async () => {
    open();
    await waitFor(() => expect(rowNames()).toHaveLength(5));

    await userEvent.click(screen.getByRole("button", { name: "Sort by Age" }));

    // Ascending by age in seconds: 1d, 2d, 10d, 30d, 120d — a run that only
    // exists by crossing every kind, since each kind here contributes just
    // one row.
    await waitFor(() =>
      expect(rowNames()).toEqual(["node-exporter", "web-1", "checkout", "db", "nightly-backup"]),
    );
  });

  it("opens the resource's /k/<kind>/<namespace>/<name> route, keyed by the row's own kind", async () => {
    open();
    await waitFor(() => expect(rowNames()).toHaveLength(5));

    // web-1 is a Pod in this union — its route must carry "Pod", not the
    // "/resources" route this list is itself opened at.
    fireEvent.doubleClick(screen.getByText("web-1").closest("tr")!);

    await waitFor(() => expect(tabFor("/k/Pod/default/web-1")).toBeTruthy());
    expect(store.currentWorkspace().activeId).toBe(tabFor("/k/Pod/default/web-1").id);
  });

  it("keeps listing the kinds that answered when one of the five fails", async () => {
    watchResource.mockImplementation(
      async (
        _context: string,
        _namespace: string,
        kind: string,
        onRows: (rows: unknown[]) => void,
        _onStatus: (status: "live" | "reconnecting") => void,
        onError: (message: string) => void,
      ) => {
        if (kind === "deployments") {
          onError("forbidden: cannot list deployments");
          return { stop };
        }
        onRows(FIXTURES[kind] ?? []);
        return { stop };
      },
    );

    open();

    await waitFor(() => expect(rowNames()).toHaveLength(4));
    expect(rowNames()).toEqual(
      expect.arrayContaining(["db", "node-exporter", "web-1", "nightly-backup"]),
    );
    expect(screen.queryByText("checkout")).toBeNull();

    expect(screen.getByText(/could not list deployments/i)).toBeTruthy();
    expect(screen.getByText(/forbidden: cannot list deployments/i)).toBeTruthy();
  });

  // Whole-branch review, Correction (a): zero options while `namespaces` is
  // still null reads as "this cluster has no namespaces" — a bare
  // `MultiSelect options={(namespaces ?? []).map(...)}` says exactly that.
  // `Resources.tsx` already shows a disabled, spinning stand-in instead; this
  // screen dropped the same treatment.
  it("shows the namespace picker as loading rather than empty before namespaces arrive", async () => {
    useNamespaceOptions.mockReturnValue({ namespaces: null, scope: "", error: "" });

    open();
    await waitFor(() => expect(rowNames()).toHaveLength(5));

    expect(screen.queryByRole("combobox", { name: "Namespaces" })).toBeNull();
    const placeholder = screen.getByRole("button", { name: "Namespaces" }) as HTMLButtonElement;
    expect(placeholder.disabled).toBe(true);
    expect(within(placeholder).getByRole("status", { name: "Loading namespaces" })).toBeTruthy();
  });

  // Whole-branch review, Correction (b): this screen destructured `{
  // namespaces, scope }` off `useNamespaceOptions` and dropped `error`
  // entirely, so a namespace-listing failure was silent. `Resources.tsx`
  // surfaces it in a warn Alert above the table without replacing the picker.
  it("warns above the table when namespace listing fails, without hiding the picker or the rows", async () => {
    useNamespaceOptions.mockReturnValue({
      namespaces: ["default", "kube-system"],
      scope: "",
      error: "namespaces: etcd timeout",
    });

    open();

    expect(await screen.findByText("Namespaces could not be listed")).toBeTruthy();
    expect(screen.getByText(/didn't respond in time/)).toBeTruthy();
    expect(document.querySelector('[data-slot="raw"]')?.textContent).toContain(
      "namespaces: etcd timeout",
    );
    expect(screen.getByRole("combobox", { name: "Namespaces" })).toBeTruthy();
    await waitFor(() => expect(rowNames()).toHaveLength(5));
  });

  // Whole-branch review, Correction (c): this screen rendered both its
  // failed-kind and stale Alerts inside `.scroll`, so they scrolled away with
  // the table. `Resources.tsx` hoists its banners out so they stay pinned.
  it("keeps a failed kind's alert outside the scrolling table body", async () => {
    watchResource.mockImplementation(
      async (
        _context: string,
        _namespace: string,
        kind: string,
        onRows: (rows: unknown[]) => void,
        _onStatus: (status: "live" | "reconnecting") => void,
        onError: (message: string) => void,
      ) => {
        if (kind === "deployments") {
          onError("forbidden: cannot list deployments");
          return { stop };
        }
        onRows(FIXTURES[kind] ?? []);
        return { stop };
      },
    );

    open();
    await waitFor(() => expect(screen.getByText(/could not list deployments/i)).toBeTruthy());

    const scrollBody = document.querySelector<HTMLElement>(".scroll")!;
    expect(within(scrollBody).queryByText(/could not list deployments/i)).toBeNull();
    // Still rendered — pinned above the scrolling body, not gone.
    expect(screen.getByText(/could not list deployments/i)).toBeTruthy();
    // And the kind that refused says what to do about it, not what the
    // apiserver called it. Four kinds still answered; this is a banner over
    // real rows, so it stays a warning rather than becoming an error state.
    expect(screen.getByText(/Check your RBAC roles/)).toBeTruthy();
    expect(document.querySelector('[data-slot="raw"]')?.textContent).toContain(
      "forbidden: cannot list deployments",
    );
  });

  // A union row's actions differ by kind — this is the per-row correctness
  // that matters: the menu is dispatched by `row.kind`, not by whichever
  // kind the segment control happens to be on, so it must never leak one
  // kind's actions onto another's row.
  it("offers a Pod row's own actions — logs and shell — and none of CronJob's", async () => {
    open();
    await waitFor(() => expect(rowNames()).toHaveLength(5));

    fireEvent.contextMenu(screen.getByText("web-1").closest("tr")!);

    expect(await screen.findByText("Follow logs")).toBeTruthy();
    expect(screen.getByText("Open shell")).toBeTruthy();
    expect(screen.getByText("Port forward")).toBeTruthy();
    expect(screen.getByText("Evict")).toBeTruthy();

    expect(screen.queryByText("Suspend")).toBeNull();
    expect(screen.queryByText("Run now")).toBeNull();
    expect(screen.queryByText("Scale")).toBeNull();
  });

  it("offers a CronJob row's own actions — suspend and run now — and none of Pod's", async () => {
    open();
    await waitFor(() => expect(rowNames()).toHaveLength(5));

    fireEvent.contextMenu(screen.getByText("nightly-backup").closest("tr")!);

    expect(await screen.findByText("Suspend")).toBeTruthy();
    expect(screen.getByText("Run now")).toBeTruthy();

    expect(screen.queryByText("Follow logs")).toBeNull();
    expect(screen.queryByText("Open shell")).toBeNull();
    expect(screen.queryByText("Port forward")).toBeNull();
    expect(screen.queryByText("Evict")).toBeNull();
    expect(screen.queryByText("Scale")).toBeNull();
  });

  it("opens a Deployment row's menu with scale and restart, and confirms a scale from the rendered screen", async () => {
    open();
    await waitFor(() => expect(rowNames()).toHaveLength(5));

    fireEvent.contextMenu(screen.getByText("checkout").closest("tr")!);
    expect(await screen.findByText("Scale")).toBeTruthy();
    expect(screen.getByText("Restart rollout")).toBeTruthy();
    expect(screen.queryByText("Suspend")).toBeNull();

    await userEvent.click(screen.getByText("Scale"));
    const dialog = await screen.findByRole("dialog");
    expect(dialog.textContent).toContain("checkout");
  });

  // Proves `WorkloadRow.suspended` is actually wired, not merely declared:
  // an unsuspended and a suspended CronJob side by side, so the label has to
  // follow each row's own state rather than reading the same either way.
  // (`isSuspended` treats a missing field exactly like `false`, which is why
  // a single always-unsuspended fixture couldn't catch this dropping.)
  it("labels the CronJob row menu by each row's own suspended state, and calls cronjobSetSuspend with the inverse", async () => {
    watchResource.mockImplementation(
      async (_c: string, _n: string, kind: string, onRows: (rows: unknown[]) => void) => {
        if (kind === "cronjobs") {
          onRows([
            { name: "nightly-backup", namespace: "default", schedule: "0 2 * * *", suspended: false, active: 0, lastSchedule: "2h ago", age: "120d" },
            { name: "paused-cleanup", namespace: "default", schedule: "0 3 * * *", suspended: true, active: 0, lastSchedule: "—", age: "60d" },
          ]);
          return { stop };
        }
        onRows(FIXTURES[kind] ?? []);
        return { stop };
      },
    );

    open();
    await waitFor(() => expect(rowNames()).toHaveLength(6));

    // Not suspended: the menu offers Suspend, not Resume.
    fireEvent.contextMenu(screen.getByText("nightly-backup").closest("tr")!);
    expect(await screen.findByText("Suspend")).toBeTruthy();
    expect(screen.queryByText("Resume")).toBeNull();
    await userEvent.click(screen.getByText("Suspend"));
    await userEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Suspend" }));
    await waitFor(() =>
      expect(cronjobSetSuspend).toHaveBeenCalledWith("prod-eu", "default", "nightly-backup", true),
    );

    // Already suspended: the menu offers Resume, not Suspend — and Resume
    // must call through with `suspend: false`, the inverse of the row's
    // current (suspended) state.
    fireEvent.contextMenu(screen.getByText("paused-cleanup").closest("tr")!);
    expect(await screen.findByText("Resume")).toBeTruthy();
    expect(screen.queryByText("Suspend")).toBeNull();
    await userEvent.click(screen.getByText("Resume"));
    await userEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Resume" }));
    await waitFor(() =>
      expect(cronjobSetSuspend).toHaveBeenCalledWith("prod-eu", "default", "paused-cleanup", false),
    );
  });
});

/**
 * The object a detail pane would have fetched for the same subject the row
 * describes — same replica counts, so the two readings are of one fact and any
 * difference between them is a disagreement rather than a difference of input.
 */
function workloadObject(kind: string, name: string, ready: number, desired: number): K8sObject {
  const status = kind === "DaemonSet"
    ? { numberReady: ready, desiredNumberScheduled: desired }
    : { readyReplicas: ready };
  return {
    kind,
    apiVersion: "apps/v1",
    metadata: { name, namespace: "default" },
    spec: kind === "DaemonSet" ? {} : { replicas: desired },
    status,
  } as K8sObject;
}

/** The row's status pill, as the word it shows and the tone it shows it in. */
function pillOf(row: HTMLElement): { status: string; kind: string | null } {
  const pill = row.querySelectorAll(".status");
  const last = pill[pill.length - 1] as HTMLElement;
  return { status: last.textContent ?? "", kind: last.getAttribute("data-kind") };
}

/**
 * The row and the detail header are two renderings of one verdict, and a
 * reader sees both — double-clicking the row opens the pane. They were derived
 * by different code: the header by core's `resourceStatusLine`, the row by a
 * local table pairing a label with a tone by hand. The local table had already
 * drifted on every case below. (#331)
 */
describe("Workloads rows and the detail header they open", () => {
  function listing(rows: Record<string, unknown[]>) {
    watchResource.mockImplementation(
      async (_c: string, _n: string, kind: string, onRows: (r: unknown[]) => void) => {
        onRows(rows[kind] ?? []);
        return { stop };
      },
    );
  }

  it("calls a degraded Deployment what the header calls it, in the tone its own dot uses", async () => {
    listing({ deployments: [{ name: "checkout", namespace: "default", ready: "1/3", upToDate: 1, available: 1, age: "10d" }] });
    open();
    await waitFor(() => expect(rowNames()).toHaveLength(1));

    const header = resourceStatusLine("Deployment", workloadObject("Deployment", "checkout", 1, 3))!;
    const row = screen.getByText("checkout").closest("tr")!;
    // The header reads "Degraded", danger, flagged — and the row draws the
    // very same object.
    expect(header.status).toBe("Degraded");
    expect(header.flagged).toBe(true);
    expect(pillOf(row)).toEqual({ status: header.status, kind: header.health });
    // The dot beside it: an amber word next to a red dot is the contradiction
    // the kit fixed in `Inspector`, and it was reproduced here.
    expect(within(row).getByText("Needs attention")).toBeTruthy();
  });

  it("calls a Deployment scaled to zero what the header calls it", async () => {
    listing({ deployments: [{ name: "idle", namespace: "default", ready: "0/0", upToDate: 0, available: 0, age: "10d" }] });
    open();
    await waitFor(() => expect(rowNames()).toHaveLength(1));

    const header = resourceStatusLine("Deployment", workloadObject("Deployment", "idle", 0, 0))!;
    expect(header.status).toBe("Scaled down");
    expect(pillOf(screen.getByText("idle").closest("tr")!)).toEqual({
      status: header.status,
      kind: header.health,
    });
  });

  it("calls a StatefulSet short of its replicas what the header calls it", async () => {
    listing({ statefulsets: [{ name: "db", namespace: "default", ready: "1/2", updated: 1, service: "db-svc", age: "30d" }] });
    open();
    await waitFor(() => expect(rowNames()).toHaveLength(1));

    const header = resourceStatusLine("StatefulSet", workloadObject("StatefulSet", "db", 1, 2))!;
    expect(header.status).toBe("Degraded");
    expect(pillOf(screen.getByText("db").closest("tr")!)).toEqual({
      status: header.status,
      kind: header.health,
    });
  });

  it("calls a DaemonSet scheduled on no node what the header calls it — not the replica word", async () => {
    listing({ daemonsets: [{ name: "gpu-agent", namespace: "kube-system", desired: 0, current: 0, ready: 0, upToDate: 0, available: 0, age: "1d" }] });
    open();
    await waitFor(() => expect(rowNames()).toHaveLength(1));

    const header = resourceStatusLine("DaemonSet", workloadObject("DaemonSet", "gpu-agent", 0, 0))!;
    // A DaemonSet matching no node is "Not scheduled", not "Scaled down":
    // the zero word is the kind's, and only core knows which kind uses which.
    expect(header.status).toBe("Not scheduled");
    expect(pillOf(screen.getByText("gpu-agent").closest("tr")!)).toEqual({
      status: header.status,
      kind: header.health,
    });
  });

  it("calls a healthy Deployment what the header calls it", async () => {
    listing({ deployments: DEPLOYMENTS });
    open();
    await waitFor(() => expect(rowNames()).toHaveLength(1));

    const header = resourceStatusLine("Deployment", workloadObject("Deployment", "checkout", 2, 2))!;
    expect(header.status).toBe("Running");
    expect(pillOf(screen.getByText("checkout").closest("tr")!)).toEqual({
      status: header.status,
      kind: header.health,
    });
    expect(within(screen.getByText("checkout").closest("tr")!).queryByText("Needs attention")).toBeNull();
  });

  it("calls a suspended CronJob what the header calls it", async () => {
    listing({ cronjobs: [{ name: "paused-cleanup", namespace: "default", schedule: "0 3 * * *", suspended: true, active: 0, lastSchedule: "-", age: "60d" }] });
    open();
    await waitFor(() => expect(rowNames()).toHaveLength(1));

    const header = resourceStatusLine("CronJob", {
      kind: "CronJob",
      apiVersion: "batch/v1",
      metadata: { name: "paused-cleanup", namespace: "default" },
      spec: { suspend: true },
      status: {},
    } as K8sObject)!;
    expect(pillOf(screen.getByText("paused-cleanup").closest("tr")!)).toEqual({
      status: header.status,
      kind: header.health,
    });
  });
});
