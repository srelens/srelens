import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { act, render, screen, cleanup, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

/**
 * Only the BACKEND boundary is doubled. The session store, its emulators and
 * `describeError` all stay real — this screen's whole job is composing them,
 * and a test that stubbed the store would prove the screen calls a stub. Same
 * arrangement `TerminalView.test.tsx` uses, extended with the listing calls
 * `NewSessionMenu` makes when the header opens it.
 */
const core = vi.hoisted(() => ({
  startPodExec: vi.fn(),
  startLocalTerminal: vi.fn(),
  deletePod: vi.fn(),
  listNamespaces: vi.fn(),
  listPods: vi.fn(),
  listNodes: vi.fn(),
  getObject: vi.fn(),
  createNodeDebugPod: vi.fn(),
  isTauri: vi.fn(() => true),
  notify: { success: vi.fn(), error: vi.fn(), info: vi.fn() },
}));
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  ...core,
}));

/**
 * `@xterm/addon-fit` measures a real canvas, which jsdom cannot do. Doubled so
 * mounting the pane does not depend on a measurement that is always zero here;
 * the emulator itself stays the real xterm instance, because the theme this
 * screen dresses it in is read back off that instance.
 */
vi.mock("@xterm/addon-fit", () => ({
  FitAddon: class {
    activate() {}
    dispose() {}
    fit() {}
  },
}));

import type { ClusterContext } from "@srelens/core";
import { ConsoleProvider, useConsole } from "../console";
import { resetContexts, setContexts } from "../lib/clusters";
import {
  __resetSessionsForTests,
  endSession,
  getSessions,
  startPodSession,
  terminalFor,
} from "../lib/sessions";
import { defaultState } from "../lib/tabs";
import * as tabs from "../lib/tabsStore";
import { Terminals } from "./Terminals";

const CTX: ClusterContext = {
  name: "prod-eu",
  stableId: "prod",
  cluster: "prod",
  server: "https://prod",
  isCurrent: true,
  sourceFile: "/home/dana/.kube/config",
  authKind: "client certificate",
};

/** The cluster the rail can move to while the new-session menu is open. */
const STAGE: ClusterContext = {
  ...CTX,
  name: "stage-eu",
  stableId: "stage",
  cluster: "stage",
  server: "https://stage",
  isCurrent: false,
};

/** jsdom has no `matchMedia`; xterm's `CoreBrowserService` reads it on open(). */
function stubMatchMedia(target: typeof globalThis) {
  (target as unknown as { matchMedia: (query: string) => MediaQueryList }).matchMedia = (
    query: string,
  ) =>
    ({
      matches: false,
      media: query,
      onchange: null,
      addListener: () => {},
      removeListener: () => {},
      addEventListener: () => {},
      removeEventListener: () => {},
      dispatchEvent: () => false,
    }) as unknown as MediaQueryList;
}
stubMatchMedia(globalThis);
if (typeof window !== "undefined") stubMatchMedia(window as unknown as typeof globalThis);

if (!("ResizeObserver" in globalThis)) {
  (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}

/** The far end of a session: what `startPodExec` resolves to. */
function handle() {
  return { send: vi.fn(), resize: vi.fn(), close: vi.fn() };
}

let asked: string[] = [];

/** Records what the ask chip put to the console, which has no dock here. */
function AskSpy() {
  const console_ = useConsole();
  console_.registerSubmit((question) => {
    asked.push(question);
  });
  return null;
}

function draw() {
  asked = [];
  return render(
    <ConsoleProvider>
      <AskSpy />
      <Terminals route="/terminals" />
    </ConsoleProvider>,
  );
}

/** A pod session through the real store, so the row carries a real kind and title. */
async function openPod(pod: string, namespace = "checkout", container = "api") {
  return act(() => startPodSession({ context: CTX.name, namespace, pod, container }));
}

const paneHead = () => document.querySelector('[data-slot="main-head"]') as HTMLElement;
/**
 * A rail row, found INSIDE the rail. Scoped rather than global because the ask
 * chip's accessible name carries the active session's title too — a global
 * query for a session's name matches both, and which of them it happened to
 * find would be the assertion.
 */
const railRow = (name: RegExp) =>
  within(document.querySelector('[data-slot="rail-body"]') as HTMLElement).getByRole("button", {
    name,
  });
const sessionName = () => document.querySelector('[data-slot="session-name"]') as HTMLElement;
const headerActions = () =>
  document.querySelector('[data-slot="screen-actions"]') as HTMLElement;
/** Every xterm root currently in the document — one pane, one emulator. */
const attached = () => Array.from(document.querySelectorAll(".xterm"));

beforeEach(() => {
  vi.clearAllMocks();
  core.isTauri.mockReturnValue(true);
  core.startPodExec.mockImplementation(async () => handle());
  core.startLocalTerminal.mockImplementation(async () => handle());
  core.deletePod.mockResolvedValue({});
  core.listNamespaces.mockResolvedValue({ namespaces: [] });
  core.listPods.mockResolvedValue({ pods: [] });
  core.listNodes.mockResolvedValue({ nodes: [] });
  core.createNodeDebugPod.mockResolvedValue({});
  __resetSessionsForTests();
  resetContexts();
  setContexts([CTX]);
  tabs.setState(defaultState([CTX]));
  document.documentElement.removeAttribute("data-theme");
  document.documentElement.removeAttribute("style");
});

afterEach(() => {
  cleanup();
  __resetSessionsForTests();
  document.documentElement.removeAttribute("data-theme");
  document.documentElement.removeAttribute("style");
});

describe("Terminals", () => {
  it("lets the pane shrink, so the rail is not pushed off the window", async () => {
    // xterm sizes its content in explicit pixels, and a flex item's implicit
    // `min-width: auto` refuses to shrink below its content — so without
    // `min-w-0` the pane grows to the terminal's width and the 230px rail
    // goes off the right edge. Seen on a real screen: the rail read
    // "SESSIONS · 1 ATTACHE" with the D cut off.
    //
    // jsdom lays nothing out, so this asserts the mechanism. Fifth time this
    // property has bitten on this migration.
    draw();
    await openPod("checkout-api-5c8b7f2d9-mk3wl");
    const view = document.querySelector('[data-slot="terminal-body"]');
    expect(view?.className ?? "").toContain("min-w-0");
  });

  it("draws an ask control that is actually visible", () => {
    // `AskChip` is the ROW chip: `.row-ask` is `opacity: 0` until a `.tbl
    // tbody tr` is hovered, which is right for one of forty rows and
    // invisible on a header, where there is no row to hover. `Events.tsx` and
    // `Overview.tsx` already use a `Button` here and say why.
    //
    // jsdom does not apply the stylesheet, so this asserts the mechanism: the
    // header's control is not the row chip.
    draw();
    const ask = screen.getByRole("button", { name: /Draft a command/i });
    expect(ask.className).not.toContain("row-ask");
  });

  it("shows the session the rail selected, neither the first nor the newest", async () => {
    const first = await openPod("checkout-api-5c8b7f2d9-mk3wl");
    const middle = await openPod("search-indexer-0", "search", "indexer");
    const newest = await openPod("otel-collector-0", "observability", "otel");
    const user = userEvent.setup();
    draw();

    // The MIDDLE row, on purpose: with three sessions on screen, "shows the
    // active one" agrees with neither "shows the first one" nor "shows the
    // last one", so the assertion cannot pass by position.
    await user.click(railRow(/search-indexer-0/));

    expect(sessionName().textContent).toBe("search-indexer-0 · indexer");
    // And the pane is attached to THAT session's emulator, not merely titled
    // with its name: one xterm root on screen, and it is the middle one's.
    expect(attached()).toEqual([terminalFor(middle)?.element]);
    expect(attached()).not.toContain(terminalFor(first)?.element);
    expect(attached()).not.toContain(terminalFor(newest)?.element);
  });

  it("opens on the newest session, so one started from a row menu is what the tab shows", async () => {
    await openPod("checkout-api-5c8b7f2d9-mk3wl");
    const newest = await openPod("search-indexer-0", "search", "indexer");
    draw();

    expect(sessionName().textContent).toBe("search-indexer-0 · indexer");
    expect(attached()).toEqual([terminalFor(newest)?.element]);
  });

  it("keeps the active session when the rows around it come and go", async () => {
    const alpha = await openPod("alpha");
    const bravo = await openPod("bravo");
    const charlie = await openPod("charlie");
    const user = userEvent.setup();
    draw();

    await user.click(railRow(/bravo/));
    expect(sessionName().textContent).toBe("bravo · api");

    // Both neighbours go, from both ends: `charlie` shortens the array under
    // the pane and `alpha` shifts every index in it. A pane holding a position
    // rather than an id would follow the shuffle instead of the reader.
    await act(async () => endSession(charlie));
    expect(sessionName().textContent).toBe("bravo · api");
    await act(async () => endSession(alpha));

    expect(sessionName().textContent).toBe("bravo · api");
    expect(attached()).toEqual([terminalFor(bravo)?.element]);
  });

  it("names the active session in normal case, beside its state badge", async () => {
    await openPod("checkout-api-5c8b7f2d9-mk3wl");
    draw();

    // §14 asks for the pane head's name in normal case, against this design's
    // usual uppercase `.pane-head` — a class assertion, because the case is a
    // stylesheet property and the text node reads the same either way.
    expect(sessionName().classList.contains("normal-case")).toBe(true);
    expect(within(paneHead()).getByText("Attached")).toBeTruthy();
  });

  it("says in the footer that a shell is not gated", async () => {
    await openPod("checkout-api-5c8b7f2d9-mk3wl");
    draw();

    expect(
      screen.getByText(
        "Destructive commands inside a shell are not gated — the shell is your own session",
      ),
    ).toBeTruthy();
  });

  it("Clear clears the emulator and leaves the session running", async () => {
    const id = await openPod("checkout-api-5c8b7f2d9-mk3wl");
    const term = terminalFor(id);
    if (!term) throw new Error("expected a live emulator");
    const clear = vi.spyOn(term, "clear");
    const far = await core.startPodExec.mock.results[0].value;
    const user = userEvent.setup();
    draw();

    await user.click(screen.getByRole("button", { name: "Clear" }));

    expect(clear).toHaveBeenCalledTimes(1);
    // A Clear that ended the session would pass the assertion above. These
    // are what tell the two apart.
    expect(getSessions().map((s) => s.id)).toEqual([id]);
    expect(getSessions()[0].state).toBe("attached");
    expect(terminalFor(id)).toBe(term);
    expect(far.close).not.toHaveBeenCalled();
  });

  it("Detach ends the session, cleanup and all", async () => {
    const id = await act(() =>
      startPodSession({
        context: CTX.name,
        namespace: "kube-system",
        pod: "node-debug-abc",
        container: "debug",
        kind: "node",
        title: "eu-w4-c3-standard-a1",
      }),
    );
    const far = await core.startPodExec.mock.results[0].value;
    const user = userEvent.setup();
    draw();

    await user.click(screen.getByRole("button", { name: "Detach" }));

    expect(getSessions()).toHaveLength(0);
    expect(far.close).toHaveBeenCalled();
    expect(terminalFor(id)).toBeUndefined();
    // Detach is `endSession`, not a row removal that happens to look like it:
    // the privileged debug pod a node shell left on the cluster goes with it.
    expect(core.deletePod).toHaveBeenCalledWith(CTX.name, "kube-system", "node-debug-abc");
  });

  it("reports a session that never opened in described words, not the backend's", async () => {
    core.startPodExec.mockRejectedValueOnce("403 Forbidden");
    await openPod("locked-down");
    draw();

    expect(
      screen.getByText(/Your account doesn't have permission for this on the cluster/),
    ).toBeTruthy();
    expect(document.body.textContent ?? "").not.toContain("403 Forbidden");
    // Still selectable, still showing its transcript: the row is why it ended.
    expect(attached()).toHaveLength(1);
  });

  it("names the active session's cluster and namespace above the title", async () => {
    await openPod("checkout-api-5c8b7f2d9-mk3wl", "checkout");
    draw();

    expect(screen.getByRole("heading", { name: "Terminals" })).toBeTruthy();
    expect(document.querySelector(".crumb")?.textContent).toBe("prod-eu / checkout");
  });

  it("renders an empty state when nothing is open", () => {
    draw();

    expect(screen.getByText("No shell open")).toBeTruthy();
    expect(paneHead()).toBeNull();
    expect(attached()).toHaveLength(0);
  });

  it("opens the new-session menu from the header", async () => {
    const user = userEvent.setup();
    draw();

    await user.click(within(headerActions()).getByRole("button", { name: "New session" }));

    expect(await screen.findByRole("button", { name: "Start session" })).toBeTruthy();
  });

  /**
   * Open the menu on `CTX`, pick a namespace and a pod out of THAT cluster's
   * listings, then move the rail to `STAGE` under it.
   *
   * The menu is mounted on a boolean beside the screen's body, and since #357 a
   * dialog covers only its own tab — so the rail is live behind it and
   * `setActiveCluster` switches the active cluster in place with nothing
   * remounting.
   */
  async function pickPodThenMove(user: ReturnType<typeof userEvent.setup>) {
    setContexts([CTX, STAGE]);
    tabs.setState(defaultState([CTX, STAGE]));
    core.listNamespaces.mockResolvedValue({ namespaces: ["checkout"] });
    core.listPods.mockResolvedValue({ pods: [{ name: "checkout-api-0" }] });
    core.getObject.mockResolvedValue({
      object: {
        kind: "Pod",
        apiVersion: "v1",
        metadata: { name: "checkout-api-0", namespace: "checkout" },
        spec: { containers: [{ name: "api" }] },
        status: { phase: "Running", containerStatuses: [{ name: "api", ready: true, started: true, state: { running: {} } }] },
      },
    });
    draw();

    await user.click(within(headerActions()).getByRole("button", { name: "New session" }));
    await screen.findByRole("button", { name: "Start session" });
    await waitFor(() => expect(core.listNamespaces).toHaveBeenCalledWith("prod-eu"));

    await user.selectOptions(screen.getByLabelText("Namespace"), "checkout");
    await waitFor(() => expect(core.listPods).toHaveBeenCalledWith("prod-eu", "checkout"));
    await user.selectOptions(screen.getByLabelText("Pod"), "checkout-api-0");
    await waitFor(() =>
      expect(core.getObject).toHaveBeenCalledWith("prod-eu", "Pod", "checkout", "checkout-api-0"),
    );

    act(() => tabs.setActiveCluster(STAGE.stableId, STAGE.name));
  }

  it("keeps the new-session menu on the cluster it was opened against when the rail moves", async () => {
    const user = userEvent.setup();
    await pickPodThenMove(user);

    // The listings stay with the cluster the pod was picked in: a namespace
    // select that followed the rail would offer another cluster's namespaces
    // under a pod name read off this one.
    expect(core.listNamespaces).toHaveBeenCalledTimes(1);
    expect(core.listNamespaces).not.toHaveBeenCalledWith("stage-eu");
    expect(core.listPods).not.toHaveBeenCalledWith("stage-eu", "checkout");
    // And the divergence is said, rather than the menu quietly renaming its
    // own target: a shell writes nothing, so this states it and stops.
    expect(screen.getByText("This still runs against prod-eu, not stage-eu")).toBeTruthy();
  });

  it("opens the shell on the cluster the pod was picked in, not the one the rail moved to", async () => {
    const user = userEvent.setup();
    await pickPodThenMove(user);

    await user.click(screen.getByRole("button", { name: "Start session" }));

    await waitFor(() => expect(core.startPodExec).toHaveBeenCalledTimes(1));
    // `startPodExec(context, namespace, pod, …)` — the first three arguments
    // are the whole of the claim.
    expect(core.startPodExec.mock.calls[0].slice(0, 3)).toEqual([
      "prod-eu",
      "checkout",
      "checkout-api-0",
    ]);
  });

  it("says nothing while the rail has not moved under the menu", async () => {
    const user = userEvent.setup();
    setContexts([CTX, STAGE]);
    tabs.setState(defaultState([CTX, STAGE]));
    core.listNamespaces.mockResolvedValue({ namespaces: ["checkout"] });
    draw();

    await user.click(within(headerActions()).getByRole("button", { name: "New session" }));
    await screen.findByRole("button", { name: "Start session" });
    await waitFor(() => expect(core.listNamespaces).toHaveBeenCalledWith("prod-eu"));

    expect(screen.queryByText(/This still runs against/)).toBeNull();
  });

  it("hands the console a question about the session on screen", async () => {
    await openPod("checkout-api-5c8b7f2d9-mk3wl");
    const user = userEvent.setup();
    draw();

    await user.click(within(headerActions()).getByRole("button", { name: /Draft a command/ }));

    expect(asked).toHaveLength(1);
    expect(asked[0]).toContain("checkout-api-5c8b7f2d9-mk3wl · api");
  });

  it("dresses the emulator from the app's tokens rather than colours of its own", async () => {
    document.documentElement.style.setProperty("--surface-sunk", "#101014");
    document.documentElement.style.setProperty("--ink-soft", "#443f52");
    document.documentElement.style.setProperty("--accent", "#4b3bd6");
    document.documentElement.style.setProperty("--font-mono", '"Test Mono", monospace');
    const id = await openPod("checkout-api-5c8b7f2d9-mk3wl");
    draw();

    await waitFor(() => {
      expect(terminalFor(id)?.options.theme?.background).toBe("#101014");
    });
    expect(terminalFor(id)?.options.theme?.foreground).toBe("#443f52");
    expect(terminalFor(id)?.options.theme?.cursor).toBe("#4b3bd6");
    expect(terminalFor(id)?.options.fontFamily).toBe('"Test Mono", monospace');
  });

  it("re-reads the tokens when the theme changes under it", async () => {
    document.documentElement.style.setProperty("--surface-sunk", "#fafafc");
    const id = await openPod("checkout-api-5c8b7f2d9-mk3wl");
    draw();
    await waitFor(() => {
      expect(terminalFor(id)?.options.theme?.background).toBe("#fafafc");
    });

    // What a theme switch does: the same token, a different value, announced
    // by the attribute `applyNextThemeAttribute` writes on the root.
    await act(async () => {
      document.documentElement.style.setProperty("--surface-sunk", "#121118");
      document.documentElement.dataset.theme = "dark";
    });

    await waitFor(() => {
      expect(terminalFor(id)?.options.theme?.background).toBe("#121118");
    });
  });
});
