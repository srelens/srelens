import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, act, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

/**
 * The forwards store is core's — module-level, driven by the backend — so the
 * rows are supplied at that boundary rather than by starting a real tunnel.
 * The getter hands back the same array until it is swapped, which is what
 * `useSyncExternalStore` requires of it.
 *
 * **`forwardAddress` and `toKubectl` stay REAL.** They are the two things this
 * screen is forbidden from re-deriving, and a test that mocked them would
 * assert the screen calls a stub rather than that a web reader gets an address
 * they can actually open. The platform is flipped one layer lower instead —
 * `@srelens/core/platform` resolves to the same module `forward.ts` imports
 * `isTauri` from — so `forwardAddress` computes for real on both platforms.
 * `describeError` stays real for the same reason.
 */
const platform = vi.hoisted(() => ({ isTauri: vi.fn(() => true) }));
vi.mock("@srelens/core/platform", async (orig) => ({
  ...(await orig<typeof import("@srelens/core/platform")>()),
  isTauri: platform.isTauri,
}));

const store = vi.hoisted(() => ({
  list: [] as unknown[],
  listeners: new Set<() => void>(),
}));
const core = vi.hoisted(() => ({
  stopPortForward: vi.fn(),
  // The dialog's own write, so what the header's `New forward` finally starts —
  // and the cluster it starts it on — can be read rather than inferred.
  startPortForward: vi.fn(),
  rehydrateForwards: vi.fn(),
  openExternal: vi.fn(),
  // Only so the mounted dialog has something to list. What it does with them
  // is `NewForwardDialog.test.tsx`'s business; this file only cares that both
  // `New forward` buttons reach it.
  listNamespaces: vi.fn(),
  listServices: vi.fn(),
  listPods: vi.fn(),
}));
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  getForwards: () => store.list,
  subscribeForwards: (l: () => void) => {
    store.listeners.add(l);
    return () => store.listeners.delete(l);
  },
  ...core,
}));

import { type ActiveForward, type ClusterContext, kindToForwardTarget, toKubectl } from "@srelens/core";
import { Forwards } from "./Forwards";
import { resetContexts, setContexts } from "../lib/clusters";
import { setActiveCluster, setState } from "../lib/tabsStore";
import { defaultState } from "../lib/tabs";

const ROUTE = "/forwards";

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;

/**
 * §13's own four rows: four tunnels across three clusters, one of them
 * flapping, and a Pod among the Services so the target prefix has something to
 * be wrong about.
 *
 * The byte totals are chosen so the sum matches NO individual row — 54.5 MB is
 * not 1.2, 44.1, 0.312 or 8.9 — because a traffic badge asserted against a
 * single-row fixture passes whether it sums or merely echoes.
 */
function fixture(now: number): ActiveForward[] {
  return [
    {
      id: 1,
      context: "prod-eu",
      namespace: "checkout",
      kind: "Service",
      name: "checkout-api",
      localPort: 8080,
      remotePort: 8080,
      status: "active",
      bytesMoved: 1_200_000,
      startedAt: now - 18 * MINUTE,
    },
    {
      id: 2,
      context: "prod-eu",
      namespace: "observability",
      kind: "Service",
      name: "prometheus",
      localPort: 9090,
      remotePort: 9090,
      status: "active",
      bytesMoved: 44_100_000,
      startedAt: now - 2 * HOUR - 4 * MINUTE,
    },
    {
      id: 3,
      context: "prod-us",
      namespace: "search",
      kind: "Pod",
      name: "search-indexer-0",
      localPort: 6060,
      remotePort: 6060,
      status: "active",
      bytesMoved: 312_000,
      startedAt: now - 6 * MINUTE,
    },
    {
      id: 4,
      context: "staging",
      namespace: "identity",
      kind: "Service",
      // The one row whose ports differ from each other, so a cell that printed
      // the wrong one of the two would be caught rather than agree by accident.
      name: "identity-gateway",
      localPort: 8443,
      remotePort: 443,
      status: "reconnecting",
      bytesMoved: 8_900_000,
      startedAt: now - 51 * MINUTE,
    },
  ];
}

/**
 * Two clusters for the rail to move between. This screen lists every cluster's
 * forwards, but a NEW one is made in exactly one — whichever the rail had when
 * the dialog was asked for.
 */
const PROD: ClusterContext = {
  name: "prod-eu",
  stableId: "prod",
  cluster: "prod",
  server: "https://prod",
  isCurrent: true,
  sourceFile: "/home/dana/.kube/config",
  authKind: "client certificate",
};
const STAGE: ClusterContext = { ...PROD, name: "stage-eu", stableId: "stage", cluster: "stage", server: "https://stage", isCurrent: false };

/** Both clusters in the workspace, with `PROD` in focus — the rail's start. */
function withClusters() {
  resetContexts();
  setContexts([PROD, STAGE]);
  setState(defaultState([PROD, STAGE]));
}

let NOW = 0;
let windowOpen: ReturnType<typeof vi.fn>;

beforeEach(() => {
  vi.clearAllMocks();
  platform.isTauri.mockReturnValue(true);
  core.stopPortForward.mockResolvedValue(undefined);
  core.rehydrateForwards.mockResolvedValue(undefined);
  core.openExternal.mockResolvedValue(undefined);
  // A spy only so it can be asserted UNREACHED: `window.open` opens nothing at
  // all inside a Tauri WebView, and does it silently (#348).
  windowOpen = vi.fn();
  Object.defineProperty(window, "open", { value: windowOpen, configurable: true, writable: true });
  core.listNamespaces.mockResolvedValue({ namespaces: ["checkout"] });
  core.listServices.mockResolvedValue({ services: [] });
  core.listPods.mockResolvedValue({ pods: [] });
  core.startPortForward.mockImplementation(async (req: { localPort?: number }) => ({
    id: 9,
    localPort: req.localPort ?? 0,
    startedAt: Date.now(),
  }));
  // No cluster in the rail by default — every test above this line predates
  // the rail mattering to this screen, and reads the same as it always did.
  resetContexts();
  NOW = Date.now();
  store.list = fixture(NOW);
  store.listeners.clear();
});

/** Swap the array the way the store does — a new identity, then a notify. */
function setForwards(next: ActiveForward[]) {
  act(() => {
    store.list = next;
    for (const l of store.listeners) l();
  });
}

function open() {
  return render(<Forwards route={ROUTE} />);
}

const headers = () =>
  Array.from(document.querySelectorAll("thead th")).map((th) => th.textContent?.trim() ?? "");
const rowFor = (name: string) => screen.getByText(name).closest("tr") as HTMLElement;
const cells = (row: HTMLElement) =>
  Array.from(row.querySelectorAll("td")).map((td) => td.textContent?.trim() ?? "");
const cell = (row: HTMLElement, i: number) => row.querySelectorAll("td")[i] as HTMLElement;

/** jsdom ships no clipboard at all, so there is nothing to spy on. */
function stubClipboard(
  writeText = vi.fn<(text: string) => Promise<void>>().mockResolvedValue(undefined),
) {
  Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
  return writeText;
}

describe("Forwards — the table", () => {
  it("draws §13's columns in §13's order", () => {
    open();
    expect(headers()).toEqual([
      "Target",
      "Cluster",
      "Local",
      "Remote",
      "State",
      "Traffic",
      "Age",
      "",
    ]);
  });

  it("names the target through core's mapping, not a second copy of it", () => {
    open();
    // The mapping this screen used to keep its own `{ Service: "svc" }` table
    // for. Read from core so a drift between the cell and the copied command
    // is not possible rather than merely unlikely.
    expect(kindToForwardTarget("Service")).toBe("svc");
    expect(kindToForwardTarget("Pod")).toBe("pod");
    expect(cells(rowFor("svc/checkout-api"))[0]).toContain(
      `${kindToForwardTarget("Service")}/checkout-api`,
    );
    expect(cells(rowFor("pod/search-indexer-0"))[0]).toContain(
      `${kindToForwardTarget("Pod")}/search-indexer-0`,
    );
  });

  it("names the target the way kubectl does, over its namespace", () => {
    open();
    const svc = cell(rowFor("svc/checkout-api"), 0);
    expect(within(svc).getByText("svc/checkout-api")).toBeTruthy();
    expect(within(svc).getByText("checkout")).toBeTruthy();

    // A Pod is `pod/`, not `svc/` — the prefix is read off the kind, not
    // assumed from the commoner case.
    const pod = cell(rowFor("pod/search-indexer-0"), 0);
    expect(within(pod).getByText("pod/search-indexer-0")).toBeTruthy();
    expect(within(pod).getByText("search")).toBeTruthy();
  });

  it("stops offering to open a dead tunnel's address", async () => {
    // The address of a tunnel that has given up answers nothing. Leaving it
    // pressable is the #348 shape this branch keeps removing — a control that
    // looks like it works and does not — and here it is worse than a no-op,
    // because it would raise a browser tab onto a refused connection.
    open();
    expect(screen.getByRole("button", { name: "localhost:8080" })).toBeTruthy();

    store.list = fixture(NOW).map((f) =>
      f.id === 1 ? { ...f, status: "failed" as const } : f,
    );
    for (const l of store.listeners) l();

    await waitFor(() => expect(screen.queryByRole("button", { name: "localhost:8080" })).toBeNull());
    // Still readable — the reader has to see WHICH tunnel died.
    expect(screen.getByText("localhost:8080")).toBeTruthy();
    // And the live ones are untouched.
    expect(screen.getByRole("button", { name: "localhost:9090" })).toBeTruthy();
  });

  it("keeps a long context name inside its own cell", () => {
    // A kubeconfig context is user-chosen and routinely long. Seen against a
    // real cluster, `m01-1786968575165/kubernetes-admin@cluster.local` drew
    // straight over the Local cell beside it and both were unreadable.
    //
    // `truncate` sets `overflow: hidden`, which does nothing to an inline box,
    // so the cell has to be a block for the ellipsis to happen at all. jsdom
    // lays nothing out, so this asserts the mechanism rather than the pixels —
    // the third time column overflow has shipped on this project, and every
    // time it was invisible to the suite.
    render(<Forwards route="/forwards" />);
    const cluster = rowFor("svc/checkout-api").querySelectorAll("td")[1];
    const inner = cluster.querySelector("span");
    expect(inner?.className).toContain("truncate");
    expect(inner?.className).toContain("block");
  });

  it("gives each row its cluster, ports, traffic and age", () => {
    open();
    expect(cells(rowFor("svc/identity-gateway")).slice(1, 7)).toEqual([
      "staging",
      // Desktop: the address really is the loopback port.
      "localhost:8443",
      ":443",
      "Reconnecting",
      "8.9 MB",
      "51m",
    ]);
    expect(cells(rowFor("pod/search-indexer-0")).slice(5, 7)).toEqual(["312 KB", "6m"]);
    expect(cells(rowFor("svc/prometheus")).slice(5, 7)).toEqual(["44.1 MB", "2h"]);
  });

  it("counts the tunnels and the clusters they cross", () => {
    open();
    // Four forwards, three distinct contexts — the count is over the contexts
    // present, not over the rows.
    expect(screen.getByText(/Active tunnels · 4 across 3 clusters/i)).toBeTruthy();
  });

  it("says a single cluster in the singular", () => {
    setForwardsBeforeMount([fixture(NOW)[0]]);
    open();
    expect(screen.getByText(/Active tunnels · 1 across 1 cluster$/i)).toBeTruthy();
  });

  it("badges the traffic every tunnel has moved, added up", () => {
    open();
    // 1.2 + 44.1 + 0.312 + 8.9 MB. Deliberately not equal to any one row.
    expect(screen.getByText("54.5 MB moved")).toBeTruthy();
  });

  it("re-renders when the store changes", () => {
    open();
    expect(screen.queryByText("svc/prometheus")).toBeTruthy();
    setForwards(fixture(NOW).filter((f) => f.id !== 2));
    expect(screen.queryByText("svc/prometheus")).toBeNull();
    expect(screen.getByText(/Active tunnels · 3 across 3 clusters/i)).toBeTruthy();
  });
});

describe("Forwards — the state word", () => {
  it("reads a healthy tunnel plainly and a flapping one in the warning tone", () => {
    open();
    const active = cell(rowFor("svc/checkout-api"), 4).querySelector(".status") as HTMLElement;
    expect(active.textContent).toBe("Active");
    expect(active.getAttribute("data-kind")).toBe("success");
    // §13's asymmetric colouring rule: a good state is not worth the ink.
    expect(active.getAttribute("data-bad")).toBeNull();

    const flapping = cell(rowFor("svc/identity-gateway"), 4).querySelector(
      ".status",
    ) as HTMLElement;
    expect(flapping.textContent).toBe("Reconnecting");
    expect(flapping.getAttribute("data-kind")).toBe("warning");
    expect(flapping.getAttribute("data-bad")).toBe("true");
  });

  it("reads a forward that gave up as failed, in the severe tone", () => {
    // §13 draws only `active` and `reconnecting` and says "else warn". `failed`
    // is core's third status and it is not a warning — the tunnel is gone.
    const failed = fixture(NOW).map((f) => (f.id === 4 ? { ...f, status: "failed" as const } : f));
    setForwardsBeforeMount(failed);
    open();
    const gone = cell(rowFor("svc/identity-gateway"), 4).querySelector(".status") as HTMLElement;
    expect(gone.textContent).toBe("Failed");
    expect(gone.getAttribute("data-kind")).toBe("danger");
    expect(gone.getAttribute("data-bad")).toBe("true");
  });
});

/**
 * A tunnel that gave up, as the backend leaves it: `failed`, with the raw
 * string the cluster actually said attached. Row 4, so the fixture around it
 * still has three live tunnels across two clusters to be counted apart from.
 */
const GAVE_UP = "ApiError: Unauthorized (Status { metadata: Some(ListMeta { .. }) })";

function withDeadGateway(now: number): ActiveForward[] {
  return fixture(now).map((f) =>
    f.id === 4 ? { ...f, status: "failed" as const, error: GAVE_UP } : f,
  );
}

describe("Forwards — a tunnel that died on its own", () => {
  it("says why, in words rather than in Rust", () => {
    setForwardsBeforeMount(withDeadGateway(NOW));
    open();
    const state = cell(rowFor("svc/identity-gateway"), 4);
    expect(within(state).getByText("Failed")).toBeTruthy();
    // `describeError`'s classification, not the struct the cluster sent.
    expect(within(state).getByText("Not authorized")).toBeTruthy();
    // The struct is offered, folded away — never printed at the reader and
    // never in a title attribute, which is the rule a Secret leaked through.
    const raw = state.querySelector('[data-slot="raw"]');
    expect(raw?.textContent).toContain("ApiError");
    // Said as "nowhere but inside the disclosure", because `textContent`
    // reads a closed `details` too: a cell that printed the struct beside
    // the word would satisfy any weaker form of this.
    const outsideTheDisclosure = state.cloneNode(true) as HTMLElement;
    outsideTheDisclosure.querySelector('[data-slot="raw"]')?.remove();
    expect(outsideTheDisclosure.textContent).toContain("Not authorized");
    expect(outsideTheDisclosure.textContent).not.toContain("ApiError");
    expect(outsideTheDisclosure.textContent).not.toContain("Status { metadata");
  });

  it("says nothing about a tunnel that is merely flapping", () => {
    // Row 4 is `reconnecting`, and it HAS a reason: the backend sends its
    // error with every retry, and the store keeps the latest. Saying it
    // matters — a fixture with no error on this row passes for a screen that
    // prints the reason under any row that has one, which is a tunnel that
    // is coming back captioned as one that is gone.
    setForwardsBeforeMount(
      fixture(NOW).map((f) => (f.id === 4 ? { ...f, error: "connection reset by peer" } : f)),
    );
    open();
    const state = cell(rowFor("svc/identity-gateway"), 4);
    expect(state.querySelector('[data-slot="raw"]')).toBeNull();
    expect(state.textContent).toBe("Reconnecting");
  });

  it("keeps a reasonless failure to the one word it has", () => {
    // `forward:closed` can arrive with nothing attached. "Failed" alone is
    // the whole of what is known, and inventing a sentence under it would be
    // worse than the silence.
    setForwardsBeforeMount(
      fixture(NOW).map((f) => (f.id === 4 ? { ...f, status: "failed" as const } : f)),
    );
    open();
    const state = cell(rowFor("svc/identity-gateway"), 4);
    expect(state.textContent).toBe("Failed");
    expect(state.querySelector('[data-slot="raw"]')).toBeNull();
  });

  it("offers a dismissal, where a live tunnel is offered a stop", async () => {
    setForwardsBeforeMount(withDeadGateway(NOW));
    open();
    const dead = within(rowFor("svc/identity-gateway"));
    // Nothing left to stop: the tunnel already stopped itself.
    expect(dead.queryByRole("button", { name: /stop forwarding/i })).toBeNull();
    expect(dead.getByRole("button", { name: /dismiss/i })).toBeTruthy();

    const live = within(rowFor("svc/checkout-api"));
    expect(live.getByRole("button", { name: /stop forwarding/i })).toBeTruthy();
    expect(live.queryByRole("button", { name: /dismiss/i })).toBeNull();
  });

  it("dismisses by id, telling the backend to forget the tunnel", async () => {
    setForwardsBeforeMount(withDeadGateway(NOW));
    open();
    await userEvent.click(
      within(rowFor("svc/identity-gateway")).getByRole("button", { name: /dismiss/i }),
    );
    // `stopPortForward`, not a local delete: the manager holds a gave-up
    // forward until `stop` is called, and a set of dropped ids in the page
    // cannot survive the reload that would raise the row again.
    expect(core.stopPortForward).toHaveBeenCalledWith(4);
    expect(core.stopPortForward).toHaveBeenCalledTimes(1);
  });

  it("says why a dismissal was refused, in words rather than in Rust", async () => {
    core.stopPortForward.mockRejectedValue(new Error("handler error: no such forward"));
    setForwardsBeforeMount(withDeadGateway(NOW));
    open();
    await userEvent.click(
      within(rowFor("svc/identity-gateway")).getByRole("button", { name: /dismiss/i }),
    );
    const title = await screen.findByText(/Could not dismiss svc\/identity-gateway/i);
    const alert = title.closest("[data-tone]") as HTMLElement;
    expect(alert.textContent).toContain("no such forward");
    expect(alert.textContent).not.toContain("handler error:");
  });

  it("counts the live tunnels, and the dead one apart from them", () => {
    setForwardsBeforeMount(withDeadGateway(NOW));
    open();
    // Four rows, three live, across TWO live clusters — staging's only
    // tunnel is the dead one. Every number here differs from the number a
    // count over all four rows would give.
    expect(screen.getByText(/Active tunnels · 3 across 2 clusters · 1 failed/i)).toBeTruthy();
    expect(document.querySelectorAll("tbody tr")).toHaveLength(4);
  });

  it("keeps the dead tunnel's traffic in the total it moved", () => {
    setForwardsBeforeMount(withDeadGateway(NOW));
    open();
    // 8.9 MB really crossed that tunnel before it died. A badge that dropped
    // it would read as data loss on the way to reading as a smaller total.
    expect(screen.getByText("54.5 MB moved")).toBeTruthy();
  });

  it("puts the reason in no title attribute", () => {
    setForwardsBeforeMount(withDeadGateway(NOW));
    open();
    const titles = Array.from(document.querySelectorAll("[title]")).map(
      (el) => el.getAttribute("title") ?? "",
    );
    expect(titles.join("\n")).not.toContain("ApiError");
  });
});

describe("Forwards — the row's actions", () => {
  it("copies the kubectl command core writes, not one assembled here", async () => {
    const writeText = stubClipboard();
    open();
    await userEvent.click(
      within(rowFor("svc/checkout-api")).getByRole("button", {
        name: /copy kubectl command/i,
      }),
    );
    expect(writeText).toHaveBeenCalledWith(
      toKubectl({
        action: "port-forward",
        kind: "Service",
        name: "checkout-api",
        context: "prod-eu",
        namespace: "checkout",
        localPort: 8080,
        remotePort: 8080,
      }),
    );
    // And the command really is the port-forward one, so the assertion above
    // is not two identical mistakes agreeing.
    expect(writeText.mock.calls[0][0]).toContain("port-forward svc/checkout-api 8080:8080");
    expect(
      await within(rowFor("svc/checkout-api")).findByRole("button", {
        name: /copied kubectl command/i,
      }),
    ).toBeDefined();
    expect(screen.getByRole("status").textContent).toBe("Copied to clipboard");
  });

  it("copies the loopback address on the desktop", async () => {
    const writeText = stubClipboard();
    open();
    await userEvent.click(
      within(rowFor("svc/checkout-api")).getByRole("button", { name: /copy address/i }),
    );
    expect(writeText).toHaveBeenCalledWith("localhost:8080");
  });

  it("copies the proxy address in the browser, where a container's loopback is unreachable", async () => {
    // The whole reason §13's literal `http://localhost:<local>` is not shipped.
    platform.isTauri.mockReturnValue(false);
    const writeText = stubClipboard();
    open();
    await userEvent.click(
      within(rowFor("svc/checkout-api")).getByRole("button", { name: /copy address/i }),
    );
    const copied = writeText.mock.calls[0][0] as string;
    expect(copied).toBe(`${window.location.origin}/pf/1/`);
    // Said twice on purpose: jsdom's own origin contains "localhost", so the
    // equality above would still hold for a screen that had hardcoded the
    // desktop answer at some other port. This is the property.
    expect(copied).toContain("/pf/1/");
    expect(copied).not.toBe("localhost:8080");
  });

  it("shows the reachable address in the Local cell, the same one it copies", async () => {
    platform.isTauri.mockReturnValue(false);
    const writeText = stubClipboard();
    open();
    const shown = cells(rowFor("svc/checkout-api"))[2];
    expect(shown).toBe(`${window.location.origin}/pf/1/`);
    await userEvent.click(
      within(rowFor("svc/checkout-api")).getByRole("button", { name: /copy address/i }),
    );
    expect(writeText).toHaveBeenCalledWith(shown);
  });

  it("shows and announces a rejected address copy without claiming it succeeded", async () => {
    stubClipboard(
      vi.fn<(text: string) => Promise<void>>().mockRejectedValue(new Error("permission denied")),
    );
    open();
    await userEvent.click(
      within(rowFor("svc/checkout-api")).getByRole("button", { name: /copy address/i }),
    );

    expect(
      await within(rowFor("svc/checkout-api")).findByRole("button", {
        name: /copy address failed/i,
      }),
    ).toBeDefined();
    expect(screen.queryByRole("button", { name: /copied address/i })).toBeNull();
    expect(screen.getByRole("status").textContent).toBe("Could not copy to clipboard");
  });

  it("stops the forward the button is standing in, by id", async () => {
    open();
    await userEvent.click(
      within(rowFor("svc/identity-gateway")).getByRole("button", { name: /stop forwarding/i }),
    );
    // Id 4, not the first row's 1 and not the row's index.
    expect(core.stopPortForward).toHaveBeenCalledWith(4);
    expect(core.stopPortForward).toHaveBeenCalledTimes(1);
  });

  it("says why a stop was refused, in words rather than in Rust", async () => {
    core.stopPortForward.mockRejectedValue(
      new Error("ApiError: Unauthorized (Status { metadata: Some(ListMeta { .. }) })"),
    );
    open();
    await userEvent.click(
      within(rowFor("svc/checkout-api")).getByRole("button", { name: /stop forwarding/i }),
    );
    await waitFor(() =>
      expect(screen.getByText(/Could not stop svc\/checkout-api/i)).toBeTruthy(),
    );
    // `describeError`'s own classification, not the struct.
    expect(screen.getByText(/rejected your credentials/i)).toBeTruthy();
    const raw = document.querySelector('[data-slot="raw"]');
    expect(raw?.textContent).toContain("ApiError");
    // The struct appears ONLY inside the disclosure — never as the message.
    const alert = screen.getByText(/rejected your credentials/i).closest("[data-tone]");
    expect(alert?.querySelector('[data-slot="raw"]')).toBeTruthy();
  });

  it("puts no address, port or command in a title attribute", () => {
    open();
    const titles = Array.from(document.querySelectorAll("[title]")).map(
      (el) => el.getAttribute("title") ?? "",
    );
    // The buttons DO carry names — that is what makes four rows of identical
    // glyphs navigable. What they must not carry is a value: the rule
    // `PairList` and `KV` were stripped for.
    expect(titles.length).toBeGreaterThan(0);
    const joined = titles.join("\n");
    // The address, either platform's form.
    expect(joined).not.toContain("localhost:");
    expect(joined).not.toContain("/pf/");
    // The command — its flags, its target and its port pair.
    expect(joined).not.toContain("--context");
    expect(joined).not.toContain("port-forward");
    // Any port at all: the ports these four rows are bound to.
    for (const port of ["8080", "9090", "6060", "8443", "443"]) {
      expect(joined).not.toContain(port);
    }
  });
});

describe("Forwards — the screen around the table", () => {
  it("adopts the forwards the backend is still running, once, on mount", async () => {
    open();
    // The web-mode leak this screen exists to close: the store is module-level
    // JavaScript and a reload empties it while the server keeps forwarding.
    await waitFor(() => expect(core.rehydrateForwards).toHaveBeenCalledTimes(1));
  });

  it("offers the New forward action from the header", () => {
    open();
    const actions = document.querySelector('[data-slot="screen-actions"]') as HTMLElement;
    expect(within(actions).getByRole("button", { name: "New forward" })).toBeTruthy();
  });

  it("opens §A.4's dialog from the header action", async () => {
    open();
    expect(screen.queryByRole("dialog")).toBeNull();
    const actions = document.querySelector('[data-slot="screen-actions"]') as HTMLElement;
    await userEvent.click(within(actions).getByRole("button", { name: "New forward" }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("New port forward")).toBeTruthy();
    await userEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  /**
   * Open the dialog on `PROD`, fill in §A.4's four fields from that cluster's
   * listings, then move the rail to `STAGE` under it.
   *
   * Since #357 a dialog covers only its own tab, so the rail is live behind
   * this one and `setActiveCluster` switches the active cluster in place with
   * nothing remounting — the same premise `ResourceMenu`'s own forward gate is
   * built on. This screen's door is the other one into the same dialog.
   */
  async function fillNewForwardThenMove() {
    withClusters();
    core.listServices.mockResolvedValue({ services: [{ name: "checkout-api", namespace: "checkout" }] });
    open();
    const actions = document.querySelector('[data-slot="screen-actions"]') as HTMLElement;
    await userEvent.click(within(actions).getByRole("button", { name: "New forward" }));
    await screen.findByRole("dialog");
    await waitFor(() => expect(core.listNamespaces).toHaveBeenCalledWith("prod-eu"));

    await userEvent.selectOptions(screen.getByLabelText("Namespace"), "checkout");
    await waitFor(() => expect(core.listServices).toHaveBeenCalledWith("prod-eu", "checkout"));
    await waitFor(() =>
      expect(
        within(screen.getByLabelText("Target")).queryByRole("option", { name: "svc/checkout-api" }),
      ).toBeTruthy(),
    );
    await userEvent.selectOptions(screen.getByLabelText("Target"), "svc/checkout-api");
    await userEvent.clear(screen.getByLabelText("Local port"));
    await userEvent.type(screen.getByLabelText("Local port"), "9099");
    await userEvent.clear(screen.getByLabelText("Remote port"));
    await userEvent.type(screen.getByLabelText("Remote port"), "8080");

    act(() => setActiveCluster(STAGE.stableId, STAGE.name));
  }

  it("keeps the new-forward dialog on the cluster it was opened against when the rail moves", async () => {
    await fillNewForwardThenMove();

    // The listings stay with the pinned cluster: a namespace select that
    // followed the rail would offer another cluster's namespaces under a
    // target picked from this one.
    expect(core.listNamespaces).toHaveBeenCalledTimes(1);
    expect(core.listNamespaces).not.toHaveBeenCalledWith("stage-eu");
    expect(core.listServices).not.toHaveBeenCalledWith("stage-eu", "checkout");
    // And the equivalent command names the cluster the forward will be made
    // in, which is the same one the banner does.
    expect(screen.getByText(/--context prod-eu\b/)).toBeTruthy();
    expect(screen.queryByText(/--context stage-eu\b/)).toBeNull();
    expect(screen.getByText("This still runs against prod-eu, not stage-eu")).toBeTruthy();
  });

  it("refuses the start until the reader confirms the cluster, then forwards on that one", async () => {
    await fillNewForwardThenMove();

    // Pressed while the divergence stands: nothing goes out.
    await userEvent.click(screen.getByRole("button", { name: "Start forward" }));
    expect(core.startPortForward).not.toHaveBeenCalled();

    await userEvent.click(screen.getByRole("checkbox", { name: "Yes, still forward on prod-eu." }));
    await userEvent.click(screen.getByRole("button", { name: "Start forward" }));

    await waitFor(() => expect(core.startPortForward).toHaveBeenCalledTimes(1));
    // `prod-eu`, where the target was picked — never `stage-eu`, where the
    // rail went. A tunnel to staging under a name read off production is the
    // whole of the defect.
    expect(core.startPortForward).toHaveBeenCalledWith(
      expect.objectContaining({
        context: "prod-eu",
        namespace: "checkout",
        kind: "Service",
        name: "checkout-api",
        localPort: 9099,
        remotePort: 8080,
      }),
    );
  });

  it("says nothing, and asks nothing, while the rail has not moved under the dialog", async () => {
    withClusters();
    open();
    const actions = document.querySelector('[data-slot="screen-actions"]') as HTMLElement;
    await userEvent.click(within(actions).getByRole("button", { name: "New forward" }));
    await screen.findByRole("dialog");
    await waitFor(() => expect(core.listNamespaces).toHaveBeenCalledWith("prod-eu"));

    expect(screen.queryByText(/This still runs against/)).toBeNull();
    expect(screen.queryByRole("checkbox", { name: /Yes, still/ })).toBeNull();
  });

  /**
   * The rail is EMPTY when the dialog is asked for — every test above this
   * describe's own cluster fixtures runs that way, and it is the state a reader
   * is in before anything has connected.
   *
   * `useClusterGate` treats a pinned cluster as "nothing to compare" only when
   * it is `null`; `""` is a pinned cluster, so the door that pinned
   * `cluster?.name ?? ""` armed a divergence between an empty name and whatever
   * the reader selected next — a banner reading "This still runs against ,
   * not prod-eu" and a tick offering to "still forward on .". The dialog says
   * there is no cluster instead, once, at the top.
   */
  it("names no divergence when there was no cluster to pin", async () => {
    open();
    const actions = document.querySelector('[data-slot="screen-actions"]') as HTMLElement;
    await userEvent.click(within(actions).getByRole("button", { name: "New forward" }));
    await screen.findByRole("dialog");

    // The reader does the one thing the dialog asks: puts a cluster in focus.
    act(() => withClusters());

    expect(screen.queryByText(/This still runs against/)).toBeNull();
    expect(screen.queryByRole("checkbox", { name: /Yes, still/ })).toBeNull();
    // And no half-named refusal on the way out either.
    expect(screen.queryByText(/This runs on/)).toBeNull();
  });

  it("says there is no cluster, rather than a dialog that cannot start and does not say why", async () => {
    open();
    const actions = document.querySelector('[data-slot="screen-actions"]') as HTMLElement;
    await userEvent.click(within(actions).getByRole("button", { name: "New forward" }));
    await screen.findByRole("dialog");
    // Said at the top, once, rather than discovered by pressing a dead button:
    // with no cluster there is nothing to list and nothing to forward.
    expect(screen.getByText("No cluster in focus")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Start forward" }).hasAttribute("disabled")).toBe(true);
  });

  it("says nothing about a divergence when the rail empties under the dialog", async () => {
    // The other direction: opened on prod-eu, and the rail loses its selection
    // while it is open. The forward still runs on prod-eu — which is what
    // pinning promised — so there is nothing to compare and nothing to say.
    withClusters();
    open();
    const actions = document.querySelector('[data-slot="screen-actions"]') as HTMLElement;
    await userEvent.click(within(actions).getByRole("button", { name: "New forward" }));
    await screen.findByRole("dialog");
    await waitFor(() => expect(core.listNamespaces).toHaveBeenCalledWith("prod-eu"));

    act(() => resetContexts());

    expect(screen.queryByText(/This still runs against/)).toBeNull();
    expect(screen.queryByRole("checkbox", { name: /Yes, still/ })).toBeNull();
    // Still pinned to the cluster it was opened against — which is exactly why
    // an empty rail is not news, and why the no-cluster state is not shown
    // either: this dialog has a cluster.
    expect(screen.queryByText("No cluster in focus")).toBeNull();
    expect(core.listNamespaces).toHaveBeenCalledTimes(1);
    expect(core.listNamespaces).toHaveBeenCalledWith("prod-eu");
  });

  it("opens the SAME dialog from the empty state's way out", async () => {
    // The two buttons share one handler, and the dialog is mounted beside the
    // body rather than inside either branch — so a reader with no tunnels gets
    // the dialog too. An assertion on the header alone would pass either way.
    setForwardsBeforeMount([]);
    open();
    const empty = screen.getByText("No port forwards").closest("div")
      ?.parentElement as HTMLElement;
    await userEvent.click(within(empty).getByRole("button", { name: "New forward" }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("New port forward")).toBeTruthy();
    // And the emptiness is still behind it.
    expect(screen.getByText("No port forwards")).toBeTruthy();
  });

  it("ships the empty state §13 defines and never renders", () => {
    setForwardsBeforeMount([]);
    open();
    expect(screen.getByText("No port forwards")).toBeTruthy();
    expect(
      screen.getByText(
        "Forward a service port to reach it from this machine. Nothing is exposed outside your laptop.",
      ),
    ).toBeTruthy();
    // The way out of the emptiness, which is the whole point of the slot.
    const empty = screen.getByText("No port forwards").closest("div")?.parentElement as HTMLElement;
    expect(within(empty).getByRole("button", { name: "New forward" })).toBeTruthy();
  });

  it("heads nothing over an empty screen", () => {
    setForwardsBeforeMount([]);
    open();
    // No pane head counting tunnels that are not there, and no badge claiming
    // a total that nothing moved. (`headers()` is not the assertion: `Table`
    // draws its own empty state over an empty `data`, so a screen that never
    // branched at all would still report no columns here.)
    expect(document.querySelector(".pane-head")).toBeNull();
    expect(screen.queryByText(/moved/)).toBeNull();
  });
});

describe("Forwards — the Local cell opens", () => {
  const localCell = (target: string) => cell(rowFor(target), 2);

  it("is a control, not a label", () => {
    open();
    // A span with an onClick would satisfy "the address is on screen and
    // clicking it works" and would be reachable by neither Tab nor Enter.
    // Named by the address it shows, the way a link is named by its text —
    // `getByRole`'s name is the computed accessible name, not the markup.
    const button = within(localCell("svc/checkout-api")).getByRole("button", {
      name: "localhost:8080",
    });
    expect(button.tagName).toBe("BUTTON");
  });

  it("keeps the column's mono face, and keeps a long address inside the cell", () => {
    platform.isTauri.mockReturnValue(false);
    open();
    // §13's Local column is a `code` cell. Making it a control must not cost
    // it the face that makes a host:port readable.
    const mono = localCell("svc/checkout-api").querySelector(".code") as HTMLElement;
    expect(mono.textContent).toBe(`${window.location.origin}/pf/1/`);
    // A proxy URL is long and the button is a flex box, whose items refuse to
    // shrink below their content without `min-w-0`. jsdom lays nothing out, so
    // this asserts the mechanism — the same way the Cluster cell's does, after
    // column overflow shipped unnoticed three times on this project.
    expect(mono.className).toContain("truncate");
    expect(mono.className).toContain("min-w-0");
  });

  it("opens the desktop address with the scheme a browser needs", async () => {
    open();
    await userEvent.click(within(localCell("svc/prometheus")).getByRole("button"));
    await waitFor(() => expect(core.openExternal).toHaveBeenCalledTimes(1));
    // `forwardAddress` answers a bare `localhost:9090` here. A bare authority
    // is NOT a URL: opened verbatim it resolves against the current page.
    expect(core.openExternal).toHaveBeenCalledWith("http://localhost:9090");
    // And it goes through core rather than the WebView's own dead `window.open`.
    expect(windowOpen).not.toHaveBeenCalled();
  });

  it("opens the proxy address in web mode, not a loopback the browser cannot reach", async () => {
    platform.isTauri.mockReturnValue(false);
    open();
    await userEvent.click(within(localCell("svc/checkout-api")).getByRole("button"));
    await waitFor(() => expect(core.openExternal).toHaveBeenCalledTimes(1));
    const url = core.openExternal.mock.calls[0][0] as string;
    expect(url).toBe(`${window.location.origin}/pf/1/`);
    // Said twice on purpose: jsdom's own origin contains "localhost", so the
    // equality above would still hold for a screen that had hardcoded the
    // desktop answer at some other port. This is the property.
    expect(url).toContain("/pf/1/");
    expect(url).not.toContain("localhost:8080");
  });

  it("opens the address the row it was clicked in is showing", async () => {
    open();
    await userEvent.click(within(localCell("svc/identity-gateway")).getByRole("button"));
    await waitFor(() => expect(core.openExternal).toHaveBeenCalledTimes(1));
    // The LOCAL port, 8443 — not the row's remote 443 and not the first row's.
    expect(core.openExternal).toHaveBeenCalledWith("http://localhost:8443");
  });

  it("says why an open was refused, in words rather than in Rust", async () => {
    core.openExternal.mockRejectedValue(
      new Error("handler error: No such file or directory (os error 2)"),
    );
    open();
    await userEvent.click(within(localCell("svc/checkout-api")).getByRole("button"));
    const title = await screen.findByText(/Could not open svc\/checkout-api/i);
    const alert = title.closest("[data-tone]") as HTMLElement;
    // The screen's own error surface, in its severe tone — not a raw backend
    // string dropped into a cell.
    expect(alert.getAttribute("data-tone")).toBe("sev");
    // `describeError`'s cleaning: `handler error:` is `CapabilityError`'s
    // Display prefix and is news to nobody.
    expect(alert.textContent).toContain("No such file or directory");
    expect(alert.textContent).not.toContain("handler error:");
  });
});

/** Seed the store before the screen mounts, for the cases about first paint. */
function setForwardsBeforeMount(next: ActiveForward[]) {
  store.list = next;
}
