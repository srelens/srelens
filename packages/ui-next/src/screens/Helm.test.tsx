import { describe, it, expect, vi, beforeEach } from "vitest";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

/**
 * **Only the three capability wrappers are replaced.**
 *
 * `helmStatus` stays real, so every status assertion below is against core's
 * own verdict rather than a copy of it — that is the whole point of Task 2,
 * and a stubbed one would let a hand-paired table on the screen pass.
 * `describeError`, `ageFromTimestamp` and `plural` stay real for the same
 * reason: the failure copy, the ages and the counts are core's arithmetic.
 */
const core = vi.hoisted(() => ({
  listHelmReleases: vi.fn(),
  getHelmRelease: vi.fn(),
  startHelmOp: vi.fn(),
}));
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  ...core,
}));

if (!("ResizeObserver" in globalThis)) {
  (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}

import type { ClusterContext, HelmReleaseSummary } from "@srelens/core";
import { Helm } from "./Helm";
import { ConsoleProvider } from "../console";
import { resetContexts, setContexts } from "../lib/clusters";
import { __resetHelmOpsForTests } from "../lib/helmOps";
import { defaultState } from "../lib/tabs";
import * as store from "../lib/tabsStore";
import { resetView } from "../lib/workspace";

const ROUTE = "/helm";

const CTX: ClusterContext = {
  name: "prod-eu",
  stableId: "prod",
  cluster: "prod",
  server: "https://prod",
  isCurrent: true,
  namespace: "platform",
};

const DAY = 86_400_000;
const iso = (ago: number) => new Date(Date.now() - ago).toISOString();

/**
 * §16's releases, trimmed to four — and the four are chosen so that every
 * branch of core's verdict is on screen at once: `deployed` is the only
 * healthy word, `failed` the only broken one, `pending-upgrade` is a mutation
 * in flight, and `quiescing` is a word this build has never heard of.
 *
 * `quiescing` is not a Helm status. That is deliberate: Helm's set is Helm's
 * to extend, and a screen that kept its own label table would either render
 * nothing for it or invent a word. Core renders it verbatim, toned neutral.
 */
const INGRESS: HelmReleaseSummary = {
  name: "ingress-nginx",
  namespace: "platform",
  revision: 14,
  status: "deployed",
  chart: "ingress-nginx",
  chartVersion: "4.12.0",
  appVersion: "1.12.0",
  updated: iso(9 * DAY),
};

const CHECKOUT: HelmReleaseSummary = {
  name: "checkout",
  namespace: "checkout",
  revision: 119,
  status: "failed",
  chart: "acme-service",
  chartVersion: "2.4.1",
  appVersion: "4f2a1c",
  updated: iso(6 * 60_000),
};

const PAYMENTS: HelmReleaseSummary = {
  name: "payments",
  namespace: "payments",
  revision: 62,
  status: "pending-upgrade",
  chart: "acme-service",
  chartVersion: "2.4.1",
  appVersion: "9.1.3",
  updated: iso(5 * DAY),
};

const REDIS: HelmReleaseSummary = {
  name: "redis-session",
  namespace: "checkout",
  revision: 5,
  status: "quiescing",
  chart: "redis",
  chartVersion: "20.6.1",
  appVersion: "7.4.1",
  updated: iso(41 * DAY),
};

const RELEASES = [INGRESS, CHECKOUT, PAYMENTS, REDIS];

/** Two rendered manifests per release, so the pane has something to diff. */
const MANIFESTS: Record<string, Record<number, string>> = {
  checkout: {
    118: 'replicaCount: 12\nimage:\n  tag: "118a7e"',
    119: 'replicaCount: 12\nimage:\n  tag: "4f2a1c"',
    120: 'replicaCount: 12\nimage:\n  tag: "9de110"',
  },
  payments: { 61: "replicas: 1", 62: "replicas: 4" },
  "ingress-nginx": { 13: "replicas: 1", 14: "replicas: 2" },
  "redis-session": { 4: "replicas: 1", 5: "replicas: 2" },
};

beforeEach(() => {
  vi.clearAllMocks();
  core.listHelmReleases.mockResolvedValue({ releases: RELEASES });
  core.getHelmRelease.mockImplementation(
    async (_ctx: string, _ns: string, name: string, _invoke?: unknown, revision?: number) => {
      const manifest = MANIFESTS[name]?.[revision ?? -1];
      if (manifest === undefined) return { error: `no revision ${revision} of ${name}` };
      return { release: { name, manifest, history: [] } };
    },
  );
  core.startHelmOp.mockResolvedValue({ close: vi.fn() });
  __resetHelmOpsForTests();
  resetContexts();
  setContexts([CTX]);
  store.setState(defaultState([CTX]));
  resetView();
});

function open() {
  store.openTab(ROUTE);
  return render(
    <ConsoleProvider>
      <Helm route={ROUTE} />
    </ConsoleProvider>,
  );
}

const headers = () =>
  Array.from(document.querySelectorAll("thead th")).map((th) => th.textContent?.trim() ?? "");
/**
 * A row by its Release cell.
 *
 * By the first cell rather than by text: `checkout` and `payments` are each
 * both a release name and a namespace, so a whole-document text query finds
 * two nodes in the same row and a third in another.
 */
const rowFor = (name: string) =>
  Array.from(document.querySelectorAll("tbody tr")).find(
    (tr) => tr.querySelector("td")?.textContent?.trim() === name,
  ) as HTMLElement;

/** Select a release the way the reader does: a click on its row. */
const select = (name: string) =>
  userEvent.click(rowFor(name).querySelector("td") as HTMLElement);
const cells = (row: HTMLElement) =>
  Array.from(row.querySelectorAll("td")).map((td) => td.textContent?.trim() ?? "");
const cell = (row: HTMLElement, column: string) => cells(row)[headers().indexOf(column)];

/**
 * A field whose `Field` carries a hint.
 *
 * No word boundary: the kit renders the hint INSIDE the label, so the computed
 * accessible name runs the two together.
 */
const hintedField = (label: string) => screen.getByLabelText(new RegExp(`^${label}`)) as HTMLElement;

/** The status word and the severity the pill drew it at. */
const verdict = (row: HTMLElement) => {
  const pill = row.querySelector(".status");
  return { word: pill?.textContent?.trim() ?? "", kind: pill?.getAttribute("data-kind") ?? "" };
};

/** Every pane head on screen, in document order: the table's, then §16's rail. */
const heads = () =>
  Array.from(document.querySelectorAll(".pane-head")).map((h) => h.textContent?.trim() ?? "");

/** The 420px rail's own head — the second one. */
const railHead = () => heads()[1] ?? "";

async function ready() {
  await screen.findByText("ingress-nginx");
}

describe("Helm — the header", () => {
  it("titles the screen and names the cluster it is listing", async () => {
    open();
    await ready();
    expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("Helm");
    expect(screen.getByText("prod-eu / releases")).toBeTruthy();
  });

  /**
   * #352's defect, pointed at this screen. `.row-ask` is `opacity: 0` until a
   * `.tbl tbody tr` is hovered and a header has no row to hover, so an
   * `AskChip` here would ship an invisible control — which Logs did.
   */
  it("asks with a Button rather than the row chip, which a header cannot reveal", async () => {
    const { container } = open();
    await ready();
    const explain = screen.getByRole("button", { name: /^Explain: / });
    expect(explain.className).not.toContain("row-ask");
    expect(container.querySelector(".row-ask")).toBeNull();
  });

  it("asks about the release the reader is looking at", async () => {
    open();
    await ready();
    // Nothing selected: the question is about the cluster, because there is no
    // release to name.
    expect(screen.getByRole("button", { name: /^Explain: / }).getAttribute("aria-label")).toBe(
      "Explain: Which Helm releases on prod-eu need attention?",
    );

    await select("checkout");
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /^Explain: / }).getAttribute("aria-label")).toBe(
        "Explain: What did checkout release 119 change?",
      ),
    );
  });

  it("opens the install dialog on a release name, never an empty one", async () => {
    open();
    await ready();
    await userEvent.click(screen.getByRole("button", { name: "Install chart" }));

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("Install new-release")).toBeTruthy();
    // The gate that proves the name is a prop and not a field: an empty
    // release can never open it.
    expect(within(dialog).getByLabelText("Chart")).toBeTruthy();
  });
});

describe("Helm — the release table", () => {
  it("heads the pane with how many releases this cluster has", async () => {
    open();
    await ready();
    expect(heads()[0]).toBe("Releases · 4 in this cluster");
  });

  it("renders §16's columns in §16's order", async () => {
    open();
    await ready();
    expect(headers().slice(0, 6)).toEqual([
      "Release",
      "Namespace",
      "Chart",
      "Rev",
      "Status",
      "Updated",
    ]);
  });

  it("renders a row per release, with its chart, revision and age", async () => {
    open();
    await ready();
    const row = rowFor("ingress-nginx");
    expect(cell(row, "Namespace")).toBe("platform");
    expect(cell(row, "Chart")).toBe("ingress-nginx-4.12.0");
    expect(cell(row, "Rev")).toBe("14");
    expect(cell(row, "Updated")).toBe("9d ago");
  });

  /**
   * The load-bearing one. Every word and tone here is core's `helmStatus`, and
   * the unknown status is what makes a hand-paired table on this screen fail
   * rather than pass — it has no entry for `quiescing` to have.
   */
  it("takes every status word and tone from core's verdict", async () => {
    open();
    await ready();
    expect(verdict(rowFor("ingress-nginx"))).toEqual({ word: "deployed", kind: "success" });
    expect(verdict(rowFor("checkout"))).toEqual({ word: "failed", kind: "danger" });
    expect(verdict(rowFor("payments"))).toEqual({ word: "pending-upgrade", kind: "warning" });
    // Helm's set is Helm's to extend: the word Helm gave, toned neutral.
    expect(verdict(rowFor("redis-session"))).toEqual({ word: "quiescing", kind: "neutral" });
  });

  /**
   * `min-width: auto` has shipped seven times on this migration and jsdom sees
   * none of them, so the classes are asserted instead. A 420px pane beside a
   * table whose chart column is unbounded is exactly the setup that pushes the
   * pane off the window.
   */
  it("lets the table shrink instead of pushing the 420px pane off the window", async () => {
    const { container } = open();
    await ready();
    const main = container.querySelector('[data-slot="release-main"]');
    expect(main?.className).toContain("min-w-0");
    const chart = container.querySelector('[data-slot="chart-name"]');
    expect(chart?.className).toContain("truncate");
    expect(chart?.className).toContain("max-w-[200px]");
  });

  it("offers §16's three operations on every row, with the release named", async () => {
    open();
    await ready();
    const row = rowFor("checkout");
    expect(within(row).getByRole("button", { name: "Upgrade checkout" })).toBeTruthy();
    expect(within(row).getByRole("button", { name: "Roll back checkout" })).toBeTruthy();
    expect(within(row).getByRole("button", { name: "Uninstall checkout" })).toBeTruthy();
  });
});

describe("Helm — the operations", () => {
  it("opens the dialog in upgrade mode, on the row's own chart", async () => {
    open();
    await ready();
    await userEvent.click(screen.getByRole("button", { name: "Upgrade checkout" }));

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("Upgrade checkout")).toBeTruthy();
    expect((within(dialog).getByLabelText("Chart") as HTMLInputElement).value).toBe("acme-service");
    expect((within(dialog).getByLabelText("Chart version") as HTMLInputElement).value).toBe("2.4.1");
  });

  it("opens the dialog in rollback mode", async () => {
    open();
    await ready();
    await userEvent.click(screen.getByRole("button", { name: "Roll back checkout" }));

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("Roll back checkout")).toBeTruthy();
    expect(hintedField("Target revision")).toBeTruthy();
  });

  it("opens the dialog in uninstall mode, behind the typed gate", async () => {
    open();
    await ready();
    await userEvent.click(screen.getByRole("button", { name: "Uninstall checkout" }));

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("Uninstall checkout")).toBeTruthy();
    // The gate exists only because `release` is a non-empty prop.
    expect(within(dialog).getByLabelText("Type checkout to confirm")).toBeTruthy();
  });

  /**
   * §16's footer names the diff's left-hand revision — 118, one before what is
   * running. The dialog's own default target is `lastGoodRevision`, a
   * different question with a different answer, and the two are NOT
   * reconciled: the pane says which revision it is showing you, the dialog
   * says which one it is safe to return to.
   */
  it("rolls back from the pane's footer without overriding the dialog's own target", async () => {
    open();
    await ready();
    await select("checkout");
    await waitFor(() => expect(railHead()).toContain("118 → 119"));

    await userEvent.click(screen.getByRole("button", { name: "Roll back to 118" }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("Roll back checkout")).toBeTruthy();
    // The screen passes no history, so the dialog has no revision it can
    // vouch for and says so rather than filling in 118 on the pane's say-so.
    expect((hintedField("Target revision") as HTMLInputElement).value).toBe("");
  });

  it("opens the values editor as an upgrade of the selected release", async () => {
    open();
    await ready();
    await select("payments");
    await waitFor(() => expect(railHead()).toContain("payments"));

    await userEvent.click(screen.getByRole("button", { name: "Values editor" }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("Upgrade payments")).toBeTruthy();
  });
});

describe("Helm — the pane", () => {
  it("starts with nothing selected and says so", async () => {
    open();
    await ready();
    expect(screen.getByText("No release selected")).toBeTruthy();
  });

  /**
   * The other load-bearing one: the pane follows the SELECTED row. A pane
   * hard-wired to the first release — which is what §16's own mock does —
   * passes every other assertion in this file.
   */
  it("follows the selected row rather than the first one", async () => {
    open();
    await ready();

    await select("payments");
    await waitFor(() => expect(railHead()).toContain("payments · 61 → 62"));
    expect(railHead()).not.toContain("ingress-nginx");

    await select("checkout");
    await waitFor(() => expect(railHead()).toContain("checkout · 118 → 119"));
    expect(railHead()).not.toContain("payments");
  });

  it("tones the pane's badge with core's verdict for the selected release", async () => {
    open();
    await ready();
    await select("redis-session");
    await waitFor(() => expect(railHead()).toContain("redis-session"));
    expect(railHead()).toContain("quiescing");
  });

  /**
   * Requirement 1 from the previous tasks: without a refresh, a successful
   * upgrade leaves the pane diffing 118 → 119 — the pair from BEFORE the
   * upgrade — on a release that is now at 120.
   */
  it("refreshes the revision when an operation finishes, so the diff moves on", async () => {
    let exit: ((reason: unknown) => void) | undefined;
    core.startHelmOp.mockImplementation(
      async (
        _ctx: string,
        _args: string[],
        _onData: (line: string) => void,
        onExit: (reason: unknown) => void,
      ) => {
        exit = onExit;
        return { close: vi.fn() };
      },
    );

    open();
    await ready();
    await select("checkout");
    await waitFor(() => expect(railHead()).toContain("118 → 119"));

    await userEvent.click(screen.getByRole("button", { name: "Upgrade checkout" }));
    await screen.findByRole("dialog");
    await userEvent.click(screen.getByRole("button", { name: "Upgrade" }));
    await waitFor(() => expect(core.startHelmOp).toHaveBeenCalled());

    // The upgrade lands: helm moved the release to 120.
    core.listHelmReleases.mockResolvedValue({
      releases: RELEASES.map((r) => (r.name === "checkout" ? { ...r, revision: 120 } : r)),
    });
    await act(async () => {
      exit?.(null);
    });

    await waitFor(() => expect(railHead()).toContain("119 → 120"));
  });
});

describe("Helm — the three states §16 leaves out", () => {
  it("says it is listing while the releases are on their way", async () => {
    let release: (v: { releases: HelmReleaseSummary[] }) => void = () => {};
    core.listHelmReleases.mockReturnValue(
      new Promise<{ releases: HelmReleaseSummary[] }>((resolve) => {
        release = resolve;
      }),
    );
    open();

    expect(screen.getByRole("status", { name: "Listing Helm releases" })).toBeTruthy();
    expect(screen.queryByText("ingress-nginx")).toBeNull();

    await act(async () => {
      release({ releases: RELEASES });
    });
    expect(await screen.findByText("ingress-nginx")).toBeTruthy();
  });

  /** A cluster with no releases is ordinary, not a failure. */
  it("treats a cluster with no releases as ordinary", async () => {
    core.listHelmReleases.mockResolvedValue({ releases: [] });
    open();

    expect(await screen.findByText("No Helm releases")).toBeTruthy();
    expect(screen.getByText("prod-eu has no Helm releases.")).toBeTruthy();
    expect(heads()[0]).toBe("Releases · 0 in this cluster");
    // Nothing here says anything failed.
    expect(screen.queryByText(/Could not list/)).toBeNull();
  });

  it("says why the listing failed, in core's words rather than the backend's", async () => {
    core.listHelmReleases.mockResolvedValue({
      error: "ApiError: Unauthorized (Status { code: 401 })",
    });
    open();

    expect(await screen.findByText("Could not list Helm releases on prod-eu")).toBeTruthy();
    // `describeError`'s classification, not the raw Rust string.
    expect(screen.getByText(/rejected your credentials/i)).toBeTruthy();
    expect(screen.queryByText("ingress-nginx")).toBeNull();
  });

  it("offers a retry that lists again", async () => {
    core.listHelmReleases.mockResolvedValue({ error: "boom" });
    open();
    await screen.findByText("Could not list Helm releases on prod-eu");

    core.listHelmReleases.mockResolvedValue({ releases: RELEASES });
    await userEvent.click(screen.getByRole("button", { name: /retry|try again/i }));
    expect(await screen.findByText("ingress-nginx")).toBeTruthy();
  });

  it("has nothing to list without a cluster in focus", () => {
    resetContexts();
    store.setState(defaultState([]));
    open();
    expect(screen.getByText("No cluster in focus")).toBeTruthy();
    expect(core.listHelmReleases).not.toHaveBeenCalled();
  });
});
