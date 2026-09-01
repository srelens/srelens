import { useEffect } from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

// The two things this screen reaches outside itself for: the stream its rows
// arrive on, and the namespace list behind the picker. `watchResource` is held
// open rather than resolved once — half of what this screen does is react to a
// stream that keeps arriving, and a mock that answers only the first call can
// say nothing about a dropped connection or a second snapshot.
const { watchResource, useNamespaceOptions } = vi.hoisted(() => ({
  watchResource: vi.fn(),
  useNamespaceOptions: vi.fn(),
}));

vi.mock("@srelens/core", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@srelens/core")>()),
  watchResource: (...a: unknown[]) => watchResource(...a),
}));

vi.mock("@srelens/core/react", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@srelens/core/react")>()),
  useNamespaceOptions: (...a: unknown[]) => useNamespaceOptions(...a),
}));

// jsdom has neither, and the namespace picker is a Radix popover. Inert stubs:
// jsdom does no layout, so there is never a resize to report.
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

import type { ClusterContext } from "@srelens/core";
import { Events } from "./Events";
import { ConsoleProvider, useConsole } from "../console";
import { resetContexts, setContexts, setKubeconfigFiles } from "../lib/clusters";
import { loadColumnPrefs, toggleColumn } from "../lib/columnPrefs";
import { resetListCache } from "../lib/resourceList";
import { defaultState } from "../lib/tabs";
import * as store from "../lib/tabsStore";
import { resetView, setNamespaces } from "../lib/workspace";

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
 * Four events, chosen so every distinction the screen makes is visible in the
 * data: two types, three namespaces (one of them empty — a Node's event is
 * cluster-scoped and belongs to none), two kinds of involved object, and a
 * repeat count that is not 1.
 *
 * `name` is the event's KEY, which the backend still prefixes with the
 * namespace so one value is unique across the cluster; `namespace` is its own
 * field beside it. Nothing here reads the key's shape — that is
 * `eventNamespace`'s business and nobody else's.
 */
const EVENTS = [
  {
    name: "billing/api-7.17a",
    namespace: "billing",
    type: "Warning",
    reason: "BackOff",
    object: "Pod/api-7",
    message: "Back-off restarting failed container api",
    count: 37,
    age: "12s",
  },
  {
    name: "shop/web-1.17b",
    namespace: "shop",
    type: "Normal",
    reason: "Scheduled",
    object: "Pod/web-1",
    message: "Successfully assigned shop/web-1 to node-3",
    count: 1,
    age: "5m",
  },
  {
    name: "shop/checkout.17c",
    namespace: "shop",
    type: "Warning",
    reason: "Unhealthy",
    object: "Deployment/checkout-api",
    message: "Readiness probe failed: HTTP 503",
    count: 9,
    age: "41s",
  },
  {
    name: "node-3.17d",
    namespace: "",
    type: "Normal",
    reason: "NodeReady",
    object: "Node/node-3",
    message: "Node node-3 status is now: NodeReady",
    count: 1,
    age: "2d",
  },
];

/** The live watch: what the screen was handed, so a test can push into it. */
let stream: {
  rows: (rows: unknown[]) => void;
  status: (status: "live" | "reconnecting") => void;
  error: (message: string) => void;
};
/** What the watch delivers on subscribe — `null` to leave it silent, which is
 *  the only way to see the screen's loading state. */
let initialRows: unknown[] | null;

beforeEach(() => {
  vi.clearAllMocks();
  asked = [];
  initialRows = EVENTS;
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
      if (initialRows) onRows(initialRows);
      return { stop: vi.fn() };
    },
  );
  useNamespaceOptions.mockReturnValue({ namespaces: ["billing", "shop"], scope: "", error: "" });

  resetContexts();
  setContexts([CTX]);
  setKubeconfigFiles(["/home/u/.kube/config"]);
  store.setState(defaultState([CTX]));
  resetView();
  resetListCache();
  // Module state that outlives a test; localStorage is cleared by the shared
  // setup, so re-reading it is a reset.
  loadColumnPrefs();
});

/** Every question the console was handed, in the order it was asked. */
let asked: string[];

/** Stands in for the dock: registers as the console's listener the way
 *  `ConsoleDock` does, and records what arrives instead of rendering it. */
function AskPeek() {
  const { registerSubmit } = useConsole();
  useEffect(() => registerSubmit((question) => asked.push(question)), [registerSubmit]);
  return null;
}

/**
 * Open the route in a tab, then render its screen — the way `Window` does it.
 * The screen reads its sort and filter off its own tab, so a screen rendered
 * against a route no tab holds would have nowhere to put them.
 */
function open(route = "/events") {
  store.openTab(route);
  return render(
    <ConsoleProvider>
      <Events route={route} />
      <AskPeek />
    </ConsoleProvider>,
  );
}

/** Every header, in the order the table draws them. Sortable or not: `Table`
 *  wraps every header in the same button, disabling the ones that do not sort. */
const headers = () =>
  // `:not(.th-caret)` because the sort caret is a second span inside the very
  // same button — without it every sortable column reports an extra "".
  Array.from(document.querySelectorAll("thead th .th-sort > span:not(.th-caret)")).map(
    (el) => el.textContent,
  );

/** Every rendered row, as the text of its cells. */
const cells = () =>
  Array.from(document.querySelectorAll("tbody tr.tbl-row")).map((tr) =>
    Array.from(tr.querySelectorAll("td")).map((td) => td.textContent ?? ""),
  );

/** The Reason column of every rendered row — the shortest way to say WHICH
 *  events are on screen. */
const reasons = () => cells().map((row) => row[1]);

/** The row whose Reason cell reads `reason`. */
const row = (reason: string) =>
  Array.from(document.querySelectorAll<HTMLTableRowElement>("tbody tr.tbl-row")).find(
    (tr) => tr.querySelectorAll("td")[1]?.textContent === reason,
  )!;

/** Every open tab's route, in strip order. */
const routes = () => store.currentWorkspace().tabs.map((t) => t.route);

// By role, not by label: `FilterBar` names the search LANDMARK and the field
// inside it with the same string, and a label query finds both.
const search = () => screen.getByRole("searchbox", { name: "Filter events" });

/** The by-reason rail's rows, as the reason and the count each one shows. */
const railRows = () =>
  Array.from(
    screen.getByRole("complementary", { name: "By reason" }).querySelectorAll("button"),
  ).map(
    (el) =>
      [
        el.querySelector(".status")?.textContent ?? "",
        el.querySelector(".path")?.textContent ?? "",
      ] as const,
  );

const eyebrowText = () =>
  Array.from(document.querySelectorAll("[data-slot='screen-actions'] .eyebrow")).map(
    (el) => el.textContent,
  );

describe("Events", () => {
  it("applies regex mode to the same event fields as plain filtering", async () => {
    open();
    await waitFor(() => expect(reasons()).toHaveLength(4));

    await userEvent.type(search(), "^(BackOff|Unhealthy)$");
    await waitFor(() => expect(reasons()).toEqual([]));
    await userEvent.click(screen.getByRole("button", { name: "Use regular expression" }));
    await waitFor(() => expect(reasons()).toEqual(["BackOff", "Unhealthy"]));
  });

  it("renders the design's eight columns, in its order", async () => {
    open();
    await waitFor(() => expect(cells().length).toBe(4));
    // The eighth is the hover ask, which the design leaves unnamed.
    expect(headers()).toEqual(["Type", "Reason", "Object", "Namespace", "Message", "Count", "Age", ""]);
  });

  it("fills each cell from its own field, not from a neighbouring one", async () => {
    open();
    await waitFor(() => expect(cells().length).toBe(4));
    // The object is `<kind lowercased>/<name>`; the namespace is the event's
    // own, read through the one helper that answers for it.
    expect(cells()[0].slice(0, 7)).toEqual([
      "Warning",
      "BackOff",
      "pod/api-7",
      "billing",
      "Back-off restarting failed container api",
      "37",
      "12s",
    ]);
    // A cluster-scoped event has no namespace — a real answer, not a blank.
    expect(cells()[3].slice(0, 4)).toEqual(["Normal", "NodeReady", "node/node-3", "—"]);
  });

  it("counts the filtered set in its eyebrow, not the loaded set", async () => {
    const user = userEvent.setup();
    open();
    await waitFor(() => expect(cells().length).toBe(4));
    expect(eyebrowText()).toContain("4 events · 2 warnings");

    await user.type(search(), "probe");
    await waitFor(() => expect(reasons()).toEqual(["Unhealthy"]));
    // One of each, and it says so. `<n> events · <m> warnings` is placeholder
    // notation in a design document, not copy — and this is not a hypothetical
    // corner: the demo cluster splits 1 Warning / 63 Normal, so clicking
    // `Warnings` (the first thing anyone does here) lands on exactly this.
    expect(eyebrowText()).toContain("1 event · 1 warning");
  });

  it("segments by type, in the design's order, and back again", async () => {
    const user = userEvent.setup();
    open();
    await waitFor(() => expect(cells().length).toBe(4));

    const segments = screen.getByRole("tablist", { name: "Type" });
    expect(within(segments).getAllByRole("tab").map((t) => t.textContent)).toEqual([
      "All",
      "Warnings",
      "Normal",
    ]);

    await user.click(within(segments).getByRole("tab", { name: "Warnings" }));
    await waitFor(() => expect(reasons()).toEqual(["BackOff", "Unhealthy"]));
    expect(eyebrowText()).toContain("2 events · 2 warnings");

    await user.click(within(segments).getByRole("tab", { name: "Normal" }));
    await waitFor(() => expect(reasons()).toEqual(["Scheduled", "NodeReady"]));
    expect(eyebrowText()).toContain("2 events · 0 warnings");

    await user.click(within(segments).getByRole("tab", { name: "All" }));
    await waitFor(() => expect(reasons().length).toBe(4));
  });

  it("searches the fields its placeholder promises, and no others", async () => {
    const user = userEvent.setup();
    open();
    await waitFor(() => expect(cells().length).toBe(4));
    expect((search() as HTMLInputElement).placeholder).toBe("Filter by reason, message or object");

    // The object, case-insensitively.
    await user.type(search(), "checkout-api");
    await waitFor(() => expect(reasons()).toEqual(["Unhealthy"]));

    await user.clear(search());
    // A namespace is not one of the three, and the picker beside the field is
    // what narrows by namespace.
    await user.type(search(), "billing");
    await waitFor(() => expect(reasons()).toEqual([]));
    expect(screen.getByText("No events match this filter")).toBeTruthy();
  });

  it("opens the involved object, not the event, when a row is clicked", async () => {
    const user = userEvent.setup();
    open();
    await waitFor(() => expect(cells().length).toBe(4));

    await user.click(row("BackOff"));
    // The event's own key is `billing/api-7.17a`; what the reader wants is the
    // pod it is about, in the namespace the key carried.
    expect(routes()).toContain("/k/Pod/billing/api-7");
    expect(routes().some((r) => r.includes("17a"))).toBe(false);
  });

  it("routes a cluster-scoped involved object with no namespace", async () => {
    const user = userEvent.setup();
    open();
    await waitFor(() => expect(cells().length).toBe(4));

    await user.click(row("NodeReady"));
    expect(routes()).toContain("/k/Node/-/node-3");
  });

  it("inherits the workspace's namespace selection", async () => {
    setNamespaces("prod", ["shop"]);
    open();
    // A namespaced event is narrowed; a cluster-scoped one — which belongs to
    // no namespace at all — falls outside any selection that names one.
    await waitFor(() => expect(reasons()).toEqual(["Scheduled", "Unhealthy"]));
    expect(eyebrowText()).toContain("2 events · 1 warning");
  });

  it("asks the design's question from the header chip", async () => {
    const user = userEvent.setup();
    open();
    await waitFor(() => expect(cells().length).toBe(4));

    await user.click(screen.getByRole("button", { name: /^Group by cause/ }));
    expect(asked).toEqual(["What do these warning events have in common?"]);
  });

  it("asks about one event from its row, without opening it", async () => {
    const user = userEvent.setup();
    open();
    await waitFor(() => expect(cells().length).toBe(4));
    const before = routes().length;

    await user.click(within(row("BackOff")).getByRole("button", { name: /^Ask/ }));
    expect(asked).toEqual(["Explain this event: BackOff — Back-off restarting failed container api"]);
    // Asking about a row is not opening it.
    expect(routes().length).toBe(before);
  });

  it("keeps the ask column last when a column is hidden", async () => {
    toggleColumn("events", "count");
    open();
    await waitFor(() => expect(cells().length).toBe(4));
    expect(headers()).toEqual(["Type", "Reason", "Object", "Namespace", "Message", "Age", ""]);
  });

  it("says it is loading before the stream has answered", async () => {
    initialRows = null;
    open();
    await waitFor(() => expect(screen.getAllByText("Loading events").length).toBeGreaterThan(0));
    expect(document.querySelector("tbody")).toBeNull();
  });

  it("tells the cluster having none from a filter hiding them", async () => {
    initialRows = [];
    open();
    await waitFor(() => expect(screen.getByText("No events")).toBeTruthy());
    expect(
      screen.getByText("prod-eu has no events in the namespaces you are looking at."),
    ).toBeTruthy();
  });

  it("names what failed, and offers a way back, when nothing arrived", async () => {
    const user = userEvent.setup();
    initialRows = null;
    open();
    await waitFor(() => expect(watchResource).toHaveBeenCalledTimes(1));
    await act(async () => stream.error("watch closed: 401"));

    // The screen's own title, which names WHAT failed, plus the
    // classification, which says what to do about it. The watch's own words
    // are folded away rather than dropped.
    expect(screen.getByText("Could not list events on prod-eu")).toBeTruthy();
    expect(screen.getByText(/rejected your credentials/)).toBeTruthy();
    const folded = document.querySelector('[data-slot="raw"]') as HTMLDetailsElement;
    expect(folded.open).toBe(false);
    expect(folded.textContent).toContain("watch closed: 401");

    initialRows = EVENTS;
    await user.click(screen.getByRole("button", { name: "Retry" }));
    await waitFor(() => expect(cells().length).toBe(4));
  });

  it("keeps stale rows on screen, with the warning above the scrolling area", async () => {
    open();
    await waitFor(() => expect(cells().length).toBe(4));
    await act(async () => stream.error("watch closed: connection reset"));

    // The last good list is the only information the reader has; emptying the
    // table would throw it away.
    expect(reasons()).toEqual(["BackOff", "Scheduled", "Unhealthy", "NodeReady"]);
    const alert = screen.getByText("These events are stale").closest("[role='status']");
    expect(alert).toBeTruthy();
    expect(screen.getByText("watch closed: connection reset")).toBeTruthy();
    // Pinned above the scrolling table: a warning the reader can scroll past
    // no longer warns anyone.
    const scroller = document.querySelector(".scroll");
    expect(scroller).toBeTruthy();
    expect(scroller?.contains(alert as Node)).toBe(false);
  });

  it("says so when the stream drops back to a poll", async () => {
    open();
    await waitFor(() => expect(cells().length).toBe(4));
    expect(screen.getByRole("status").textContent).toContain("Live");

    await act(async () => stream.status("reconnecting"));
    expect(screen.getByRole("status").textContent).toContain("Stream lost");
  });

  it("ranks the reasons of what is on screen in its right rail", async () => {
    open();
    await waitFor(() => expect(cells().length).toBe(4));

    // A second Unhealthy arrives: two events, one reason. The rail is the only
    // thing on the screen that says so.
    await act(async () =>
      stream.rows([
        ...EVENTS,
        {
          name: "shop/checkout.17e",
          namespace: "shop",
          type: "Warning",
          reason: "Unhealthy",
          object: "Deployment/checkout-api",
          message: "Liveness probe failed: context deadline exceeded",
          count: 1,
          age: "9s",
        },
      ]),
    );
    await waitFor(() => expect(cells().length).toBe(5));

    expect(railRows()).toEqual([
      ["Unhealthy", "2"],
      ["BackOff", "1"],
      ["Scheduled", "1"],
      ["NodeReady", "1"],
    ]);
  });

  it("searches for a reason when its rail row is clicked", async () => {
    const user = userEvent.setup();
    open();
    await waitFor(() => expect(cells().length).toBe(4));

    await user.click(screen.getByRole("button", { name: "Unhealthy 1" }));

    await waitFor(() => expect(reasons()).toEqual(["Unhealthy"]));
    // In the search box, not in some filter of the rail's own: the reader can
    // see what narrowed the table, and can clear it the way they clear a search.
    expect((search() as HTMLInputElement).value).toBe("Unhealthy");
  });

  it("still lists the other reasons after one has been clicked", async () => {
    const user = userEvent.setup();
    open();
    await waitFor(() => expect(cells().length).toBe(4));

    await user.click(screen.getByRole("button", { name: "Unhealthy 1" }));
    await waitFor(() => expect(reasons()).toEqual(["Unhealthy"]));

    // The rail is the way IN and the way BACK OUT. A rail that narrowed itself
    // to the row just clicked would take away the list of other things going
    // wrong — the very question it exists to answer — and leave the reader with
    // nothing on screen saying that the search box is what to clear.
    expect(railRows()).toEqual([
      ["BackOff", "1"],
      ["Scheduled", "1"],
      ["Unhealthy", "1"],
      ["NodeReady", "1"],
    ]);

    // And a second click changes the reader's mind, rather than dead-ending.
    await user.click(screen.getByRole("button", { name: "BackOff 1" }));
    await waitFor(() => expect(reasons()).toEqual(["BackOff"]));
    expect((search() as HTMLInputElement).value).toBe("BackOff");
  });

  it("reshapes the rail with the type control, which is a different question", async () => {
    const user = userEvent.setup();
    open();
    await waitFor(() => expect(cells().length).toBe(4));

    const segments = screen.getByRole("tablist", { name: "Type" });
    await user.click(within(segments).getByRole("tab", { name: "Normal" }));

    await waitFor(() =>
      expect(railRows()).toEqual([
        ["Scheduled", "1"],
        ["NodeReady", "1"],
      ]),
    );
  });

  it("keeps the rail whole while the search narrows the table beneath it", async () => {
    const user = userEvent.setup();
    open();
    await waitFor(() => expect(cells().length).toBe(4));

    await user.type(search(), "probe");
    await waitFor(() => expect(reasons()).toEqual(["Unhealthy"]));

    // The search is the reader's own narrowing and they can see it in the box.
    // The rail is what they narrow FROM, so it does not follow the box down.
    expect(railRows()).toEqual([
      ["BackOff", "1"],
      ["Scheduled", "1"],
      ["Unhealthy", "1"],
      ["NodeReady", "1"],
    ]);
  });

  it("leaves the rail blank, not boxed, when nothing is in scope at all", async () => {
    const user = userEvent.setup();
    // billing holds one event and it is a Warning, so asking for the Normal
    // ones leaves the screen — table and rail alike — with nothing to show.
    setNamespaces("prod", ["billing"]);
    open();
    await waitFor(() => expect(reasons()).toEqual(["BackOff"]));

    const segments = screen.getByRole("tablist", { name: "Type" });
    await user.click(within(segments).getByRole("tab", { name: "Normal" }));
    await waitFor(() => expect(reasons()).toEqual([]));

    expect(railRows()).toEqual([]);
    // The rail's own head and nothing else — §8 gives an empty rail no copy.
    expect(screen.getByRole("complementary", { name: "By reason" }).textContent).toBe("By reason");
  });

  it("stands aside when no cluster is in focus", () => {
    resetContexts();
    setContexts([]);
    render(
      <ConsoleProvider>
        <Events route="/events" />
      </ConsoleProvider>,
    );
    expect(screen.getByText("No cluster in focus")).toBeTruthy();
    expect(screen.getByText("Pick a cluster in the rail to list its events.")).toBeTruthy();
  });
});
