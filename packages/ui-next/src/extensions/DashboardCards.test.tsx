import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const core = vi.hoisted(() => ({
  listExtensions: vi.fn(),
  resolveDashboardCards: vi.fn(),
  onExtensionInventoryChanged: vi.fn(async () => () => {}),
  openExtensionView: vi.fn(),
}));
vi.mock("@srelens/core", async (original) => ({
  ...(await original<typeof import("@srelens/core")>()),
  ...core,
  isTauri: () => true,
}));
const namespaceOptions = vi.hoisted(() => ({ useNamespaceOptions: vi.fn() }));
vi.mock("@srelens/core/react", async (original) => ({
  ...(await original<typeof import("@srelens/core/react")>()),
  ...namespaceOptions,
}));

import {
  extensionCardRoute,
  type ClusterContext,
  type ExtensionDashboardCard,
  type InstalledExtension,
  type ResolvedDashboardCard,
} from "@srelens/core";
// jsdom has no ResizeObserver or scrollIntoView; the namespace picker's popover wants both.
HTMLElement.prototype.scrollIntoView ??= () => {};
if (!("ResizeObserver" in globalThis)) {
  (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}

import { DashboardCards } from "./DashboardCards";
import * as tabs from "../lib/tabsStore";
import { TabScope } from "../lib/tabScope";
import { defaultState } from "../lib/tabs";
import { resetView, setNamespaces } from "../lib/workspace";

const CTX: ClusterContext = {
  name: "prod-eu", stableId: "/kube/config#prod", key: "/kube/config#prod", cluster: "prod",
  server: "https://prod", isCurrent: true, sourceFile: "/kube/config", authKind: "token",
};

const card = (fields: Partial<ExtensionDashboardCard> & { id: string }): ExtensionDashboardCard => ({
  title: fields.id, size: "s", type: "count", source: "certificates", ...fields,
});

function app(cards: ExtensionDashboardCard[], extra: Partial<InstalledExtension> = {}): InstalledExtension {
  return {
    enabled: true, revision: 4, grants: [], settings: {}, source: "local", installedAt: 0, history: [],
    manifest: {
      id: "org.example.certs", name: "Certificates", version: "1.0.0", srelensApiVersion: "^0.4",
      kind: "declarative", permissions: [], capabilities: [],
      contributions: {
        pages: [{ id: "certificates", title: "Certificates", capability: "certificates" }],
        detailTabs: [], detailLinks: [], dashboardCards: cards,
      },
    },
    ...extra,
  } as InstalledExtension;
}

function installed(...plugins: InstalledExtension[]) {
  core.listExtensions.mockResolvedValue({ schemaVersion: 1, nextRevision: 9, plugins });
}

function answer(cards: ResolvedDashboardCard[]) {
  core.resolveDashboardCards.mockResolvedValue({ cards });
}

const cardRegion = (title: string) => screen.getByRole("region", { name: title });

beforeEach(() => {
  vi.clearAllMocks();
  resetView();
  tabs.setState(defaultState([]));
  namespaceOptions.useNamespaceOptions.mockReturnValue({ namespaces: ["prod", "team"], scope: "", error: "" });
  // A host that never answers an open: the cards stay as read, and say they are connecting.
  core.openExtensionView.mockImplementation((id: string) => ({ view: id, close: async () => {}, open: () => new Promise(() => {}) }));
});
afterEach(cleanup);

// A card's title and app name are app prose of up to 120 characters. Cut off
// with an ellipsis, the rest was reachable only by pointer hover, never by
// keyboard focus or touch. So they wrap, and a long unbroken word breaks
// rather than overflowing the tile. jsdom has no layout, so the contract is
// pinned in the stylesheet it lives in, as ExtensionResourceDetails does.
it("shows a card's whole title and app name rather than cutting them off", async () => {
  const { readFileSync } = await import("node:fs");
  const { join } = await import("node:path");
  const css = readFileSync(join(__dirname, "extensions.css"), "utf8");
  const rule = (selector: string) => {
    const at = css.indexOf(`${selector} {`);
    expect(at, `${selector} has a rule`).toBeGreaterThanOrEqual(0);
    return css.slice(at, css.indexOf("}", at));
  };
  for (const selector of [".dashboard-card-title", ".dashboard-card-app"]) {
    expect(rule(selector), selector).not.toMatch(/text-overflow|white-space\s*:\s*nowrap/);
    expect(rule(selector), selector).toMatch(/overflow-wrap\s*:\s*anywhere/);
  }
});

// What a reconnecting view shows is marked without fading it (#566): an
// opacity drops muted text below the contrast floor in every theme, and a bare
// `[data-stale]` rule would reach any element in the app that uses the name.
it("marks stale regions without fading their text, and only its own regions", async () => {
  const { readFileSync } = await import("node:fs");
  const { join } = await import("node:path");
  const css = readFileSync(join(__dirname, "extensions.css"), "utf8");
  const rules = [...css.replace(/\/\*[\s\S]*?\*\//g, "").matchAll(/([^{}]*\[data-stale\][^{}]*)\{([^}]*)\}/g)];
  expect(rules.length).toBeGreaterThan(0);
  for (const [, selector, body] of rules) {
    expect(body, selector).not.toMatch(/opacity|filter/);
    for (const part of selector.split(",")) expect(part.trim(), selector).toMatch(/^\.(extension|dashboard)-[\w-]+\[data-stale\]/);
  }
});

describe("DashboardCards", () => {
  it("draws loading, couldn't read and zero as three different things", async () => {
    installed(app([
      card({ id: "loading", title: "Loading card" }),
      card({ id: "failed", title: "Failed card" }),
      card({ id: "zero", title: "Zero card" }),
    ]));
    let finish!: (value: { cards: ResolvedDashboardCard[] }) => void;
    core.resolveDashboardCards.mockImplementationOnce(() => new Promise((done) => { finish = done; }));
    render(<DashboardCards context={CTX} />);

    // Loading: a status, and no figure at all.
    const loading = await screen.findByRole("region", { name: "Loading card" });
    expect((loading).getAttribute("data-state")).toBe("loading");
    expect((within(loading).getByRole("status")).textContent).toMatch(/Loading/);
    expect((loading).textContent).not.toMatch(/\d/);

    await act(async () => finish({ cards: [
      { id: "loading", state: "count", count: 7 },
      { id: "failed", state: "error", reason: 'applications.argoproj.io is forbidden: User "dev" cannot list resource' },
      { id: "zero", state: "count", count: 0 },
    ] }));

    // Couldn't read: the reason and a retry, and still no figure.
    const failed = cardRegion("Failed card");
    expect((failed).getAttribute("data-state")).toBe("error");
    expect((within(failed).getByRole("alert")).textContent).toMatch(/Couldn.t read/);
    expect((failed).textContent).toMatch(/forbidden/);
    expect(within(failed).getByRole("button", { name: /Retry/ })).toBeTruthy();
    expect((failed).textContent).not.toMatch(/\b0\b/);

    // Zero: a figure, said as an answer.
    const zero = cardRegion("Zero card");
    expect((zero).getAttribute("data-state")).toBe("zero");
    expect((zero).textContent).toMatch("0");
    expect((zero).textContent).toMatch(/None match/);
    expect(within(zero).queryByRole("alert")).toBeNull();

    expect((cardRegion("Loading card")).getAttribute("data-state")).toBe("value");
    expect((cardRegion("Loading card")).textContent).toMatch("7");
  });

  it("retries a failed read and shows the new answer", async () => {
    installed(app([card({ id: "c", title: "Expiring" })]));
    core.resolveDashboardCards
      .mockResolvedValueOnce({ cards: [{ id: "c", state: "error", reason: "connection refused" }] })
      .mockResolvedValueOnce({ cards: [{ id: "c", state: "count", count: 3 }] });
    render(<DashboardCards context={CTX} />);
    const retry = await screen.findByRole("button", { name: /Retry/ });
    await userEvent.click(retry);
    await waitFor(() => expect((cardRegion("Expiring")).textContent).toMatch("3"));
    expect(core.resolveDashboardCards).toHaveBeenCalledTimes(2);
  });

  it("says every card of an app could not be read when the whole call fails", async () => {
    installed(app([card({ id: "a", title: "First" }), card({ id: "b", title: "Second" })]));
    core.resolveDashboardCards.mockRejectedValue(new Error("Extension was disabled or updated; refresh the view"));
    render(<DashboardCards context={CTX} />);
    for (const title of ["First", "Second"]) {
      await waitFor(() => expect((cardRegion(title)).getAttribute("data-state")).toBe("error"));
      expect((cardRegion(title)).textContent).toMatch(/refresh the view/);
    }
  });

  it("says a status card with nothing to count is none, not a failure", async () => {
    installed(app([card({ id: "s", title: "By status", type: "countByStatus", size: "m" })]));
    answer([{ id: "s", state: "countByStatus", total: 0, statuses: [] }]);
    render(<DashboardCards context={CTX} />);
    await waitFor(() => expect((cardRegion("By status")).getAttribute("data-state")).toBe("zero"));
    const region = cardRegion("By status");
    expect(region.querySelector(".dashboard-card-figure")?.textContent).toBe("0");
    expect((region).textContent).toMatch(/None match/);
    expect(within(region).queryByRole("alert")).toBeNull();
  });

  it("draws each card type, and its size", async () => {
    installed(app([
      card({ id: "status", title: "Health", type: "countByStatus", size: "m" }),
      card({ id: "metric", title: "Critical CVEs", type: "metric", size: "s",
        metric: { jsonPath: ".report.critical", aggregate: "sum" } }),
      card({ id: "empty-min", title: "Oldest", type: "metric", size: "s",
        metric: { jsonPath: ".report.age", aggregate: "min" } }),
      card({ id: "list", title: "Soonest", type: "list", size: "l", list: { jsonPath: ".status.notAfter" } }),
    ]));
    answer([
      { id: "status", state: "countByStatus", total: 3, statuses: [{ status: "Degraded", count: 2 }, { status: "Healthy", count: 1 }] },
      { id: "metric", state: "metric", value: 12345, counted: 4 },
      { id: "empty-min", state: "metric", value: null, counted: 0 },
      { id: "list", state: "list", total: 12, rows: [
        { namespace: "team", name: "web", value: "2026-09-25T00:00:00Z" },
        { namespace: "prod", name: "shop" },
      ] },
    ]);
    render(<DashboardCards context={CTX} />);
    await waitFor(() => expect((cardRegion("Soonest")).getAttribute("data-state")).toBe("value"));
    expect((cardRegion("Health")).getAttribute("data-size")).toBe("m");
    expect((cardRegion("Health")).textContent).toMatch(/Degraded\s*2/);
    expect((cardRegion("Health")).textContent).toMatch(/Healthy\s*1/);
    // Formatted in the reader's locale, as every other figure in the app is.
    expect((cardRegion("Critical CVEs")).textContent).toContain((12345).toLocaleString());
    // A minimum of nothing is no value, not zero.
    expect((cardRegion("Oldest")).getAttribute("data-state")).toBe("zero");
    expect((cardRegion("Oldest")).textContent).toMatch(/No value/);
    expect((cardRegion("Oldest")).textContent).toMatch(/No matching resource has a number at \.report\.age/);
    expect((cardRegion("Oldest")).textContent).not.toMatch(/\b0\b/);
    const list = cardRegion("Soonest");
    expect((list).getAttribute("data-size")).toBe("l");
    expect(within(list).getAllByRole("listitem")).toHaveLength(2);
    expect((list).textContent).toMatch(/team\/web/);
    expect((list).textContent).toMatch(/2 of 12/);
  });

  it("follows the dashboard's namespace selection and keeps the cluster pinned", async () => {
    installed(app([card({ id: "c", title: "Expiring" })]));
    answer([{ id: "c", state: "count", count: 1 }]);
    render(<DashboardCards context={CTX} />);
    await waitFor(() => expect(core.resolveDashboardCards).toHaveBeenCalled());
    // The stable ID, never the display name: a renamed context must not move the read.
    expect(core.resolveDashboardCards).toHaveBeenLastCalledWith("org.example.certs", 4, CTX.key, []);
    act(() => setNamespaces(CTX.stableId, ["team"]));
    await waitFor(() =>
      expect(core.resolveDashboardCards).toHaveBeenLastCalledWith("org.example.certs", 4, CTX.key, ["team"]),
    );
    act(() => setNamespaces(CTX.stableId, ["team", "prod"]));
    await waitFor(() =>
      expect(core.resolveDashboardCards).toHaveBeenLastCalledWith("org.example.certs", 4, CTX.key, ["team", "prod"]),
    );
  });

  it("writes a pick to its own tab — the same cluster's cards in another tab keep their selection", async () => {
    tabs.openTab("/overview");
    tabs.openTab("/overview-copy");
    const idOf = (route: string) => tabs.currentWorkspace().tabs.find((t) => t.route === route)!.id;
    const [first, second] = [idOf("/overview"), idOf("/overview-copy")];
    installed(app([card({ id: "c", title: "Expiring" })]));
    answer([{ id: "c", state: "count", count: 1 }]);
    render(
      <>
        <div data-testid="first"><TabScope.Provider value={first}><DashboardCards context={CTX} /></TabScope.Provider></div>
        <div data-testid="second"><TabScope.Provider value={second}><DashboardCards context={CTX} /></TabScope.Provider></div>
      </>,
    );
    await userEvent.click(await within(screen.getByTestId("first")).findByRole("combobox", { name: "Namespaces" }));
    await userEvent.click(await screen.findByRole("button", { name: "Only team" }));

    const tab = (id: string) => tabs.currentWorkspace().tabs.find((t) => t.id === id)!;
    await waitFor(() => expect(tab(first).namespaces).toEqual({ [CTX.stableId]: ["team"] }));
    expect(tab(second).namespaces).toBeUndefined();
  });

  it("reads a namespace-restricted credential's one namespace whatever is selected", async () => {
    namespaceOptions.useNamespaceOptions.mockReturnValue({ namespaces: ["team"], scope: "team", error: "" });
    installed(app([card({ id: "c", title: "Expiring" })]));
    answer([{ id: "c", state: "count", count: 1 }]);
    render(<DashboardCards context={CTX} />);
    await waitFor(() =>
      expect(core.resolveDashboardCards).toHaveBeenLastCalledWith("org.example.certs", 4, CTX.key, ["team"]),
    );
  });

  it("stays loading, and reads nothing, until the namespace scope is known", async () => {
    // Still listing namespaces: a restricted credential's scope is not known yet,
    // and reading "every namespace" now would draw its 403 as "Couldn't read".
    namespaceOptions.useNamespaceOptions.mockReturnValue({ namespaces: null, scope: "", error: "" });
    installed(app([card({ id: "c", title: "Expiring" })]));
    answer([{ id: "c", state: "count", count: 2 }]);
    const view = render(<DashboardCards context={CTX} />);
    const region = await screen.findByRole("region", { name: "Expiring" });
    expect(region.getAttribute("data-state")).toBe("loading");
    await act(async () => {});
    expect(core.resolveDashboardCards).not.toHaveBeenCalled();
    // Nothing is followed yet either, and the band says why — not the web's reason.
    const status = screen.getByText("Not live").closest("[title]");
    expect(status?.getAttribute("title")).toMatch(/namespaces/);
    expect(core.openExtensionView).not.toHaveBeenCalled();
    namespaceOptions.useNamespaceOptions.mockReturnValue({ namespaces: ["team"], scope: "team", error: "" });
    view.rerender(<DashboardCards context={CTX} />);
    await waitFor(() => expect(cardRegion("Expiring").getAttribute("data-state")).toBe("value"));
    expect(core.resolveDashboardCards).toHaveBeenCalledTimes(1);
    expect(core.resolveDashboardCards).toHaveBeenLastCalledWith("org.example.certs", 4, CTX.key, ["team"]);
  });

  it("follows each card's reader and redraws its figure in place when it changes (#566)", async () => {
    const watched: Array<{ capability: string; namespace?: string; context: string; onData: (d: unknown, s: number) => void }> = [];
    core.openExtensionView.mockImplementation((id: string) => ({
      view: id, close: vi.fn(async () => {}),
      open: async (request: { context: string; namespace?: string; source: { capability: string } }, handlers: { onData: (d: unknown, s: number) => void }) => {
        watched.push({ capability: request.source.capability, namespace: request.namespace, context: request.context, onData: handlers.onData });
        return { stream: "s", cancel: vi.fn(async () => {}) };
      },
    }));
    setNamespaces(CTX.stableId, ["team"]);
    installed(app([card({ id: "c", title: "Expiring" }), card({ id: "d", title: "Issued" })]));
    core.resolveDashboardCards
      .mockResolvedValueOnce({ cards: [{ id: "c", state: "count", count: 1 }, { id: "d", state: "count", count: 0 }] })
      .mockResolvedValue({ cards: [{ id: "c", state: "count", count: 4 }, { id: "d", state: "count", count: 0 }] });
    render(<DashboardCards context={CTX} />);
    const figure = () => cardRegion("Expiring").querySelector(".dashboard-card-figure")?.textContent;
    await waitFor(() => expect(figure()).toBe("1"));
    // Two cards over one reader: one watch, in the selected namespace, on the pinned cluster.
    await waitFor(() => expect(watched.map((w) => [w.capability, w.namespace, w.context])).toEqual([["certificates", "team", CTX.key]]));
    act(() => watched[0].onData({ event: "synced" }, 1));
    expect(screen.getByText("Live")).toBeTruthy();
    act(() => watched[0].onData({ event: "changed" }, 2));
    await waitFor(() => expect(figure()).toBe("4"));
    act(() => watched[0].onData({ event: "reconnecting", message: "reset" }, 3));
    expect(screen.getByText(/The figures below may be out of date/)).toBeTruthy();
    expect(cardRegion("Expiring").closest("[data-stale]")).not.toBeNull();
  });

  it("refreshes on request", async () => {
    installed(app([card({ id: "c", title: "Expiring" })]));
    core.resolveDashboardCards
      .mockResolvedValueOnce({ cards: [{ id: "c", state: "count", count: 1 }] })
      .mockResolvedValueOnce({ cards: [{ id: "c", state: "count", count: 5 }] });
    render(<DashboardCards context={CTX} />);
    const figure = () => cardRegion("Expiring").querySelector(".dashboard-card-figure")?.textContent;
    await waitFor(() => expect(figure()).toBe("1"));
    await userEvent.click(screen.getByRole("button", { name: "Refresh app cards" }));
    // The new answer is drawn, not merely requested.
    await waitFor(() => expect(figure()).toBe("5"));
  });

  it("opens the target page on the pinned cluster with the card as its filter", async () => {
    const open = vi.spyOn(tabs, "openTab").mockImplementation(() => {});
    installed(app([card({ id: "expiring", title: "Expiring", target: { page: "certificates" } }), card({ id: "plain", title: "No target" })]));
    answer([{ id: "expiring", state: "count", count: 2 }, { id: "plain", state: "count", count: 1 }]);
    act(() => setNamespaces(CTX.stableId, ["team"]));
    render(<DashboardCards context={CTX} />);
    await userEvent.click(await screen.findByRole("button", { name: "Open Expiring" }));
    expect(open).toHaveBeenCalledWith(
      extensionCardRoute(CTX.key, "org.example.certs", "certificates", "team", "expiring"),
      { clusterName: CTX.name },
    );
    expect(within(cardRegion("No target")).queryByRole("button", { name: /Open/ })).toBeNull();
  });

  it("opens the target over exactly the namespaces the card counted in", async () => {
    const open = vi.spyOn(tabs, "openTab").mockImplementation(() => {});
    installed(app([card({ id: "expiring", title: "Expiring", target: { page: "certificates" } })]));
    answer([{ id: "expiring", state: "count", count: 2 }]);
    act(() => setNamespaces(CTX.stableId, ["team", "prod"]));
    render(<DashboardCards context={CTX} />);
    await userEvent.click(await screen.findByRole("button", { name: "Open Expiring" }));
    // Not every namespace: the page would show rows the card never counted.
    expect(open).toHaveBeenCalledWith(
      extensionCardRoute(CTX.key, "org.example.certs", "certificates", "", "expiring", ["team", "prod"]),
      { clusterName: CTX.name },
    );
  });

  it("opens the target across every namespace when nothing is selected", async () => {
    const open = vi.spyOn(tabs, "openTab").mockImplementation(() => {});
    installed(app([card({ id: "expiring", title: "Expiring", target: { page: "certificates" } })]));
    answer([{ id: "expiring", state: "count", count: 2 }]);
    render(<DashboardCards context={CTX} />);
    await userEvent.click(await screen.findByRole("button", { name: "Open Expiring" }));
    expect(open).toHaveBeenCalledWith(
      extensionCardRoute(CTX.key, "org.example.certs", "certificates", "", "expiring"),
      { clusterName: CTX.name },
    );
  });

  describe("on two contexts that share a stable ID (#695)", () => {
    // `/kube/a` declaring `b#c` and `/kube/a#b` declaring `c` share `/kube/a#b#c`.
    const first: ClusterContext = { ...CTX, name: "b#c", stableId: "/kube/a#b#c", key: "/kube/a#b%23c" };
    const second: ClusterContext = { ...CTX, name: "c", stableId: "/kube/a#b#c", key: "/kube/a%23b#c" };

    it("reads, follows and opens each one's cards by its own key", async () => {
      const open = vi.spyOn(tabs, "openTab").mockImplementation(() => {});
      const watched: string[] = [];
      core.openExtensionView.mockImplementation((id: string) => ({
        view: id, close: vi.fn(async () => {}),
        open: async (request: { context: string }) => {
          watched.push(request.context);
          return { stream: "s", cancel: vi.fn(async () => {}) };
        },
      }));
      installed(app([card({ id: "expiring", title: "Expiring", target: { page: "certificates" } })]));
      answer([{ id: "expiring", state: "count", count: 2 }]);
      for (const context of [first, second]) {
        const view = render(<DashboardCards context={context} />);
        await userEvent.click(await screen.findByRole("button", { name: "Open Expiring" }));
        expect(core.resolveDashboardCards).toHaveBeenLastCalledWith("org.example.certs", 4, context.key, []);
        await waitFor(() => expect(watched.at(-1)).toBe(context.key));
        expect(screen.queryByText(/two contexts share/)).toBeNull();
        view.unmount();
      }
      expect(open.mock.calls).toEqual([
        ["/extension-contexts/%2Fkube%2Fa%23b%2523c/org.example.certs/certificates/?card=expiring", { clusterName: "b#c" }],
        ["/extension-contexts/%2Fkube%2Fa%2523b%23c/org.example.certs/certificates/?card=expiring", { clusterName: "c" }],
      ]);
    });
  });

  it("draws app text as plain text, never markup or invisible reordering", async () => {
    const title = "<img src=x onerror=alert(1)> Exp‮iring";
    installed(app([card({ id: "c", title, type: "list" })], {
      manifest: { ...app([]).manifest, name: "Certs​<b>bold</b>", contributions: {
        ...app([]).manifest.contributions, dashboardCards: [card({ id: "c", title, type: "list" })],
      } },
    }));
    answer([{ id: "c", state: "list", total: 1, rows: [{ namespace: "team", name: "web", value: "a‮b<i>x</i>" }] }]);
    const { container } = render(<DashboardCards context={CTX} />);
    await waitFor(() => expect(container.querySelector("[data-state='value']")).not.toBeNull());
    expect(container.querySelector("img, b, i")).toBeNull();
    expect(container.textContent).not.toMatch(/[‮​]/);
    expect(container.textContent).toContain("Exp\\u202eiring");
    expect(container.textContent).toContain("<b>bold</b>");
  });

  it("shows no cards for a disabled app or one not enabled for this cluster", async () => {
    installed(
      app([card({ id: "a", title: "Disabled app card" })], { enabled: false }),
      app([card({ id: "b", title: "Other cluster card" })], { contexts: ["/kube/config#staging"] }),
    );
    const { container } = render(<DashboardCards context={CTX} />);
    await waitFor(() => expect(core.listExtensions).toHaveBeenCalled());
    await act(async () => {});
    expect(core.resolveDashboardCards).not.toHaveBeenCalled();
    expect((container).innerHTML).toBe("");
  });

  it("says the apps could not be listed rather than showing no cards", async () => {
    core.listExtensions.mockRejectedValue(new Error("extension inventory unreadable"));
    render(<DashboardCards context={CTX} />);
    const alert = await screen.findByRole("alert");
    expect((alert).textContent).toMatch(/App cards could not be listed/);
    expect((alert).textContent).toMatch(/unreadable/);
  });
});
