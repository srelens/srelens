import { useEffect } from "react";
import { describe, it, expect, vi, afterEach, beforeEach } from "vitest";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

// Everything the screen reaches into core for. `watchResource` is held open by
// the test rather than resolved once: half of what this screen does is react to
// a stream that keeps arriving, and a mock that only answers the first call
// cannot say anything about the second snapshot or a dropped connection.
const {
  watchResource,
  listCrds,
  listCustomResource,
  listNodes,
  nodeMetrics,
  podMetrics,
  useNamespaceOptions,
  deleteResource,
  getObject,
} = vi.hoisted(() => ({
  watchResource: vi.fn(),
  listCrds: vi.fn(),
  listCustomResource: vi.fn(),
  listNodes: vi.fn(),
  nodeMetrics: vi.fn(),
  podMetrics: vi.fn(),
  useNamespaceOptions: vi.fn(),
  deleteResource: vi.fn(async (): Promise<{ ok?: boolean; error?: string }> => ({ ok: true })),
  // The one read behind both detail hosts (`useObject`). Counting its calls
  // is the only way to say "the peek did not refetch" — a rendered heading
  // looks identical whether or not a second round trip went out.
  getObject: vi.fn(),
}));

vi.mock("@srelens/core", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@srelens/core")>()),
  watchResource: (...a: unknown[]) => watchResource(...a),
  listCrds: (...a: unknown[]) => listCrds(...a),
  listCustomResource: (...a: unknown[]) => listCustomResource(...a),
  listNodes: (...a: unknown[]) => listNodes(...a),
  nodeMetrics: (...a: unknown[]) => nodeMetrics(...a),
  podMetrics: (...a: unknown[]) => podMetrics(...a),
  deleteResource,
  getObject: (...a: unknown[]) => getObject(...a),
}));

/**
 * `ResourceDetailView` itself, unchanged — wrapped only to record the props
 * each host hands it. The two hosts mount two different screens now, so what
 * is captured is the SUBJECT each was pointed at: the only way to assert
 * structurally (rather than by eyeballing two rendered trees, which are meant
 * to differ) that one row and one route resolve to one resource.
 *
 * `createElement` rather than JSX inside the factory: `vi.mock` factories are
 * hoisted above this module's own imports, and the JSX runtime binding is not
 * guaranteed to be initialised when the factory runs.
 */
const { detailProps, detailFrames, tabProps } = vi.hoisted(() => ({
  detailProps: [] as Array<Record<string, unknown>>,
  detailFrames: [] as Array<{ heading: string | null; body: string }>,
  /** The same, for the OTHER host. R-5 is retired and the full tab is its own
   *  screen, so it is its own component and its own recorder — what the two
   *  must still agree on is the SUBJECT, which is what is compared. */
  tabProps: [] as Array<Record<string, unknown>>,
}));

vi.mock("./detail/ResourceDetailView", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./detail/ResourceDetailView")>();
  const { createElement, useLayoutEffect } = await import("react");
  return {
    ...actual,
    ResourceDetailView: (props: Record<string, unknown>) => {
      detailProps.push({ ...props });
      // The frame probe lives HERE, wrapped directly around the pane, because
      // this is the only component that re-renders when the peek's subject
      // changes. A probe rendered as a sibling of `<Resources>` does not: the
      // peek is state inside `KindList`, so the sibling never re-renders, its
      // layout effect never fires again, and the recording comes back empty —
      // an assertion over nothing, which is what the first cut of this test
      // was. Layout effects run bottom-up, so by the time this one fires the
      // pane below it has already committed its DOM: what it reads is the
      // frame a browser would paint.
      useLayoutEffect(() => {
        const pane = document.querySelector("section.pane");
        detailFrames.push({
          heading: pane?.querySelector("h2")?.textContent ?? null,
          body: pane?.querySelector(".pane-body")?.textContent ?? "",
        });
      });
      return createElement(actual.ResourceDetailView, props as never);
    },
  };
});

vi.mock("./detail/ResourceTabView", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./detail/ResourceTabView")>();
  const { createElement } = await import("react");
  return {
    ...actual,
    ResourceTabView: (props: Record<string, unknown>) => {
      tabProps.push({ ...props });
      return createElement(actual.ResourceTabView, props as never);
    },
  };
});

vi.mock("@srelens/core/react", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@srelens/core/react")>()),
  useNamespaceOptions: (...a: unknown[]) => useNamespaceOptions(...a),
}));

// jsdom has neither, and both pickers on this screen are Radix popovers.
// Inert stubs: jsdom does no layout, so there is never a resize to report.
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

import type { ClusterContext, CrdRef, K8sObject } from "@srelens/core";
import { ResourceDetailScreen, Resources } from "./Resources";
import { ConsoleProvider, useConsole } from "../console";
import * as store from "../lib/tabsStore";
import { defaultState } from "../lib/tabs";
import { resetContexts, setContexts, setKubeconfigFiles } from "../lib/clusters";
import { hiddenColumns, loadColumnPrefs, toggleColumn } from "../lib/columnPrefs";
import {
  DEFAULT_PEEK_WIDTH,
  MAX_PEEK_WIDTH,
  MIN_LIST_WIDTH,
  MIN_PEEK_WIDTH,
  PEEK_WIDTH_KEY,
  loadPeekWidth,
} from "../lib/peekWidth";
import { resetListCache } from "../lib/resourceList";
import { getView, resetView, setNamespaces } from "../lib/workspace";

const CTX: ClusterContext = {
  name: "prod-eu",
  stableId: "prod",
  cluster: "prod",
  server: "https://prod",
  isCurrent: true,
  sourceFile: "/home/dana/.kube/config",
  authKind: "client certificate",
};

/**
 * A SECOND cluster, for the rail switch. Same shape as `CTX` and a different
 * `stableId`, which is what the workspace holds and what a namespace selection
 * is filed under.
 */
const STAGE: ClusterContext = {
  name: "stage-eu",
  stableId: "stage",
  cluster: "stage",
  server: "https://stage",
  isCurrent: false,
  sourceFile: "/home/dana/.kube/config",
  authKind: "client certificate",
};

const PODS = [
  { name: "web-1", namespace: "default", ready: "1/1", phase: "Running", restarts: 3, node: "n1", age: "2d", image: "acme/checkout-api:118a7e" },
  { name: "api-7", namespace: "billing", ready: "1/1", phase: "Running", restarts: 1, node: "n2", age: "5d", image: "redis:7.4-alpine" },
];

const WIDGETS: CrdRef = {
  name: "widgets.example.com",
  group: "example.com",
  version: "v1",
  kind: "Widget",
  plural: "widgets",
  namespaced: true,
  printerColumns: [{ name: "Phase", jsonPath: ".status.phase", type: "string" }],
};

/** The live watch: what the screen was handed, so a test can push into it. */
let stream: {
  rows: (rows: unknown[]) => void;
  status: (status: "live" | "reconnecting") => void;
  error: (message: string) => void;
};
let stop: ReturnType<typeof vi.fn>;

beforeEach(() => {
  vi.clearAllMocks();
  stop = vi.fn();
  asked = [];
  watchResource.mockImplementation(
    async (
      _context: string,
      _namespace: string,
      _kind: string,
      onRows: (rows: unknown[]) => void,
      onStatus: (status: "live" | "reconnecting") => void,
      onError: (message: string) => void,
    ) => {
      stream = { rows: onRows, status: onStatus, error: onError };
      onRows(PODS);
      return { stop };
    },
  );
  listCrds.mockResolvedValue({ crds: [] });
  listCustomResource.mockResolvedValue({ items: [] });
  listNodes.mockResolvedValue({ nodes: [] });
  nodeMetrics.mockResolvedValue({ metrics: [] });
  podMetrics.mockResolvedValue({ metrics: [] });
  useNamespaceOptions.mockReturnValue({ namespaces: ["default", "billing"], scope: "", error: "" });
  // Every row resolves to an object that names itself, so a pane's heading
  // (which comes from the props) and its body (which comes from this read)
  // can be told apart when they disagree.
  getObject.mockImplementation(
    async (_context: string, kind: string, namespace: string | null, name: string) => ({
      object: {
        kind,
        apiVersion: "v1",
        metadata: { name, ...(namespace ? { namespace } : {}) },
      } satisfies K8sObject,
    }),
  );
  detailProps.length = 0;
  detailFrames.length = 0;
  tabProps.length = 0;

  resetContexts();
  setContexts([CTX]);
  setKubeconfigFiles(["/home/u/.kube/config"]);
  store.setState(defaultState([CTX]));
  resetView();
  resetListCache();
  // The preferences are module state that outlives a test; localStorage is
  // cleared by the shared setup, so re-reading it is a reset.
  loadColumnPrefs();
  loadPeekWidth();
});

/**
 * The name cell of every rendered row, in the order they are on screen.
 *
 * Not `td:first-child`: `Table`'s optional bulk-selection checkbox (wired in
 * this screen since the bulk action bar landed) is a real leading `<td
 * class="tbl-check">`, so the first *child* is the checkbox whenever a
 * selection is active and the name only holds `:nth-child(2)` by accident of
 * today's column order. Skipping `.tbl-check` instead reads the first *data*
 * cell whether or not the checkbox column is there, and keeps reading the
 * name correctly if a future column is ever added or removed ahead of it.
 */
const rowNames = () =>
  Array.from(document.querySelectorAll("tbody tr.tbl-row")).map(
    (row) => row.querySelector("td:not(.tbl-check)")?.textContent ?? null,
  );

const headers = () =>
  Array.from(document.querySelectorAll("thead th .th-sort span")).map((el) => el.textContent);

/** The tab a route is open in — the one the screen under test is bound to. */
const tabFor = (route: string) => store.currentWorkspace().tabs.find((t) => t.route === route)!;

/** Every question a row's ask chip has sent to the console, in order asked. */
let asked: string[];

/** Stands in for the dock: registers as the console's listener, the way
 *  `ConsoleDock` does, and records what arrives instead of rendering it. */
function AskPeek() {
  const { registerSubmit } = useConsole();
  useEffect(() => registerSubmit((question) => asked.push(question)), [registerSubmit]);
  return null;
}

/**
 * Open the route in a tab, then render its screen — the way `Window` does it.
 * The screen reads its sort and filter off its own tab, so a screen rendered
 * against a route no tab holds would have nowhere to put them. Wrapped in the
 * same `ConsoleProvider` the real shell mounts at the root, since a row's ask
 * chip now reaches `useConsole()`.
 */
function open(route: string) {
  store.openTab(route);
  return render(
    <ConsoleProvider>
      <Resources route={route} />
      <AskPeek />
    </ConsoleProvider>,
  );
}

/**
 * The detail, in whichever host is on screen — they are two screens now, not
 * one pane in two frames (R-5 is retired). `Inspector` is the only thing in
 * the app that renders `section.pane` (`Panel` renders `section.card`), and
 * `ResourceTabView` marks its own root.
 */
const peekPane = () => document.querySelector("section.pane, [data-slot='resource-tab']");

/** The heading — the subject the host CLAIMS to be showing (from its props).
 *  An `h2` in the peek, which sits inside a screen; an `h1` in the tab, which
 *  is the page. */
const paneName = () => peekPane()?.querySelector("h1, h2")?.textContent ?? null;

/** The pane's body — the subject it is ACTUALLY showing (from `getObject`). */
const paneBody = () => peekPane()?.querySelector(".pane-body")?.textContent ?? "";

/** The pane's tab strip, empty on any frame that is not the settled one. */
const paneTabs = () => screen.queryAllByRole("tab").map((tab) => tab.textContent);

/**
 * Settles on the pane's READY frame, by name.
 *
 * Neither `paneName()` nor `paneBody()` will do on its own, and that cost a
 * flake. `ResourceDetailView` renders its loading `Inspector` with `name` straight
 * from props, and that frame's own `LoadingState` reads "Loading Pod
 * default/web-1" — so the heading AND the body already say "web-1" before
 * `getObject` has answered. What does not exist until it has is the tab strip,
 * so that is the signal, and it is what the tests using this are about
 * anyway. (#331)
 */
async function paneReadyFor(name: string) {
  await waitFor(() => {
    expect(paneName()).toBe(name);
    expect(paneTabs().length).toBeGreaterThan(0);
  });
}

/**
 * A row of the LIST, by the name in its identifier cell.
 *
 * Matched the way `rowNames` reads them rather than by `getByText`: once the
 * peek is filled it holds the same name in its heading and in its Properties
 * panel, and a bare text query then finds three elements and throws.
 */
const row = (name: string) =>
  Array.from(document.querySelectorAll<HTMLTableRowElement>("tbody tr.tbl-row")).find(
    (tr) => tr.querySelector("td:not(.tbl-check)")?.textContent === name,
  )!;

/**
 * The peek's drag grip, and the width of the pane it moves.
 *
 * The grip is a direct child of the sized element by construction — that is
 * what an absolutely positioned handle needs — so reading the width off its
 * parent asks the DOM the same question the reader's eye does.
 */
const peekGrip = () => screen.getByRole("separator", { name: "Resize the resource details" });
const peekWidth = () => (peekGrip().parentElement as HTMLElement).style.width;

/** The props the most recently rendered `ResourceDetailView` was handed. */
const lastDetailProps = () => ({ ...detailProps[detailProps.length - 1] });

/**
 * Open a detail route in a tab and render the screen registered for it — the
 * way `Body` does. The tab host takes the same `{ route }` prop every screen
 * does; everything it shows is parsed back out of that string.
 *
 * Wrapped in the same `ConsoleProvider` the real shell mounts at the root, and
 * for the same reason `open` above is: the pane's footer bar reaches
 * `useConsole()` for its Ask button, in this host exactly as in the peek.
 */
function openDetailTab(route: string) {
  store.openTab(route);
  return render(
    <ConsoleProvider>
      <ResourceDetailScreen route={route} />
    </ConsoleProvider>,
  );
}

/** One row of the "About this kind" rail, by its key. */
const railRow = (rail: HTMLElement, key: string) =>
  Array.from(rail.querySelectorAll("dl.kv"))
    .find((kv) => kv.querySelector(".kv-k")?.textContent === key)
    ?.querySelector(".kv-v")?.textContent;

/** Open the column picker and hand back its panel. */
async function openColumns() {
  await userEvent.click(screen.getByRole("button", { name: /Columns/ }));
  return screen.findByRole("group");
}

describe("Resources", () => {
  it("lists a kind's rows under its own title", async () => {
    open("/k/pods");

    expect(await screen.findByRole("heading", { level: 1, name: "Pods" })).toBeTruthy();
    await waitFor(() => expect(rowNames()).toEqual(["web-1", "api-7"]));
    // The screen names no kind: the watch is opened on the route's slug.
    expect(watchResource.mock.calls[0][0]).toBe("prod-eu");
    expect(watchResource.mock.calls[0][2]).toBe("pods");
  });

  // Correction 1: the design mock titles every kind's identifier column
  // "Name" — never "Pod", "Deployment", "Secret". Classic named it by kind.
  it("titles the identifier column Name, not the kind", async () => {
    open("/k/pods");

    await waitFor(() => expect(headers()).toContain("Name"));
    expect(headers()).not.toContain("Pod");
  });

  // Correction 3: an unhealthy row gets a dot before its name, and the dot is
  // never colour alone — a reason rides beside it for anyone who cannot see
  // the colour, the same contract the cluster rail's `unavailable` follows.
  it("marks an unhealthy pod's row with a dot that also says so in words", async () => {
    watchResource.mockImplementation(
      async (_c: string, _n: string, _k: string, onRows: (rows: unknown[]) => void) => {
        onRows([
          { name: "web-1", namespace: "default", ready: "1/1", phase: "Running", restarts: 0, node: "n1", age: "2d", image: "acme/checkout-api:118a7e" },
          { name: "bad-1", namespace: "default", ready: "0/1", phase: "CrashLoopBackOff", restarts: 9, node: "n1", age: "2d", image: "acme/checkout-worker:118a7e" },
        ]);
        return { stop };
      },
    );
    open("/k/pods");
    await waitFor(() => expect(rowNames()).toHaveLength(2));

    const badRow = within(screen.getByText("bad-1").closest("tr")!);
    expect(badRow.getByText(/needs attention/i)).toBeTruthy();

    const goodRow = within(screen.getByText("web-1").closest("tr")!);
    expect(goodRow.queryByText(/needs attention/i)).toBeNull();
  });

  // Correction 3: every row gets a trailing ask chip that hands the row to
  // the console dock, naming the actual resource and its state — "Why is X
  // unhealthy?" for a bad row, a resource-use question otherwise.
  it("offers an ask chip on each row that names the resource and its state", async () => {
    watchResource.mockImplementation(
      async (_c: string, _n: string, _k: string, onRows: (rows: unknown[]) => void) => {
        onRows([
          { name: "web-1", namespace: "default", ready: "1/1", phase: "Running", restarts: 0, node: "n1", age: "2d", image: "acme/checkout-api:118a7e" },
          { name: "bad-1", namespace: "default", ready: "0/1", phase: "CrashLoopBackOff", restarts: 9, node: "n1", age: "2d", image: "acme/checkout-worker:118a7e" },
        ]);
        return { stop };
      },
    );
    open("/k/pods");
    await waitFor(() => expect(rowNames()).toHaveLength(2));

    const badRow = within(screen.getByText("bad-1").closest("tr")!);
    await userEvent.click(badRow.getByRole("button", { name: /Why is bad-1 unhealthy\?/ }));
    expect(asked).toEqual(["Why is bad-1 unhealthy?"]);

    const goodRow = within(screen.getByText("web-1").closest("tr")!);
    await userEvent.click(goodRow.getByRole("button", { name: /web-1/ }));
    expect(asked[1]).toMatch(/web-1/);
    expect(asked[1]).not.toMatch(/unhealthy/i);
  });

  it("lists a custom resource this cluster has, from its own printer columns", async () => {
    listCrds.mockResolvedValue({ crds: [WIDGETS] });
    listCustomResource.mockResolvedValue({
      items: [{ name: "left", namespace: "default", age: "1d", columns: ["Ready"] }],
    });

    open("/k/widgets.example.com");

    await waitFor(() => expect(rowNames()).toEqual(["left"]));
    expect(headers()).toContain("Phase");
    expect(listCrds).toHaveBeenCalledWith("prod-eu");
  });

  it("tells the reader what a custom kind is, in a rail beside its list", async () => {
    listCrds.mockResolvedValue({
      crds: [{ ...WIDGETS, versions: ["v1", "v1beta1"], storageVersion: "v1" }],
    });
    listCustomResource.mockResolvedValue({
      items: [{ name: "left", namespace: "default", age: "1d", columns: ["Ready"] }],
    });

    open("/k/widgets.example.com");

    await waitFor(() => expect(rowNames()).toEqual(["left"]));
    const rail = screen.getByRole("complementary", { name: "About this kind" });
    expect(railRow(rail, "Kind")).toBe("Widget");
    expect(railRow(rail, "Scope")).toBe("Namespaced");
    expect(railRow(rail, "Served versions")).toBe("v1, v1beta1");
    expect(railRow(rail, "Storage version")).toBe("v1");
    expect(railRow(rail, "Objects")).toBe("1");
    expect(within(rail).getByText(/kubectl --context prod-eu get widgets.example.com -A -o wide/)).toBeDefined();
  });

  it("counts no objects until the list has answered, rather than saying nought", async () => {
    // `Objects 0` while the rows are still in flight is not a small number,
    // it is a wrong one — and it is the number a reader glances at and
    // believes. The row waits for a count.
    listCrds.mockResolvedValue({ crds: [WIDGETS] });
    listCustomResource.mockReturnValue(new Promise(() => {}));

    open("/k/widgets.example.com");

    const rail = await screen.findByRole("complementary", { name: "About this kind" });
    expect(railRow(rail, "Kind")).toBe("Widget");
    expect(rail.textContent).not.toContain("Objects");
  });

  it("heads the custom list's own pane with the kind, not the slug", async () => {
    listCrds.mockResolvedValue({ crds: [WIDGETS] });
    listCustomResource.mockResolvedValue({
      items: [{ name: "left", namespace: "default", age: "1d", columns: ["Ready"] }],
    });

    open("/k/widgets.example.com");

    await waitFor(() => expect(rowNames()).toEqual(["left"]));
    expect(document.querySelector("[data-slot='rail-main'] .pane-head")?.textContent).toBe(
      "Widget \u00b7 custom resource",
    );
  });

  it("gives a built-in kind no rail — there is no CRD behind one", async () => {
    open("/k/pods");

    await waitFor(() => expect(rowNames()).toEqual(["web-1", "api-7"]));
    expect(screen.queryByRole("complementary", { name: "About this kind" })).toBeNull();
    expect(document.body.textContent).not.toContain("custom resource");
    // Discovery is what a rail would have to be built from, and a built-in
    // kind must never pay for it.
    expect(listCrds).not.toHaveBeenCalled();
  });

  it("narrows the list by the filter text", async () => {
    open("/k/pods");
    await waitFor(() => expect(rowNames()).toHaveLength(2));

    await userEvent.type(screen.getByRole("searchbox", { name: "Filter pods" }), "web");

    await waitFor(() => expect(rowNames()).toEqual(["web-1"]));
  });

  it("reorders by a sortable column when its header is activated", async () => {
    open("/k/pods");
    await waitFor(() => expect(rowNames()).toEqual(["web-1", "api-7"]));

    await userEvent.click(screen.getByRole("button", { name: "Sort by Restarts" }));

    await waitFor(() => expect(rowNames()).toEqual(["api-7", "web-1"]));
  });

  it("keeps a tab's sort and filter where a restart will find them", async () => {
    open("/k/pods");
    await waitFor(() => expect(rowNames()).toHaveLength(2));

    await userEvent.click(screen.getByRole("button", { name: "Sort by Restarts" }));
    await userEvent.type(screen.getByRole("searchbox", { name: "Filter pods" }), "web");

    // Component state alone would pass the two assertions above and lose both
    // values on the next launch — the tab is what gets written to disk.
    expect(tabFor("/k/pods").view).toMatchObject({
      sort: { key: "restarts", direction: "asc" },
      filter: "web",
    });
  });

  it("hides a column the picker unchecks, and remembers it for that kind", async () => {
    const view = open("/k/pods");
    await waitFor(() => expect(headers()).toContain("Restarts"));

    await openColumns();
    await userEvent.click(screen.getByRole("checkbox", { name: "Restarts" }));

    await waitFor(() => expect(headers()).not.toContain("Restarts"));
    expect([...hiddenColumns("pods")]).toEqual(["restarts"]);

    // Remembered for the kind rather than for this mounting of the screen.
    view.unmount();
    open("/k/pods");
    await waitFor(() => expect(headers()).toContain("Status"));
    expect(headers()).not.toContain("Restarts");
  });

  it("clears a filter that was on a column the user just hid", async () => {
    open("/k/pods");
    await waitFor(() => expect(headers()).toContain("Restarts"));

    // Set through the store rather than the column header's own funnel: the
    // design mock has one search box and no per-column funnels, so the
    // columns this screen hands `Table` no longer ask for one (#324) — the
    // key can still arrive here the way a restored tab would carry it in.
    act(() => store.setTabView(tabFor("/k/pods").id, { filterKey: "restarts" }));
    await waitFor(() => expect(tabFor("/k/pods").view?.filterKey).toBe("restarts"));

    await openColumns();
    await userEvent.click(screen.getByRole("checkbox", { name: "Restarts" }));

    // The classic bug: the column goes, the filter key stays, and the search
    // box quietly matches nothing for the rest of the session.
    await waitFor(() => expect(tabFor("/k/pods").view?.filterKey).toBeNull());
  });

  it("ignores a filter key naming a column another tab hid", async () => {
    open("/k/pods");
    await waitFor(() => expect(rowNames()).toHaveLength(2));
    // A key this tab has carried since a previous launch.
    act(() => store.setTabView(tabFor("/k/pods").id, { filterKey: "restarts", filter: "web" }));
    await waitFor(() => expect(rowNames()).toHaveLength(0));

    // Hidden columns belong to the kind, not to this tab: another tab — in
    // another workspace, while this screen was not mounted — can hide the
    // column this tab's filter key names, so clearing the key on the toggle
    // is not enough. Hidden here through the store, never through this
    // screen's own picker.
    act(() => toggleColumn("pods", "restarts"));

    // Pointed at a column that is not there, `filterTableData` has nothing to
    // search and quietly returns every row. Derived, the key falls away and
    // the text searches the columns that are actually on screen.
    await waitFor(() => expect(rowNames()).toEqual(["web-1"]));
  });

  it("keeps each tab's view to itself when several are mounted", async () => {
    // `Window` mounts every tab's body and only hides the inactive ones.
    listNodes.mockResolvedValue({
      nodes: [{ name: "n1", status: "Ready", roles: "worker", version: "1.30", age: "9d", taints: 0 }],
    });
    store.openTab("/k/pods");
    store.openTab("/k/nodes"); // the active one
    render(
      <ConsoleProvider>
        <Resources route="/k/pods" />
        <Resources route="/k/nodes" />
      </ConsoleProvider>,
    );
    await screen.findByRole("searchbox", { name: "Filter pods" });

    await userEvent.type(screen.getByRole("searchbox", { name: "Filter pods" }), "web");

    expect(tabFor("/k/pods").view).toMatchObject({ filter: "web" });
    // Reading the *active* tab would have written the filter here instead, and
    // re-filtered a list the user is not even looking at on every keystroke.
    expect(tabFor("/k/nodes").view).toBeUndefined();
  });

  it("shows no namespace picker for a cluster-scoped kind", async () => {
    listNodes.mockResolvedValue({ nodes: [{ name: "n1", status: "Ready", roles: "worker", version: "1.30", age: "9d", taints: 0 }] });

    open("/k/nodes");

    await waitFor(() => expect(rowNames()).toEqual(["n1"]));
    // Absent, not disabled.
    expect(screen.queryByRole("combobox", { name: "Namespaces" })).toBeNull();
    expect(headers()).not.toContain("Namespace");
  });

  it("offers the namespace picker for a namespaced kind", async () => {
    open("/k/pods");

    expect(await screen.findByRole("combobox", { name: "Namespaces" })).toBeTruthy();
  });

  // Zero options while `namespaces` is null reads as "this cluster has no
  // namespaces"; a disabled, spinning stand-in says "not yet" instead.
  it("shows the namespace picker as loading rather than empty before namespaces arrive", async () => {
    useNamespaceOptions.mockReturnValue({ namespaces: null, scope: "", error: "" });

    open("/k/pods");
    await waitFor(() => expect(rowNames()).toHaveLength(2));

    expect(screen.queryByRole("combobox", { name: "Namespaces" })).toBeNull();
    const placeholder = screen.getByRole("button", { name: "Namespaces" }) as HTMLButtonElement;
    expect(placeholder.disabled).toBe(true);
    expect(within(placeholder).getByRole("status", { name: "Loading namespaces" })).toBeTruthy();
  });

  it("warns above the table when namespace listing fails, without hiding the picker or the rows", async () => {
    useNamespaceOptions.mockReturnValue({
      namespaces: ["default", "billing"],
      scope: "",
      error: "namespaces: etcd timeout",
    });

    open("/k/pods");

    expect(await screen.findByText("Namespaces could not be listed")).toBeTruthy();
    // The sentence about the picker is this screen's and stays; what sits
    // under it is the classification, not the apiserver's own words.
    expect(screen.getByText(/didn't respond in time/)).toBeTruthy();
    const folded = document.querySelector('[data-slot="raw"]') as HTMLDetailsElement;
    expect(folded.open).toBe(false);
    expect(folded.textContent).toContain("namespaces: etcd timeout");
    // Non-fatal: the picker keeps whatever namespaces it has, and the rows load.
    expect(screen.getByRole("combobox", { name: "Namespaces" })).toBeTruthy();
    await waitFor(() => expect(rowNames()).toEqual(["web-1", "api-7"]));
  });

  it("follows the namespace a restricted credential is scoped to", async () => {
    useNamespaceOptions.mockReturnValue({ namespaces: ["team-a"], scope: "team-a", error: "" });

    open("/k/pods");

    // Written to the workspace store, so every screen on this cluster follows.
    await waitFor(() => expect(getView().namespaces.prod).toEqual(["team-a"]));
    await waitFor(() =>
      expect(watchResource.mock.calls.some((call) => call[1] === "team-a")).toBe(true),
    );
  });

  it("explains a remembered selection that no longer exists, rather than showing an empty table with no reason", async () => {
    useNamespaceOptions.mockReturnValue({ namespaces: ["default", "billing"], scope: "", error: "" });
    act(() => setNamespaces(CTX.stableId, ["deleted-ns"]));

    open("/k/pods");

    expect(await screen.findByText("Remembered namespaces are gone")).toBeTruthy();
    expect(screen.getByText("deleted-ns no longer exist on this cluster.")).toBeTruthy();

    // The alert's dismiss action is the recovery: back to "all namespaces",
    // written through the same store a manual clear would use.
    await userEvent.click(screen.getByRole("button", { name: "Show all namespaces" }));
    await waitFor(() => expect(getView().namespaces.prod).toBeUndefined());
  });

  it("does not warn about a selection that is merely empty of this kind right now", async () => {
    useNamespaceOptions.mockReturnValue({ namespaces: ["default", "billing"], scope: "", error: "" });
    act(() => setNamespaces(CTX.stableId, ["billing"])); // real namespace, just no pods in it

    open("/k/pods");
    await waitFor(() => expect(screen.getByRole("combobox", { name: "Namespaces" })).toBeTruthy());

    expect(screen.queryByText("Remembered namespaces are gone")).toBeNull();
  });

  it("says the kind has none, distinctly from the filter matching none", async () => {
    watchResource.mockImplementation(
      async (_c: string, _n: string, _k: string, onRows: (rows: unknown[]) => void) => {
        onRows([]);
        return { stop };
      },
    );
    const view = open("/k/pods");
    expect(await screen.findByText("No pods")).toBeTruthy();
    view.unmount();

    watchResource.mockImplementation(
      async (_c: string, _n: string, _k: string, onRows: (rows: unknown[]) => void) => {
        onRows(PODS);
        return { stop };
      },
    );
    open("/k/pods");
    await waitFor(() => expect(rowNames()).toHaveLength(2));
    await userEvent.type(screen.getByRole("searchbox", { name: "Filter pods" }), "zzz");

    // The second is the user's own doing, and says so.
    expect(await screen.findByText("No pods match this filter")).toBeTruthy();
    expect(screen.queryByText("No pods")).toBeNull();
  });

  it("says the cluster list could not be read rather than showing an empty table", async () => {
    watchResource.mockImplementation(
      async (
        _c: string,
        _n: string,
        _k: string,
        _onRows: unknown,
        _onStatus: unknown,
        onError: (message: string) => void,
      ) => {
        onError("pods is forbidden");
        return { stop };
      },
    );

    open("/k/pods");

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("pods is forbidden");
    expect(document.querySelector("table")).toBeNull();
  });

  it("keeps the rows on screen and calls them stale when the list stops refreshing", async () => {
    open("/k/pods");
    await waitFor(() => expect(rowNames()).toHaveLength(2));

    act(() => stream.error("connection reset"));

    // An error with rows behind it must not empty the table.
    await waitFor(() => expect(screen.getByText(/stale/i)).toBeTruthy());
    expect(rowNames()).toEqual(["web-1", "api-7"]);
  });

  // D6+D7 review: in the mock both banners sit outside the scrolling region,
  // pinned above it. A reader who scrolls the table must still see them —
  // a staleness warning that scrolls away no longer warns anyone, and
  // selection actions that scroll out of reach are worse.
  it("keeps the stale-rows alert and the bulk selection bar outside the scrolling table body", async () => {
    open("/k/pods");
    await waitFor(() => expect(rowNames()).toEqual(["web-1", "api-7"]));

    act(() => stream.error("connection reset"));
    await waitFor(() => expect(screen.getByText(/stale/i)).toBeTruthy());

    await userEvent.click(screen.getByRole("checkbox", { name: "Select default/web-1" }));
    await screen.findByText("1 selected");

    const scrollBody = document.querySelector<HTMLElement>(".scroll")!;
    expect(within(scrollBody).queryByText(/stale/i)).toBeNull();
    expect(within(scrollBody).queryByText("1 selected")).toBeNull();
    // Both still render — pinned above the scrolling body, not gone.
    expect(screen.getByText(/stale/i)).toBeTruthy();
    expect(screen.getByText("1 selected")).toBeTruthy();
  });

  // Whole-branch review (FIX 2): a namespace switch makes the old selection
  // keys meaningless (they were namespace-qualified against the namespaces
  // being left) — kept around, they'd resolve against whatever new rows
  // happen to share a key by coincidence.
  it("clears the bulk selection when the namespace filter changes", async () => {
    open("/k/pods");
    await waitFor(() => expect(rowNames()).toEqual(["web-1", "api-7"]));

    await userEvent.click(screen.getByRole("checkbox", { name: "Select default/web-1" }));
    await screen.findByText("1 selected");

    act(() => setNamespaces(CTX.stableId, ["billing"]));

    await waitFor(() => expect(screen.queryByText("1 selected")).toBeNull());

    // Not merely resolved to zero targets while `web-1` is filtered out of
    // view: the selection itself must be cleared, or switching back to a
    // namespace that still has `web-1` resurrects a checkbox the reader never
    // re-checked.
    act(() => setNamespaces(CTX.stableId, []));
    await waitFor(() => expect(rowNames()).toEqual(["web-1", "api-7"]));
    expect((screen.getByRole("checkbox", { name: "Select default/web-1" }) as HTMLInputElement).checked).toBe(false);
    expect(screen.queryByText("1 selected")).toBeNull();
  });

  // Round 4's P1, and the sixth finding of the cluster-switch class: the reset
  // above reasoned only about NAMESPACES. `KindList` is not remounted when the
  // rail switches cluster — `Resources` renders it with a new `context` prop
  // and nothing else changes — so the selected keys are only cleared if
  // something in that effect's dependencies moved.
  //
  // With ALL NAMESPACES selected, nothing did. `useNamespaces` answers an unset
  // cluster with one shared `NO_NAMESPACES` constant, so both clusters read
  // back the SAME array identity and the namespace dependency is unchanged
  // across the switch — which is why this fixture must leave both selections
  // unset. A fixture that picked a namespace per cluster would see the
  // identity change and pass against the unfixed screen, saying nothing.
  it("clears the bulk selection when the rail switches cluster, with all namespaces selected", async () => {
    setContexts([CTX, STAGE]);
    store.setState(defaultState([CTX, STAGE]));

    open("/k/pods");
    await waitFor(() => expect(rowNames()).toEqual(["web-1", "api-7"]));

    // The fixture's own premise, asserted rather than assumed: neither cluster
    // has a namespace selection, so both are on "all namespaces" and the
    // selection this screen watches cannot change identity below.
    expect(getView().namespaces).toEqual({});

    await userEvent.click(screen.getByRole("checkbox", { name: "Select default/web-1" }));
    await screen.findByText("1 selected");

    act(() => store.setActiveCluster(STAGE.stableId, STAGE.name));

    // The switch has landed and the new cluster's rows are on screen — the
    // moment the stale keys would match again, since both clusters here have a
    // `default/web-1` (which is the ordinary case: the same workloads deployed
    // to two environments).
    await waitFor(() =>
      expect(watchResource.mock.calls.some((call) => call[0] === STAGE.name)).toBe(true),
    );
    await waitFor(() => expect(rowNames()).toEqual(["web-1", "api-7"]));

    expect((screen.getByRole("checkbox", { name: "Select default/web-1" }) as HTMLInputElement).checked).toBe(false);
    // And nothing to confirm: the bar is what carries Delete, Evict and
    // Restart rollout, and it is only mounted for a selection that resolves.
    expect(screen.queryByText("1 selected")).toBeNull();
    expect(screen.queryByRole("button", { name: "Delete" })).toBeNull();
    expect(deleteResource).not.toHaveBeenCalled();
  });

  it("shows the rows and says the stream dropped when the watch is reconnecting", async () => {
    open("/k/pods");
    await waitFor(() => expect(rowNames()).toHaveLength(2));
    expect(screen.getByText("Live")).toBeTruthy();

    act(() => stream.status("reconnecting"));

    expect(await screen.findByText("Stream lost")).toBeTruthy();
    expect(rowNames()).toEqual(["web-1", "api-7"]);
  });

  it("names an unknown slug rather than rendering a blank table", async () => {
    // A route string can arrive from a session persisted against a cluster
    // that has since lost the operator.
    open("/k/nonsuch.example.com");

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("nonsuch.example.com");
    expect(document.querySelector("table")).toBeNull();
  });

  it("renders nothing but a prompt when the workspace has no active cluster", async () => {
    setContexts([]);
    store.setState(defaultState([]));

    open("/k/pods");

    expect(screen.getByText(/pick a cluster/i)).toBeTruthy();
    // Not one call into core: there is no context name to make one with.
    expect(watchResource).not.toHaveBeenCalled();
    expect(listCrds).not.toHaveBeenCalled();
    expect(useNamespaceOptions).not.toHaveBeenCalled();
  });

  // The seam between this screen and `useRowMenu` (`ResourceMenu.tsx`): the
  // hook itself is tested on its own contract in `ResourceMenu.test.tsx`,
  // but nothing there renders `Resources` — this is what proves `rowMenu`,
  // `rowMenuLabel` and the dialog are actually wired to the table this
  // screen renders, not merely both present in the file.
  it("opens a row's menu and its confirm dialog from the rendered screen", async () => {
    open("/k/pods");
    await waitFor(() => expect(rowNames()).toEqual(["web-1", "api-7"]));

    fireEvent.contextMenu(screen.getByText("web-1").closest("tr")!);
    await userEvent.click(await screen.findByText("Delete"));

    const dialog = await screen.findByRole("dialog");
    expect(dialog.textContent).toContain("web-1");
  });

  // The property that matters most for the bulk action bar: an all-namespaces
  // view can hold two same-named resources, and only the one actually checked
  // may be written to. This goes through the real rendered `Table` — its own
  // checkbox, named from its own namespace-qualified row key — rather than a
  // hand-built selection, so it proves the wiring in this screen, not just
  // `ResourceBulk`'s own contract (which `ResourceBulk.test.tsx` covers with
  // constructed keys).
  it("shows a loading state while a custom resource's CRD is being discovered", async () => {
    // Never resolves: the assertion below only holds if the screen renders
    // the loading branch itself, not just a fleeting frame before a mock
    // resolves on the next microtask.
    listCrds.mockReturnValue(new Promise(() => {}));

    open("/k/widgets.example.com");

    expect(await screen.findByText("Looking for widgets.example.com")).toBeTruthy();
    expect(document.querySelector("table")).toBeNull();
  });

  it("says the CRD lookup failed, and retries it on request", async () => {
    listCrds.mockResolvedValueOnce({ error: "customresourcedefinitions is forbidden" });

    open("/k/widgets.example.com");

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("Could not look up widgets.example.com");
    // A Forbidden that names no verb or resource falls to the generic RBAC
    // guidance rather than repeating the apiserver's phrasing at the reader.
    expect(alert.textContent).toContain("Check your RBAC roles");
    expect(alert.textContent).toContain("customresourcedefinitions is forbidden");

    listCrds.mockResolvedValueOnce({ crds: [WIDGETS] });
    listCustomResource.mockResolvedValueOnce({
      items: [{ name: "left", namespace: "default", age: "1d", columns: ["Ready"] }],
    });
    await userEvent.click(screen.getByRole("button", { name: "Retry" }));

    await waitFor(() => expect(rowNames()).toEqual(["left"]));
    expect(headers()).toContain("Phase");
  });

  // `LiveSignal` names what a *watch* is doing; a polled kind has no stream
  // to report on, so it must not appear at all — showing it would claim a
  // liveness this list does not have.
  it("shows no LiveSignal for a polled kind", async () => {
    listCrds.mockResolvedValue({ crds: [WIDGETS] });
    listCustomResource.mockResolvedValue({
      items: [{ name: "left", namespace: "default", age: "1d", columns: ["Ready"] }],
    });

    open("/k/widgets.example.com");

    await waitFor(() => expect(rowNames()).toEqual(["left"]));
    expect(screen.queryByText("Live")).toBeNull();
    expect(screen.queryByText("Stream lost")).toBeNull();
  });

  it("opens the resource's /k/<kind>/<namespace>/<name> route when a row is activated", async () => {
    open("/k/pods");
    await waitFor(() => expect(rowNames()).toEqual(["web-1", "api-7"]));

    fireEvent.doubleClick(screen.getByText("web-1").closest("tr")!);

    // web-1's own namespace is "default" (PODS above) — the descriptor's
    // k8sKind ("Pod"), not the /k/pods slug, is what mints the route.
    await waitFor(() => expect(tabFor("/k/Pod/default/web-1")).toBeTruthy());
    expect(store.currentWorkspace().activeId).toBe(tabFor("/k/Pod/default/web-1").id);
  });

  it("gives two same-named resources of different kinds their own tabs", async () => {
    // The bug this route model exists to fix: openTab dedupes by route, so
    // /resources/web opened from a Pod row and from a ConfigMap row was one
    // tab. The two kinds now mint different routes for the same name.
    watchResource.mockImplementation(
      async (_c: string, _n: string, _k: string, onRows: (rows: unknown[]) => void) => {
        onRows([{ name: "web", namespace: "default", ready: "1/1", phase: "Running", restarts: 0, node: "n1", age: "1d", image: "acme/checkout-api:118a7e" }]);
        return { stop };
      },
    );
    const podView = open("/k/pods");
    await waitFor(() => expect(rowNames()).toEqual(["web"]));
    fireEvent.doubleClick(screen.getByText("web").closest("tr")!);
    await waitFor(() => expect(tabFor("/k/Pod/default/web")).toBeTruthy());
    podView.unmount();

    watchResource.mockImplementation(
      async (_c: string, _n: string, _k: string, onRows: (rows: unknown[]) => void) => {
        onRows([{ name: "web", namespace: "default", keys: 2, age: "1d" }]);
        return { stop };
      },
    );
    open("/k/configmaps");
    await waitFor(() => expect(rowNames()).toEqual(["web"]));
    fireEvent.doubleClick(screen.getByText("web").closest("tr")!);
    await waitFor(() => expect(tabFor("/k/ConfigMap/default/web")).toBeTruthy());

    const tabs = store.currentWorkspace().tabs;
    expect(tabs.some((t) => t.route === "/k/Pod/default/web")).toBe(true);
    expect(tabs.some((t) => t.route === "/k/ConfigMap/default/web")).toBe(true);
    // Two distinct tabs, not one that the second activation merely reused.
    expect(tabFor("/k/Pod/default/web").id).not.toBe(tabFor("/k/ConfigMap/default/web").id);
  });

  it("deletes only the checked row when two selected candidates share a name across namespaces", async () => {
    watchResource.mockImplementation(
      async (_c: string, _n: string, _k: string, onRows: (rows: unknown[]) => void) => {
        onRows([
          { name: "web-0", namespace: "default", ready: "1/1", phase: "Running", restarts: 0, node: "n1", age: "1d", image: "acme/checkout-api:118a7e" },
          { name: "web-0", namespace: "billing", ready: "1/1", phase: "Running", restarts: 0, node: "n2", age: "1d", image: "acme/checkout-api:118a7e" },
        ]);
        return { stop };
      },
    );

    open("/k/pods");
    await waitFor(() => expect(rowNames()).toEqual(["web-0", "web-0"]));

    // Check only the billing one. The two rows render identical name text —
    // the checkbox is the only thing on screen that disambiguates them.
    await userEvent.click(screen.getByRole("checkbox", { name: "Select billing/web-0" }));

    await userEvent.click(await screen.findByRole("button", { name: "Delete" }));
    const dialog = within(await screen.findByRole("dialog"));
    await userEvent.click(dialog.getByRole("button", { name: "Delete" }));

    await waitFor(() => expect(deleteResource).toHaveBeenCalledTimes(1));
    expect(deleteResource).toHaveBeenCalledWith("prod-eu", "Pod", "billing", "web-0");
  });
});

/**
 * The two screens a resource detail is drawn as: the peek beside the list
 * (`ResourceDetailView`) and the page a detail route fills a tab with
 * (`ResourceTabView`). Two designs, deliberately — what is asserted here is
 * that the list points them at one subject and gives each only the controls
 * that are its own.
 */
describe("the detail pane's two hosts", () => {
  it("fills the peek from a row click, and opens no tab doing it", async () => {
    open("/k/pods");
    await waitFor(() => expect(rowNames()).toEqual(["web-1", "api-7"]));
    expect(peekPane()).toBeNull();
    const before = store.currentWorkspace().tabs.map((t) => t.route);

    fireEvent.click(row("web-1"));

    await waitFor(() => expect(paneName()).toBe("web-1"));
    // A single click peeks; it never navigates. Opening a tab from it would
    // make scanning a list a pile of tabs the reader never asked for.
    expect(store.currentWorkspace().tabs.map((t) => t.route)).toEqual(before);
  });

  it("dismisses the peek on request", async () => {
    open("/k/pods");
    await waitFor(() => expect(rowNames()).toEqual(["web-1", "api-7"]));
    fireEvent.click(row("web-1"));
    await waitFor(() => expect(paneName()).toBe("web-1"));

    await userEvent.click(screen.getByRole("button", { name: "Close inspector" }));

    await waitFor(() => expect(peekPane()).toBeNull());
    // The table is still there — dismissing the peek is not leaving the list.
    expect(rowNames()).toEqual(["web-1", "api-7"]);
  });

  it("does not refetch when the peek is already showing that row", async () => {
    open("/k/pods");
    await waitFor(() => expect(rowNames()).toEqual(["web-1", "api-7"]));

    fireEvent.click(row("web-1"));
    await waitFor(() => expect(paneBody()).toContain("web-1"));
    expect(getObject).toHaveBeenCalledTimes(1);
    const rendersAfterFirstClick = detailProps.length;

    fireEvent.click(row("web-1"));
    // A second read would be issued from an effect, so let every pending one
    // run before counting: this must fail on a refetch, not race past it.
    await act(async () => {
      await Promise.resolve();
    });
    expect(paneName()).toBe("web-1");
    expect(getObject).toHaveBeenCalledTimes(1);
    // And not so much as a re-render: `peekAt` hands back the previous state
    // object when the row clicked is the one already on show, so React bails
    // out before the pane is asked for anything. Counted through the mock
    // wrapper, which records one entry per render of the pane.
    expect(detailProps.length).toBe(rendersAfterFirstClick);

    // The counter above is live, not merely stuck: a click on a DIFFERENT row
    // does move it. Without this the assertion would still pass if the pane
    // had stopped rendering altogether.
    fireEvent.click(row("api-7"));
    await waitFor(() => expect(paneBody()).toContain("api-7"));
    expect(detailProps.length).toBeGreaterThan(rendersAfterFirstClick);
  });

  it("never pairs one row's heading with another row's body, and keeps one pane across the switch", async () => {
    // Settled assertions cannot see this: RTL flushes effects synchronously,
    // so a bad frame is overwritten before `waitFor` resolves. A real browser
    // paints whatever was committed. `detailFrames` (recorded by the mock
    // wrapper around the pane — see its comment for why it has to live there)
    // holds every frame the switch below committed.
    open("/k/pods");
    await waitFor(() => expect(rowNames()).toEqual(["web-1", "api-7"]));

    fireEvent.click(row("web-1"));
    await waitFor(() => expect(paneBody()).toContain("web-1"));
    const paneNode = peekPane();
    detailFrames.length = 0;

    fireEvent.click(row("api-7"));
    await waitFor(() => expect(paneBody()).toContain("api-7"));
    expect(paneName()).toBe("api-7");

    // The same pane instance, not a fresh one: remounting per row would throw
    // away the reader's selected tab on every click, and would paper over
    // `ResourceDetailView`'s own target gate rather than honour it.
    expect(peekPane()).toBe(paneNode);

    // The probe has to have seen something, or the assertion below is over an
    // empty list and says nothing at all. That is not a hypothetical: the
    // first cut of this test recorded nothing and passed anyway.
    expect(detailFrames.length).toBeGreaterThan(0);

    const mismatched = detailFrames.filter((frame) => {
      if (!frame.heading) return false;
      const other = frame.heading === "web-1" ? "api-7" : "web-1";
      return frame.body.includes(other) && !frame.body.includes(frame.heading);
    });
    expect(mismatched).toEqual([]);
  });

  it("shows the error, not a blank pane, for a row whose resource has gone", async () => {
    getObject.mockResolvedValue({ error: 'pods "web-1" not found' });
    open("/k/pods");
    await waitFor(() => expect(rowNames()).toEqual(["web-1", "api-7"]));

    fireEvent.click(row("web-1"));

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("Could not load Pod default/web-1");
    expect(alert.textContent).toContain("not found");
  });

  it("keeps a click on a row's own controls out of the peek", async () => {
    // Both live inside the `<tr>` the peek gesture is bound to. Checking a
    // row is not looking at it, and asking about a row is not looking at it
    // either — either one filling the peek is a bubbling bug.
    open("/k/pods");
    await waitFor(() => expect(rowNames()).toEqual(["web-1", "api-7"]));

    await userEvent.click(screen.getByRole("checkbox", { name: "Select default/web-1" }));
    expect(peekPane()).toBeNull();

    await userEvent.click(
      within(row("web-1")).getByRole("button", { name: /What is web-1 using right now\?/ }),
    );
    expect(peekPane()).toBeNull();

    // The row menu is a right-click, which produces no click at all — pinned
    // so a future trigger button inside the row cannot quietly start peeking.
    fireEvent.contextMenu(row("web-1"));
    // By role: the checkbox above put a Delete on the bulk bar too, and the
    // menu's own item is the one this is about.
    expect(await screen.findByRole("menuitem", { name: "Delete" })).toBeTruthy();
    expect(peekPane()).toBeNull();
  });

  it("opens the detail route's own screen for a full tab, reading the subject off the route", async () => {
    openDetailTab("/k/Pod/default/web-1");

    await waitFor(() => expect(paneName()).toBe("web-1"));
    expect(getObject).toHaveBeenCalledWith("prod-eu", "Pod", "default", "web-1");
    // The tab host is the whole tab — it offers no close of its own, because
    // closing a tab is the strip's job.
    expect(screen.queryByRole("button", { name: "Close inspector" })).toBeNull();
  });

  it("reads a cluster-scoped detail route's null namespace off the sentinel", async () => {
    openDetailTab("/k/Node/-/worker-1");

    await waitFor(() => expect(paneName()).toBe("worker-1"));
    expect(getObject).toHaveBeenCalledWith("prod-eu", "Node", null, "worker-1");
  });

  it("prompts for a cluster rather than reading anything when the workspace has none", async () => {
    setContexts([]);
    store.setState(defaultState([]));

    openDetailTab("/k/Pod/default/web-1");

    expect(screen.getByText(/pick a cluster/i)).toBeTruthy();
    expect(getObject).not.toHaveBeenCalled();
  });

  /**
   * R-5 IS RETIRED, and this is what replaced the test that pinned it.
   *
   * The old assertion captured both hosts' props, deleted exactly one key and
   * compared the rest to a four-key literal — a sound way to say "one pane,
   * two hosts", and a statement that is no longer true: the user's full-tab
   * mock makes the tab its own screen. Deleting the test would have left the
   * property that DID survive unwatched, so it says that instead: the two
   * hosts are pointed at the very same subject, by the very same four facts,
   * and only the peek's own two controls are the peek's.
   */
  it("points both hosts at the same subject, and gives only the peek its own controls", async () => {
    const list = open("/k/pods");
    await waitFor(() => expect(rowNames()).toEqual(["web-1", "api-7"]));
    fireEvent.click(row("web-1"));
    await waitFor(() => expect(paneName()).toBe("web-1"));
    const fromPeek = lastDetailProps();
    list.unmount();

    detailProps.length = 0;
    detailFrames.length = 0;
    tabProps.length = 0;
    openDetailTab("/k/Pod/default/web-1");
    await waitFor(() => expect(paneName()).toBe("web-1"));
    const fromTab = { ...tabProps[tabProps.length - 1] };

    // Both of the peek host's controls ride in one object, so a host cannot
    // hand over one without the other — dismiss, and promote to a tab.
    const peekControls = fromPeek.peek as { onClose?: unknown; onOpenTab?: unknown };
    expect(typeof peekControls.onClose).toBe("function");
    expect(typeof peekControls.onOpenTab).toBe("function");
    delete fromPeek.peek;

    // The subject itself: the same four facts, value by value, whichever
    // screen is drawing it. A tab that resolved its own context, or read the
    // kind off the route differently from the list, is exactly how one
    // resource becomes two.
    const subject = { context: "prod-eu", kind: "Pod", namespace: "default", name: "web-1" };
    expect(fromPeek).toEqual(subject);
    expect(fromTab).toEqual(subject);
  });

  it("promotes the peeked row to its own tab, at the very route a double click opens", async () => {
    open("/k/pods");
    await waitFor(() => expect(rowNames()).toEqual(["web-1", "api-7"]));
    fireEvent.click(row("web-1"));
    await waitFor(() => expect(paneName()).toBe("web-1"));

    await userEvent.click(screen.getByRole("button", { name: "Open tab" }));

    await waitFor(() => expect(tabFor("/k/Pod/default/web-1")).toBeTruthy());
    expect(store.currentWorkspace().activeId).toBe(tabFor("/k/Pod/default/web-1").id);
    const routes = store.currentWorkspace().tabs.map((t) => t.route);

    // The same route the row's own activate gesture mints. `openTab` dedupes
    // by route, so two spellings of one resource would quietly become two
    // tabs — which is the bug the route model was built to stop.
    fireEvent.doubleClick(row("web-1"));
    expect(store.currentWorkspace().tabs.map((t) => t.route)).toEqual(routes);
  });

  // Both settle through `paneReadyFor`, never on the subject's name. The
  // loading frame carries that name in its heading AND in its own
  // "Loading Pod default/web-1" label, so a `waitFor` on either returned
  // before `getObject` answered — which let the panes test race the fetch
  // (one failure in twelve runs) and made the Open-tab test vacuous, since
  // the loading `Inspector` renders `actions` too and a full-tab host has
  // none in either frame.
  it("offers no Open tab in the full-tab host, which is already the tab", async () => {
    openDetailTab("/k/Pod/default/web-1");

    await paneReadyFor("web-1");
    expect(screen.queryByRole("button", { name: "Open tab" })).toBeNull();
  });

  it("offers the same data panes in both hosts, under the names each screen gives them", async () => {
    // Not the same STRIP any more: the design's full tab heads its first pane
    // Overview and folds the containers table into it, where the peek calls it
    // Details and gives Containers a tab of its own. What must still hold is
    // that neither host offers a pane the other cannot reach — the panes are
    // the kind's, and only their labels and their grouping are the screen's.
    const list = open("/k/pods");
    await waitFor(() => expect(rowNames()).toEqual(["web-1", "api-7"]));
    fireEvent.click(row("web-1"));
    await paneReadyFor("web-1");
    const peekTabs = paneTabs();
    expect(peekTabs).toContain("Details");
    expect(peekTabs).toContain("YAML");
    expect(peekTabs).toContain("Events");
    list.unmount();

    openDetailTab("/k/Pod/default/web-1");
    await paneReadyFor("web-1");

    const tabTabs = paneTabs();
    expect(tabTabs).toContain("Overview");
    expect(tabTabs).not.toContain("Details");
    // Every pane the peek offers beyond its first is offered here too, by the
    // same name.
    for (const label of peekTabs.filter((t) => t !== "Details" && t !== "Containers")) {
      expect(tabTabs).toContain(label);
    }
  });
});

/**
 * The peek's width. The user's report was that the pane "is not draggable to
 * increase width", so the grip is the feature; the rest is making sure it
 * cannot be dragged somewhere useless, and that dragging it does not throw
 * the pane away and refetch the resource forty times on the way across.
 */
describe("the peek's width", () => {
  async function peekAtWeb1() {
    open("/k/pods");
    await waitFor(() => expect(rowNames()).toEqual(["web-1", "api-7"]));
    fireEvent.click(row("web-1"));
    await waitFor(() => expect(paneBody()).toContain("web-1"));
  }

  it("gives the peek a named grip carrying its width between its bounds", async () => {
    await peekAtWeb1();
    expect(peekGrip().getAttribute("aria-orientation")).toBe("vertical");
    expect(peekGrip().getAttribute("aria-valuenow")).toBe(String(DEFAULT_PEEK_WIDTH));
    expect(peekGrip().getAttribute("aria-valuemin")).toBe(String(MIN_PEEK_WIDTH));
    // jsdom's window is 1024 wide, which leaves the peek its whole ceiling.
    expect(peekGrip().getAttribute("aria-valuemax")).toBe(String(MAX_PEEK_WIDTH));
    expect(peekWidth()).toBe(`${DEFAULT_PEEK_WIDTH}px`);
  });

  it("widens as the pointer goes left and narrows as it goes right", async () => {
    await peekAtWeb1();
    fireEvent.mouseDown(peekGrip(), { clientX: 800 });
    // The pane is docked on the right: its left edge moving left is the pane
    // getting wider. This is the whole of the user's report.
    fireEvent.mouseMove(window, { clientX: 700 });
    expect(peekWidth()).toBe(`${DEFAULT_PEEK_WIDTH + 100}px`);
    fireEvent.mouseMove(window, { clientX: 860 });
    expect(peekWidth()).toBe(`${DEFAULT_PEEK_WIDTH - 60}px`);
    fireEvent.mouseUp(window);
  });

  it("takes the arrow keys, with ArrowLeft the wider one", async () => {
    await peekAtWeb1();
    peekGrip().focus();
    await userEvent.keyboard("{ArrowLeft}");
    expect(peekWidth()).toBe(`${DEFAULT_PEEK_WIDTH + 16}px`);
    await userEvent.keyboard("{ArrowRight}{ArrowRight}");
    expect(peekWidth()).toBe(`${DEFAULT_PEEK_WIDTH - 16}px`);
  });

  it("goes no wider than the window leaves room for", async () => {
    await peekAtWeb1();
    fireEvent.mouseDown(peekGrip(), { clientX: 800 });
    fireEvent.mouseMove(window, { clientX: -4000 });
    fireEvent.mouseUp(window);
    expect(peekWidth()).toBe(`${MAX_PEEK_WIDTH}px`);
    // And no narrower than its own content.
    fireEvent.mouseDown(peekGrip(), { clientX: 800 });
    fireEvent.mouseMove(window, { clientX: 4000 });
    fireEvent.mouseUp(window);
    expect(peekWidth()).toBe(`${MIN_PEEK_WIDTH}px`);
  });

  it("opens at the width the reader left it at last time", async () => {
    localStorage.setItem(PEEK_WIDTH_KEY, JSON.stringify(420));
    loadPeekWidth();
    await peekAtWeb1();
    expect(peekWidth()).toBe("420px");
  });

  it("remembers a drag, and only once it settles", async () => {
    await peekAtWeb1();
    fireEvent.mouseDown(peekGrip(), { clientX: 800 });
    fireEvent.mouseMove(window, { clientX: 780 });
    fireEvent.mouseMove(window, { clientX: 760 });
    // A write per pixel of the drag is what the grip's two callbacks avoid.
    expect(localStorage.getItem(PEEK_WIDTH_KEY)).toBeNull();
    fireEvent.mouseUp(window);
    expect(localStorage.getItem(PEEK_WIDTH_KEY)).toBe(String(DEFAULT_PEEK_WIDTH + 40));
  });

  it("keeps the one pane across a resize, and reads the resource once", async () => {
    await peekAtWeb1();
    expect(getObject).toHaveBeenCalledTimes(1);
    const paneNode = peekPane();

    fireEvent.mouseDown(peekGrip(), { clientX: 800 });
    for (let x = 790; x >= 700; x -= 10) fireEvent.mouseMove(window, { clientX: x });
    fireEvent.mouseUp(window);
    await act(async () => {
      await Promise.resolve();
    });

    expect(peekWidth()).toBe(`${DEFAULT_PEEK_WIDTH + 100}px`);
    // A width change that remounted the pane would refetch the resource on
    // every one of those ten frames, and throw away the reader's selected tab
    // while it was at it.
    expect(peekPane()).toBe(paneNode);
    expect(getObject).toHaveBeenCalledTimes(1);
  });
});

/**
 * The room the peek leaves the list.
 *
 * The list does not own the window. The cluster rail (~46px) and the
 * navigation `Sidebar` (238px, and up to 420 once the reader drags it) sit
 * outside this screen entirely, so a ceiling computed from `window.innerWidth`
 * hands the peek space that was never the list's to give. What the list and
 * the peek actually share is the flex row they are siblings in, and the only
 * way to say how wide that is under jsdom — which does no layout at all — is
 * to tell the observer watching it.
 */
describe("the room the peek leaves the list", () => {
  /** A 1280px window, less the 46px rail and a sidebar dragged to its widest. */
  const SHARED = 1280 - 46 - 420;

  type Watch = { target: Element; cb: ResizeObserverCallback };
  let watches: Watch[];
  let original: typeof globalThis.ResizeObserver;

  beforeEach(() => {
    watches = [];
    original = globalThis.ResizeObserver;
    globalThis.ResizeObserver = class Recording implements ResizeObserver {
      constructor(private readonly cb: ResizeObserverCallback) {}
      observe(target: Element) {
        watches.push({ target, cb: this.cb });
      }
      unobserve() {}
      disconnect() {}
    };
  });

  afterEach(() => {
    globalThis.ResizeObserver = original;
  });

  /** The box the list and the peek are siblings in — what the ceiling is measured against. */
  const listRow = () => peekGrip().parentElement!.parentElement as HTMLElement;

  /**
   * Tell everything watching the ROW that the row is this wide.
   *
   * Filtered by target, and asserted to have found something, rather than
   * fired at every callback the stub recorded: observing the body, or an
   * ancestor, or nothing at all is a measurement of the wrong box, and no
   * assertion downstream could tell the difference — a number arriving proves
   * only that a number arrived.
   */
  function measured(width: number) {
    const row = listRow();
    const watching = watches.filter((watch) => watch.target === row);
    expect(watching.length, "nothing is observing the row the list and the peek share").toBeGreaterThan(0);
    act(() => {
      for (const { target, cb } of watching) {
        cb([{ target, contentRect: { width } }] as unknown as ResizeObserverEntry[], {} as ResizeObserver);
      }
    });
  }

  async function peekAtWeb1() {
    open("/k/pods");
    await waitFor(() => expect(rowNames()).toEqual(["web-1", "api-7"]));
    fireEvent.click(row("web-1"));
    await waitFor(() => expect(paneBody()).toContain("web-1"));
  }

  /** Drag the grip as far left as it will go — the widest the peek can get. */
  function dragAsWideAsItGoes() {
    fireEvent.mouseDown(peekGrip(), { clientX: 800 });
    fireEvent.mouseMove(window, { clientX: -8000 });
    fireEvent.mouseUp(window);
  }

  it("keeps the list its floor however wide the peek is dragged", async () => {
    await peekAtWeb1();
    measured(SHARED);

    expect(peekGrip().getAttribute("aria-valuemax")).toBe(String(SHARED - MIN_LIST_WIDTH));
    dragAsWideAsItGoes();
    // The property the floor constant was always for, stated as the reader
    // would see it: whatever is left of the row is still a usable table.
    expect(SHARED - parseInt(peekWidth(), 10)).toBeGreaterThanOrEqual(MIN_LIST_WIDTH);
  });

  it("measures its row even when the descriptor arrives late", async () => {
    // A custom resource's descriptor waits on CRD discovery, so this screen's
    // first render is a loading state with no row in it at all —
    // `descriptorFor` is synchronous, CRD discovery is not. An effect
    // keyed on a ref OBJECT never re-runs when the row finally mounts, since
    // a ref's identity never changes, and the ceiling then falls back to the
    // absolute maximum for every CRD list there is.
    listCrds.mockResolvedValue({ crds: [WIDGETS] });
    listCustomResource.mockResolvedValue({
      items: [{ name: "left", namespace: "default", age: "1d", columns: ["Ready"] }],
    });

    open("/k/widgets.example.com");
    await waitFor(() => expect(rowNames()).toEqual(["left"]));
    fireEvent.click(row("left"));
    await waitFor(() => expect(paneName()).toBe("left"));

    measured(SHARED);
    expect(peekGrip().getAttribute("aria-valuemax")).toBe(String(SHARED - MIN_LIST_WIDTH));
  });

  it("still stops at its own ceiling when the row is generous", async () => {
    await peekAtWeb1();
    measured(4000);

    expect(peekGrip().getAttribute("aria-valuemax")).toBe(String(MAX_PEEK_WIDTH));
    dragAsWideAsItGoes();
    expect(peekWidth()).toBe(`${MAX_PEEK_WIDTH}px`);
  });

  it("stays legible in a row too narrow for both, and lets the table scroll instead", async () => {
    await peekAtWeb1();
    measured(MIN_LIST_WIDTH + 100);

    // Below this the pane cannot show what it holds, so the minimum wins over
    // the floor and the list scrolls inside itself — which is what `min-w-0`
    // on its column is there for.
    expect(peekGrip().getAttribute("aria-valuemax")).toBe(String(MIN_PEEK_WIDTH));
    dragAsWideAsItGoes();
    expect(peekWidth()).toBe(`${MIN_PEEK_WIDTH}px`);
  });
});
