import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

/**
 * **Only the capability wrappers and the two platform reads are replaced.**
 *
 * `describeError`, `plural` and `contextDisplayName` stay real, so the failure
 * copy and both halves of the sub-count are core's own arithmetic rather than a
 * copy of it. `latencyLabel`, `viaOf` and `joined` stay real for the same
 * reason — they are what the table and the rail already agree on.
 */
const core = vi.hoisted(() => ({
  listContexts: vi.fn(),
  connectCluster: vi.fn(),
  clusterFacts: vi.fn(),
  isTauri: vi.fn(),
  pickKubeconfigFiles: vi.fn(),
  saveKubeconfigFiles: vi.fn(),
}));
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  ...core,
}));

import { describeError, plural, type ClusterContext, type ClusterFacts, type ClusterInfo } from "@srelens/core";
import { Connections } from "./Connections";
import { getKubeconfigFiles, resetContexts, setContexts, setKubeconfigFiles } from "../lib/clusters";
import { resetProbes } from "../lib/probe";
import { defaultState } from "../lib/tabs";
import * as store from "../lib/tabsStore";
import { resetView } from "../lib/workspace";

const ROUTE = "/connections";

/** The kubeconfig two of the three contexts were declared in. */
const CONFIG = "/home/dana/.kube/config";

const PROD: ClusterContext = {
  name: "prod-eu",
  stableId: "prod-eu",
  cluster: "prod",
  server: "https://prod-eu.example:6443",
  isCurrent: true,
  namespace: "platform",
  sourceFile: CONFIG,
  authKind: "exec plugin · gcloud",
};

const STAGING: ClusterContext = {
  name: "staging-eu",
  stableId: "staging-eu",
  cluster: "staging",
  server: "https://staging-eu.example:6443",
  isCurrent: false,
  sourceFile: CONFIG,
  authKind: "token",
};

/**
 * A third context from a SECOND file, used only where a second source matters.
 *
 * The default fixture deliberately has two clusters in one file, because that
 * is the shape the sub-count can get wrong: a count of the stored file list is
 * `0` here (this window stores none — see {@link FILES}) while the rail draws
 * one row, and a count of the rows is `2` where the files are `1`.
 */
const EDGE: ClusterContext = {
  name: "edge-1",
  stableId: "edge-1",
  cluster: "edge",
  server: "https://edge-1.example:6443",
  isCurrent: false,
  sourceFile: "/home/dana/work/edge.yaml",
  authKind: "client certificate",
};

/**
 * **No stored kubeconfig files**, which is web mode's own shape (`Window.tsx`
 * hands it `[]`) and the one that separates "count the stored list" from "count
 * what the rail draws". The desktop tests set their own.
 */
const FILES: string[] = [];

const facts = (over: Partial<ClusterFacts> = {}): ClusterFacts => ({
  context: "prod-eu",
  provider: "gke",
  region: "europe-west4",
  metricsServer: { state: "present", version: "v0.7.1" },
  ...over,
});

/** What each cluster answers, and how long it takes about it. */
const REACHABLE: Record<string, ClusterInfo> = {
  "prod-eu": { context: "prod-eu", reachable: true, version: "v1.31.2" },
  "staging-eu": { context: "staging-eu", reachable: true, version: "v1.30.6" },
  "edge-1": { context: "edge-1", reachable: true, version: "v1.29.4" },
};
const LATENCY: Record<string, number> = { "prod-eu": 41, "staging-eu": 12, "edge-1": 7 };

/**
 * A listing failure with a machine string in it.
 *
 * Classified by `describeError` (it matches its connect branch), so the screen
 * has a sentence to show and an original to fold away — which is the only shape
 * that can tell "reported through `describeError`" apart from "printed the
 * backend's string".
 */
const LIST_FAILURE = "ServiceError: connection refused: ECONNREFUSED 10.0.5.2:6443";

/**
 * A frozen clock, advanced only by the `connectCluster` double.
 *
 * `probeCluster` times its own round trip off `Date.now` and the screen calls
 * it with the default, so this is the only way a latency assertion can be an
 * exact number rather than whatever the machine happened to take.
 */
let clock = 0;

function deferred<T>() {
  let settle: (value: T) => void = () => {};
  const promise = new Promise<T>((resolve) => {
    settle = resolve;
  });
  return { promise, settle };
}

/** A promise that never settles — a cluster that does not answer at all. */
const never = <T,>(): Promise<T> => new Promise<T>(() => {});

beforeEach(() => {
  vi.clearAllMocks();
  clock = 1_700_000_000_000;
  vi.spyOn(Date, "now").mockImplementation(() => clock);
  core.listContexts.mockResolvedValue({ contexts: [PROD, STAGING] });
  core.connectCluster.mockImplementation(async (name: string) => {
    clock += LATENCY[name] ?? 0;
    return REACHABLE[name] ?? { context: name, reachable: false, error: "no route to host" };
  });
  core.clusterFacts.mockImplementation(async (context: string) => facts({ context }));
  // Web by default, so a desktop-only control has to be asked for explicitly.
  core.isTauri.mockReturnValue(false);
  core.pickKubeconfigFiles.mockResolvedValue([]);
  resetProbes();
  resetContexts();
  setContexts([PROD, STAGING]);
  setKubeconfigFiles(FILES);
  store.setState(defaultState([PROD, STAGING]));
  resetView();
});

afterEach(() => {
  vi.restoreAllMocks();
});

function open() {
  store.openTab(ROUTE);
  return render(<Connections route={ROUTE} />);
}

/** The `Screen`'s sub, exactly as it is drawn. */
const sub = () => document.querySelector(".crumb")?.textContent ?? null;

/**
 * Every cluster the table is drawing, in the order it draws them.
 *
 * Read from the name element rather than from the cell's `textContent`: the
 * Cluster cell also carries the mark's initials and the facts line, so the
 * cell's text is `PEprod-eugke · v1.31.2 · europe-west4` and an equality
 * against it would pass for reasons that have nothing to do with the row.
 */
const drawn = () =>
  Array.from(document.querySelectorAll("tbody tr.tbl-row")).map(
    (tr) => tr.querySelector('[data-testid^="cluster-name-"]')?.textContent?.trim() ?? "",
  );

/** One row, by the cluster its name cell names — exactly, not by inclusion. */
const rowFor = (name: string) =>
  Array.from(document.querySelectorAll("tbody tr.tbl-row")).find(
    (tr) => tr.querySelector('[data-testid^="cluster-name-"]')?.textContent?.trim() === name,
  ) as HTMLElement;

/** The file rows the Sources rail is drawing. */
const sourceRows = () => Array.from(document.querySelectorAll('[data-testid="source-file"]'));

const refreshAll = () => screen.getByRole("button", { name: "Refresh all" });

describe("Connections", () => {
  it("counts the clusters it is showing, and the files they came from", async () => {
    open();
    expect(await screen.findByText(/2 clusters · 1 source/)).toBeTruthy();
  });

  /**
   * The count and the rows, pinned to each other rather than each to a literal.
   *
   * `Releases · 383 in this cluster` over six rows is the defect this guards,
   * and a pair of hard-coded numbers cannot guard it: they agree with the
   * fixture, not with the screen. Both halves are read back off the DOM the
   * reader is looking at — the table's rows and the rail's file rows — so a
   * count taken from anything else fails here whatever the fixture says.
   */
  it("counts what is on the screen, not the lists behind it", async () => {
    setContexts([PROD, STAGING, EDGE]);
    open();
    await waitFor(() => expect(drawn().length).toBe(3));
    // Two files: the one PROD and STAGING share, and EDGE's own. Neither is in
    // the stored list, which is empty — so `files.length` is 0 here.
    expect(getKubeconfigFiles()).toEqual([]);
    expect(sourceRows().length).toBe(2);
    expect(sub()).toBe(`${plural(drawn().length, "cluster")} · ${plural(sourceRows().length, "source")}`);
  });

  it("draws the table before any cluster has answered", async () => {
    core.connectCluster.mockImplementation(() => never<ClusterInfo>());
    open();
    expect(await screen.findByText("prod-eu")).toBeTruthy();
    expect(screen.getByText("staging-eu")).toBeTruthy();
    // No reading, and specifically not a reading of zero.
    expect(screen.queryByText(/\d+\s*ms/)).toBeNull();
    expect(screen.getAllByText("no reading").length).toBe(2);
  });

  it("one cluster that does not answer does not hold up the others", async () => {
    core.connectCluster.mockImplementation(async (name: string) => {
      if (name === "prod-eu") return never<ClusterInfo>();
      clock += LATENCY[name] ?? 0;
      return REACHABLE[name];
    });
    open();
    // The reachable one's own round trip, while the other is still out.
    expect(await screen.findByText("12 ms")).toBeTruthy();
    expect(within(rowFor("staging-eu")).getByText("reachable")).toBeTruthy();
    expect(within(rowFor("prod-eu")).getByText("no reading")).toBeTruthy();
  });

  /**
   * The second line of the Cluster cell, which is the whole reason the facts
   * are fetched at all: provider, server version and region, from the two
   * different round trips that answer them.
   */
  it("fills in the control-plane facts once a cluster has answered", async () => {
    open();
    expect(await screen.findByText("gke · v1.30.6 · europe-west4")).toBeTruthy();
    expect(core.clusterFacts).toHaveBeenCalledWith("staging-eu");
  });

  it("does not ask a cluster that did not answer for its facts", async () => {
    core.connectCluster.mockImplementation(async (name: string) => ({
      context: name,
      reachable: false,
      error: "no route to host",
    }));
    open();
    await waitFor(() => expect(screen.getAllByText("unreachable").length).toBe(2));
    expect(core.clusterFacts).not.toHaveBeenCalled();
  });

  it("reports a listing that fails, through describeError", async () => {
    resetContexts();
    setContexts([], LIST_FAILURE);
    open();
    const alert = await screen.findByRole("alert");
    expect(within(alert).getByText("Could not list your clusters")).toBeTruthy();
    // The classification, word for word — not a sentence this screen wrote.
    expect(within(alert).getByText(describeError(LIST_FAILURE).detail)).toBeTruthy();
    /**
     * **The backend's own string is never what the reader is shown.**
     *
     * Asserted on the detail slot rather than on the whole document: the
     * original is deliberately offered, folded away inside `RawError`'s
     * disclosure, and `queryByText` cannot tell a closed `details` from a
     * paragraph. What must not happen is the machine string standing in for the
     * sentence — so the element that carries the sentence is the one asked.
     */
    const detail = alert.querySelector('[data-slot="detail"]');
    expect(detail?.textContent ?? "").not.toContain("ECONNREFUSED");
    expect(alert.querySelector('[data-slot="raw"] pre')?.textContent).toContain("ECONNREFUSED");
  });

  /**
   * A count over a screen with no table on it would be a count of nothing,
   * asserted as a fact. The sub says nothing until there are rows to say it
   * about.
   */
  it("claims no count while the clusters are still being listed", () => {
    resetContexts();
    open();
    expect(sub()).toBeNull();
    expect(screen.getByRole("status")).toBeTruthy();
  });

  it("claims no count over a listing that failed", async () => {
    resetContexts();
    setContexts([], LIST_FAILURE);
    open();
    await screen.findByRole("alert");
    expect(sub()).toBeNull();
  });

  it("sends a reader with no clusters to connect rather than an empty table", async () => {
    resetContexts();
    setContexts([]);
    open();
    expect(await screen.findByRole("button", { name: "Connect a cluster" })).toBeTruthy();
    expect(screen.getByText("No clusters yet")).toBeTruthy();
    // Not an empty table with six column headers over it.
    expect(document.querySelector('[data-testid="cluster-table"]')).toBeNull();
    expect(sub()).toBeNull();
  });

  it("opens the connect route from the empty state", async () => {
    resetContexts();
    setContexts([]);
    const user = userEvent.setup();
    open();
    await user.click(await screen.findByRole("button", { name: "Connect a cluster" }));
    expect(store.currentWorkspace().tabs.some((t) => t.route === "/connect")).toBe(true);
  });

  it("opens the connect route from the header", async () => {
    const user = userEvent.setup();
    open();
    await user.click(screen.getByRole("button", { name: "Add connection" }));
    expect(store.currentWorkspace().tabs.some((t) => t.route === "/connect")).toBe(true);
  });

  it("re-lists and re-reads every cluster on Refresh all", async () => {
    const user = userEvent.setup();
    open();
    await waitFor(() => expect(screen.getByText("12 ms")).toBeTruthy());
    expect(core.connectCluster).toHaveBeenCalledTimes(2);

    core.listContexts.mockResolvedValue({ contexts: [PROD, STAGING, EDGE] });
    await user.click(refreshAll());

    await waitFor(() => expect(drawn()).toEqual(["prod-eu", "staging-eu", "edge-1"]));
    /**
     * **Every cluster read again, not only the one that is new.**
     *
     * Counted per cluster rather than asserted as a new latency: the clock is
     * one number shared by three probes that run at once, so a per-cluster
     * millisecond figure is only exact when a single cluster answers. The call
     * count is exact whatever the interleaving, and it is the property — a
     * `Refresh all` that re-read only the unread clusters would leave twenty
     * stale readings under a control that says it refreshed them.
     */
    await waitFor(() => {
      for (const name of ["prod-eu", "staging-eu", "edge-1"]) {
        expect(core.connectCluster.mock.calls.filter((c) => c[0] === name).length).toBe(
          name === "edge-1" ? 1 : 2,
        );
      }
    });
    expect(core.connectCluster).toHaveBeenCalledTimes(5);
    // And their facts with them: the round trip is re-made, not remembered.
    expect(core.clusterFacts.mock.calls.filter((c) => c[0] === "staging-eu").length).toBe(2);
  });

  /**
   * **The sequence guard.** A reader who hits `Refresh all` twice must be left
   * looking at the second answer, whatever order the two listings come back in.
   */
  it("a listing that answers late cannot paint over a fresh one", async () => {
    const slow = deferred<{ contexts: ClusterContext[] }>();
    core.listContexts
      .mockReturnValueOnce(slow.promise)
      .mockResolvedValueOnce({ contexts: [PROD] });
    const user = userEvent.setup();
    open();
    await waitFor(() => expect(drawn().length).toBe(2));

    await user.click(refreshAll());
    await user.click(refreshAll());
    await waitFor(() => expect(drawn()).toEqual(["prod-eu"]));

    // The first listing answers now, with a list the reader has moved on from.
    await act(async () => {
      slow.settle({ contexts: [PROD, STAGING, EDGE] });
    });
    expect(drawn()).toEqual(["prod-eu"]);
  });

  /**
   * The same guard on the second round trip. Facts are per cluster and land
   * after their probe, so a slow one can answer under a listing that has since
   * been replaced.
   */
  it("a facts answer that arrives after a re-listing cannot paint over it", async () => {
    const slow = deferred<ClusterFacts>();
    // Only the FIRST read of staging is slow. A double that stayed slow would
    // hand the second round the same pending promise, and settling it would
    // then be a fresh answer arriving — which tests nothing.
    let staged = 0;
    core.clusterFacts.mockImplementation(async (context: string) =>
      context === "staging-eu" && ++staged === 1 ? slow.promise : facts({ context }),
    );
    const user = userEvent.setup();
    open();
    await waitFor(() => expect(drawn().length).toBe(2));

    core.listContexts.mockResolvedValue({ contexts: [PROD, STAGING] });
    await user.click(refreshAll());
    await waitFor(() => expect(core.connectCluster).toHaveBeenCalledTimes(4));

    await act(async () => {
      slow.settle(facts({ context: "staging-eu", provider: "eks", region: "eu-west-1" }));
    });
    // The round that asked for it has been superseded, so its answer is dropped
    // rather than drawn under a listing it does not belong to.
    expect(screen.queryByText(/eks/)).toBeNull();
  });

  /**
   * Two unordered writes of one cluster's reading, refused at the source.
   *
   * `probeCluster` writes to a module store the screen cannot un-write, so the
   * guard here is not to start a second read of a cluster whose first is still
   * out: an answer that is already on its way IS the fresh reading.
   */
  it("does not start a second read of a cluster that is still answering", async () => {
    const slow = deferred<ClusterInfo>();
    core.connectCluster.mockImplementation(async (name: string) => {
      if (name === "prod-eu") return slow.promise;
      clock += LATENCY[name] ?? 0;
      return REACHABLE[name];
    });
    const user = userEvent.setup();
    open();
    await waitFor(() => expect(screen.getByText("12 ms")).toBeTruthy());

    await user.click(refreshAll());
    await waitFor(() =>
      expect(core.connectCluster.mock.calls.filter((c) => c[0] === "staging-eu").length).toBe(2),
    );
    expect(core.connectCluster.mock.calls.filter((c) => c[0] === "prod-eu").length).toBe(1);

    await act(async () => {
      clock += 41;
      slow.settle(REACHABLE["prod-eu"]);
    });
    expect(within(rowFor("prod-eu")).getByText("reachable")).toBeTruthy();
  });

  /**
   * **The desktop's file picker, which the rail cannot ask for itself.**
   *
   * `SourcesRail` treats an absent `onAddFile` as "there is no filesystem to
   * browse" and prints that instead of the control, so a screen that forgets to
   * pass it loses the button on the desktop with nothing saying why.
   */
  it("offers the file picker on the desktop", async () => {
    core.isTauri.mockReturnValue(true);
    open();
    expect(await screen.findByRole("button", { name: "Add" })).toBeTruthy();
    expect(screen.queryByText(/added on the desktop/)).toBeNull();
  });

  it("says why there is no file picker in the browser", async () => {
    open();
    expect(await screen.findByText(/Kubeconfig files are added on the desktop/)).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Add" })).toBeNull();
  });

  it("writes an added file through saveKubeconfigFiles and lists again", async () => {
    core.isTauri.mockReturnValue(true);
    core.pickKubeconfigFiles.mockResolvedValue(["/home/dana/work/edge.yaml"]);
    core.listContexts.mockResolvedValue({ contexts: [PROD, STAGING] });
    const user = userEvent.setup();
    open();
    await waitFor(() => expect(drawn().length).toBe(2));

    core.listContexts.mockResolvedValue({ contexts: [PROD, STAGING, EDGE] });
    await user.click(await screen.findByRole("button", { name: "Add" }));

    await waitFor(() => expect(drawn().length).toBe(3));
    expect(core.saveKubeconfigFiles).toHaveBeenCalledWith(["/home/dana/work/edge.yaml"]);
    // The backend has to be told the path before a client can be built for a
    // context that came out of it, so the re-listing carries it.
    expect(core.listContexts).toHaveBeenLastCalledWith(["/home/dana/work/edge.yaml"]);
    expect(getKubeconfigFiles()).toEqual(["/home/dana/work/edge.yaml"]);
  });

  it("reports a file picker that fails, through describeError", async () => {
    core.isTauri.mockReturnValue(true);
    core.pickKubeconfigFiles.mockRejectedValue(new Error("no route to host"));
    const user = userEvent.setup();
    open();
    await user.click(await screen.findByRole("button", { name: "Add" }));
    const alert = await screen.findByRole("alert");
    expect(within(alert).getByText("Could not add that kubeconfig file")).toBeTruthy();
    expect(within(alert).getByText(describeError("no route to host").detail)).toBeTruthy();
    // The rows are untouched: this failed before anything was written.
    expect(drawn().length).toBe(2);
    expect(core.saveKubeconfigFiles).not.toHaveBeenCalled();
  });

  it("keeps the clusters it has when a re-listing fails, and says so", async () => {
    const user = userEvent.setup();
    open();
    await waitFor(() => expect(drawn().length).toBe(2));

    core.listContexts.mockResolvedValue({ error: LIST_FAILURE });
    await user.click(refreshAll());

    // `status`, not `alert`: the rows are still there, so this is a warning
    // over content rather than a stop — `errorCopy`'s own rule, and the tone is
    // what decides the role.
    const alert = await screen.findByRole("status");
    expect(within(alert).getByText("Could not refresh your clusters")).toBeTruthy();
    // Still on screen, and still counted: a refresh that failed took nothing
    // away from the reader.
    expect(drawn().length).toBe(2);
    expect(sub()).toBe("2 clusters · 1 source");
  });

  it("opens a cluster on the cluster the row is about", async () => {
    // STAGING is listed but not in the workspace, which is the case an `Open`
    // that only set the focus would silently do nothing for.
    store.setState(defaultState([PROD]));
    const user = userEvent.setup();
    open();
    await waitFor(() => expect(drawn().length).toBe(2));

    await user.click(within(rowFor("staging-eu")).getByRole("button", { name: "Open" }));
    expect(store.activeCluster()).toBe("staging-eu");
    expect(store.currentWorkspace().tabs.some((t) => t.route === "/overview")).toBe(true);
    expect(store.currentWorkspace().clusters).toContain("staging-eu");
  });

  /**
   * **`min-width: auto` is why this is asserted as a class list.**
   *
   * A flex item refuses to shrink below its content, so without `min-w-0` on
   * the table's column a long kubeconfig path pushes the rail's fixed 292px off
   * the window and the whole screen scrolls sideways. jsdom computes no layout,
   * so the classes are the only thing a test can hold.
   */
  it("lets the table column shrink so the rail keeps its width", async () => {
    open();
    await waitFor(() => expect(drawn().length).toBe(2));
    const main = document.querySelector('[data-slot="connections-main"]') as HTMLElement;
    expect(main.className.split(/\s+/)).toEqual(
      expect.arrayContaining(["flex", "min-h-0", "min-w-0", "flex-1", "flex-col"]),
    );
    const table = document.querySelector('[data-testid="cluster-table"]') as HTMLElement;
    expect(table.className.split(/\s+/)).toEqual(
      expect.arrayContaining(["scroll", "min-h-0", "min-w-0", "flex-1"]),
    );
    expect((document.querySelector("aside.side-rail") as HTMLElement).style.width).toBe("292px");
  });
});
