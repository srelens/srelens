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

/**
 * The namespace options, doubled the way `Workloads.test.tsx` and
 * `Resources.test.tsx` double them: this screen must ask the SHARED hook for
 * them rather than growing a Helm-local list, and a stub is what makes the
 * loading, forbidden and failed branches reachable from a test at all.
 */
const { useNamespaceOptions } = vi.hoisted(() => ({ useNamespaceOptions: vi.fn() }));
vi.mock("@srelens/core/react", async (orig) => ({
  ...(await orig<typeof import("@srelens/core/react")>()),
  useNamespaceOptions: (...a: unknown[]) => useNamespaceOptions(...a),
}));

if (!("ResizeObserver" in globalThis)) {
  (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}
// The picker's popover keeps its highlighted row in view, and jsdom has no
// such method — the same stand-in `Workloads.test.tsx` installs.
const proto = window.HTMLElement.prototype as unknown as Record<string, unknown>;
proto.scrollIntoView ??= () => {};

import type { ClusterContext, HelmReleaseSummary, HelmRevision } from "@srelens/core";
import { Helm } from "./Helm";
import { ConsoleProvider } from "../console";
import { resetContexts, setContexts, setKubeconfigFiles } from "../lib/clusters";
import { __resetHelmOpsForTests, startHelmOperation } from "../lib/helmOps";
import { defaultState } from "../lib/tabs";
import * as store from "../lib/tabsStore";
import { getView, resetView, setNamespaces } from "../lib/workspace";

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

/**
 * A SECOND release called `checkout`, in another namespace.
 *
 * Helm scopes a release name to a namespace, so `checkout/checkout` and
 * `staging/checkout` are two different releases that happen to share a name —
 * the ordinary shape of a cluster running staging beside production. It is
 * here because the other four share namespaces but no names, which leaves the
 * load-bearing half of the row key unexercised: with a name-only key, clicking
 * this row would diff PRODUCTION's release and the pane's `Roll back` would
 * then operate on the row the lookup found rather than the row the reader
 * clicked. On a screen with an Uninstall button that is the worst failure it
 * has.
 */
const STAGING_CHECKOUT: HelmReleaseSummary = {
  name: "checkout",
  namespace: "staging",
  revision: 7,
  status: "deployed",
  chart: "acme-service",
  chartVersion: "2.3.0",
  appVersion: "0a91bb",
  updated: iso(2 * DAY),
};

const RELEASES = [INGRESS, CHECKOUT, PAYMENTS, REDIS, STAGING_CHECKOUT];

/** The namespaces the cluster has, as the shared hook answers with them. */
const NAMESPACES = ["checkout", "payments", "platform", "staging"];

/**
 * The kubeconfigs this window was started with.
 *
 * Two of them, and neither is `~/.kube/config`: a machine whose contexts came
 * from srelens-imported files or from `KUBECONFIG` is exactly the machine on
 * which an operation started without these fails to build a client at all.
 */
const FILES = ["/home/u/.config/srelens/imported.yaml", "/home/u/work/kubeconfig.yaml"];

/**
 * Two rendered manifests per release, keyed the way helm scopes a release:
 * `<namespace>/<name>`. Production's `checkout` and staging's have different
 * revisions AND different manifests, so a pane showing the wrong one cannot
 * look right by accident.
 */
const MANIFESTS: Record<string, Record<number, string>> = {
  "checkout/checkout": {
    118: 'replicaCount: 12\nimage:\n  tag: "118a7e"',
    119: 'replicaCount: 12\nimage:\n  tag: "4f2a1c"',
    120: 'replicaCount: 12\nimage:\n  tag: "9de110"',
  },
  "payments/payments": { 59: "replicas: 1", 60: "replicas: 2", 61: "replicas: 3", 62: "replicas: 4" },
  "platform/ingress-nginx": { 13: "replicas: 1", 14: "replicas: 2" },
  "checkout/redis-session": { 4: "replicas: 1", 5: "replicas: 2" },
  "staging/checkout": { 6: "replicaCount: 1", 7: "replicaCount: 2" },
};

/**
 * The values each release was actually installed with — what `helm get values`
 * answers, and what an upgrade must not throw away.
 *
 * `checkout` carries a body no chart default would produce, so a dialog that
 * opened on the chart's defaults instead cannot look right by accident.
 */
const VALUES: Record<string, string> = {
  "checkout/checkout": "replicaCount: 12\nimage:\n  tag: \"4f2a1c\"\nresources:\n  limits:\n    cpu: 500m",
  "payments/payments": "replicas: 4",
};

/**
 * What helm reports about each release's revisions — what `lastGoodRevision`
 * reads, and the only thing that stops the rollback dialog opening blank.
 *
 * `checkout` is the ordinary shape: the revision before the current one is
 * `superseded`, so the newest revision helm does not report failed IS the one
 * §16's footer names. `payments` is the shape where the two diverge: its
 * revision 61 failed, so the footer names 61 and the dialog offers 60.
 */
const HISTORIES: Record<string, HelmRevision[]> = {
  "checkout/checkout": [
    { revision: 117, status: "superseded", updated: iso(3 * DAY), chartVersion: "2.3.9", description: "Upgrade complete" },
    { revision: 118, status: "superseded", updated: iso(1 * DAY), chartVersion: "2.4.0", description: "Upgrade complete" },
    { revision: 119, status: "failed", updated: iso(6 * 60_000), chartVersion: "2.4.1", description: "Upgrade failed" },
  ],
  "payments/payments": [
    { revision: 60, status: "superseded", updated: iso(9 * DAY), chartVersion: "2.3.8", description: "Upgrade complete" },
    { revision: 61, status: "failed", updated: iso(7 * DAY), chartVersion: "2.4.0", description: "Upgrade failed" },
    { revision: 62, status: "pending-upgrade", updated: iso(5 * DAY), chartVersion: "2.4.1", description: "Upgrade in progress" },
  ],
};

beforeEach(() => {
  vi.clearAllMocks();
  // **The backend honours the namespace it is given**, the way the real one
  // does. A screen that fetched every namespace and threw most of it away
  // would still draw the right rows against a stub that ignored the argument,
  // which is precisely the escape this implementation has to be denied: on a
  // cluster with 383 releases, listing all of them to draw six is the common
  // case rather than the edge one.
  core.listHelmReleases.mockImplementation(
    async (_ctx: string, namespace?: string | null) => ({
      releases: namespace ? RELEASES.filter((r) => r.namespace === namespace) : RELEASES,
    }),
  );
  core.getHelmRelease.mockImplementation(
    async (_ctx: string, ns: string, name: string, _invoke?: unknown, revision?: number) => {
      const key = `${ns}/${name}`;
      const history = HISTORIES[key] ?? [];
      // No revision named: the screen asking what this release's revisions are,
      // which is what a rollback dialog opens on.
      if (revision === undefined) {
        return { release: { name, manifest: "", history, valuesYaml: VALUES[key] ?? "" } };
      }
      const manifest = MANIFESTS[key]?.[revision];
      if (manifest === undefined) return { error: `no revision ${revision} of ${name}` };
      return { release: { name, manifest, history } };
    },
  );
  core.startHelmOp.mockResolvedValue({ close: vi.fn() });
  useNamespaceOptions.mockReturnValue({ namespaces: NAMESPACES, scope: "", error: "" });
  __resetHelmOpsForTests();
  resetContexts();
  setContexts([CTX]);
  // After `resetContexts`, which clears them.
  setKubeconfigFiles(FILES);
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
const rowFor = (name: string, namespace?: string) =>
  Array.from(document.querySelectorAll("tbody tr")).find((tr) => {
    const td = Array.from(tr.querySelectorAll("td")).map((c) => c.textContent?.trim() ?? "");
    return td[0] === name && (namespace === undefined || td[1] === namespace);
  }) as HTMLElement;

/** Select a release the way the reader does: a click on its row. */
const select = (name: string, namespace?: string) =>
  userEvent.click(rowFor(name, namespace).querySelector("td") as HTMLElement);
const cells = (row: HTMLElement) =>
  Array.from(row.querySelectorAll("td")).map((td) => td.textContent?.trim() ?? "");
const cell = (row: HTMLElement, column: string) => cells(row)[headers().indexOf(column)];

/**
 * Every release on screen as `<namespace>/<name>` — helm's own identity for a
 * release, and the only way to tell production's `checkout` from staging's.
 */
const drawn = () =>
  Array.from(document.querySelectorAll("tbody tr.tbl-row")).map((tr) => {
    const td = Array.from(tr.querySelectorAll("td")).map((c) => c.textContent?.trim() ?? "");
    return `${td[1]}/${td[0]}`;
  });

/** The namespace picker's trigger. */
const picker = () => screen.getByRole("combobox", { name: "Namespaces" });

/** Narrow to one namespace the way the reader does: the row's own `only`. */
async function pickOnly(namespace: string) {
  await userEvent.click(picker());
  await userEvent.click(await screen.findByRole("button", { name: `Only ${namespace}` }));
}

/** The namespaces the last listing was scoped to — `null` for every namespace. */
const listedNamespaces = () =>
  core.listHelmReleases.mock.calls.map((call) => (call[1] as string | null) ?? null);

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

/**
 * Arm the backend so the next helm operation can be ended on demand.
 *
 * `fire` settles it: `null` is a clean exit, anything else is the reason it
 * failed.
 */
function armOperation(): { fire: (reason: unknown) => Promise<void> } {
  let onExit: ((reason: unknown) => void) | undefined;
  core.startHelmOp.mockImplementation(
    async (
      _ctx: string,
      _args: string[],
      _onData: (line: string) => void,
      exit: (reason: unknown) => void,
    ) => {
      onExit = exit;
      return { close: vi.fn() };
    },
  );
  return {
    fire: async (reason: unknown) => {
      await act(async () => {
        onExit?.(reason);
      });
    },
  };
}

/**
 * Run an operation to completion through the store rather than through the
 * screen — an upgrade started somewhere else, or on another cluster.
 */
async function settleElsewhere(context: string) {
  const armed = armOperation();
  await startHelmOperation({
    kind: "upgrade",
    release: "checkout",
    namespace: "checkout",
    context,
    args: ["upgrade", "checkout"],
  });
  await armed.fire(null);
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

    await select("checkout", "checkout");
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /^Explain: / }).getAttribute("aria-label")).toBe(
        "Explain: What did checkout release 119 change?",
      ),
    );
  });

  /**
   * **The name is the reader's, not a fixture's.** This screen used to open
   * the install dialog on `new-release` — the design's own placeholder — and
   * the dialog had no field to change it with. Every install was therefore
   * called `new-release`, and the second one helm refused outright: "cannot
   * re-use a name that is still in use". The same defect as §A.5's
   * "Twelve pods" alert, which this branch caught and this did not.
   */
  it("opens the install dialog on an empty name field, not on the mock's fixture", async () => {
    open();
    await ready();
    await userEvent.click(screen.getByRole("button", { name: "Install chart" }));

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).queryByText("Install new-release")).toBeNull();
    expect((within(dialog).getByLabelText("Release name") as HTMLInputElement).value).toBe("");
    // The context's own namespace, prefilled and editable.
    expect((within(dialog).getByLabelText("Namespace") as HTMLInputElement).value).toBe("platform");
    // Nothing to install until the reader says what to call it.
    expect((within(dialog).getByRole("button", { name: "Install" }) as HTMLButtonElement).disabled).toBe(
      true,
    );

    await userEvent.type(within(dialog).getByLabelText("Release name"), "checkout-api");
    await userEvent.type(within(dialog).getByLabelText("Chart"), "bitnami/nginx");
    expect(document.querySelector(".copy-command-text")?.textContent).toBe(
      "helm install checkout-api bitnami/nginx --namespace platform --create-namespace",
    );
  });
});

describe("Helm — the release table", () => {
  it("heads the pane with how many releases this cluster has", async () => {
    open();
    await ready();
    expect(heads()[0]).toBe("Releases · 5 in this cluster");
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
    expect(verdict(rowFor("checkout", "checkout"))).toEqual({ word: "failed", kind: "danger" });
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

  /**
   * **The eighth `min-width: auto`-family defect, seen on a real cluster.**
   *
   * A screenshot of `K8SM01-ADMIN` shows every row ending in a clipped `U`
   * where the 420px pane begins: `Uninstall` — the one destructive control on
   * this screen — reduced to a sliver of a letter, and mistakable for part of
   * `Roll back`'s group. Release names there are `cnips-abena-…` long and the
   * namespaces `m01-cnips-01-dataservices` long, so the two text columns take
   * the width and the action group is what falls off the end.
   *
   * The two text columns absorb the loss instead: they are the ones a reader
   * can recover by widening the column or reading the pane, and a truncated
   * name is not a control they cannot press. The group itself refuses to
   * shrink at all, so nothing inside the cell can be clipped either.
   */
  it("caps the wide text columns rather than letting the action group be clipped", async () => {
    const { container } = open();
    await ready();
    const release = container.querySelector('[data-slot="release-name"]');
    expect(release?.className).toContain("truncate");
    expect(release?.className).toMatch(/max-w-\[\d+px\]/);
    const namespace = container.querySelector('[data-slot="release-namespace"]');
    expect(namespace?.className).toContain("truncate");
    expect(namespace?.className).toMatch(/max-w-\[\d+px\]/);
    // `flex` items shrink by default, and these three carry the only
    // irreversible action on the screen.
    const actions = container.querySelector('[data-slot="release-actions"]');
    expect(actions?.className).toContain("shrink-0");
    expect(actions?.className).toContain("whitespace-nowrap");
  });

  it("offers §16's three operations on every row, with the release named", async () => {
    open();
    await ready();
    const row = rowFor("checkout", "checkout");
    expect(within(row).getByRole("button", { name: "Upgrade checkout" })).toBeTruthy();
    expect(within(row).getByRole("button", { name: "Roll back checkout" })).toBeTruthy();
    expect(within(row).getByRole("button", { name: "Uninstall checkout" })).toBeTruthy();
  });
});

describe("Helm — the operations", () => {
  it("opens the dialog in upgrade mode, on the row's own chart", async () => {
    open();
    await ready();
    await userEvent.click(within(rowFor("checkout", "checkout")).getByRole("button", { name: "Upgrade checkout" }));

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("Upgrade checkout")).toBeTruthy();
    expect((within(dialog).getByLabelText("Chart") as HTMLInputElement).value).toBe("acme-service");
    expect((within(dialog).getByLabelText("Chart version") as HTMLInputElement).value).toBe("2.4.1");
  });

  it("opens the dialog in rollback mode", async () => {
    open();
    await ready();
    await userEvent.click(within(rowFor("checkout", "checkout")).getByRole("button", { name: "Roll back checkout" }));

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("Roll back checkout")).toBeTruthy();
    expect(hintedField("Target revision")).toBeTruthy();
  });

  it("opens the dialog in uninstall mode, behind the typed gate", async () => {
    open();
    await ready();
    await userEvent.click(within(rowFor("checkout", "checkout")).getByRole("button", { name: "Uninstall checkout" }));

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("Uninstall checkout")).toBeTruthy();
    // The gate exists only because `release` is a non-empty prop.
    expect(within(dialog).getByLabelText("Type checkout to confirm")).toBeTruthy();
  });

  /**
   * A control must honour its own label. §16's footer reads `Roll back to 118`,
   * and the dialog behind it has to open on 118 — not on a blank field asking
   * the reader to type back the number they just clicked.
   */
  it("opens the pane's `Roll back to 118` on 118", async () => {
    open();
    await ready();
    await select("checkout", "checkout");
    await waitFor(() => expect(railHead()).toContain("118 → 119"));

    await userEvent.click(screen.getByRole("button", { name: "Roll back to 118" }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("Roll back checkout")).toBeTruthy();
    expect((hintedField("Target revision") as HTMLInputElement).value).toBe("118");
  });

  /**
   * The row action asked for no particular revision, so the dialog's own
   * default stands — which on this release is the same number, because helm
   * reports 118 superseded and `lastGoodRevision` picks the newest revision it
   * does not report failed.
   */
  it("opens the row's `Roll back` on the dialog's own default", async () => {
    open();
    await ready();
    await userEvent.click(
      within(rowFor("checkout", "checkout")).getByRole("button", { name: "Roll back checkout" }),
    );

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("Roll back checkout")).toBeTruthy();
    expect((hintedField("Target revision") as HTMLInputElement).value).toBe("118");
  });

  /**
   * **And the two numbers are still not reconciled.**
   *
   * `payments` is at 62 and its revision 61 FAILED. The pane names 61 —
   * "here is the revision the diff on screen is comparing against" — and the
   * dialog offers 60 — "here is the newest revision that is safe to return
   * to". Different questions, different answers, and nothing in this screen
   * makes either say the other's number.
   */
  it("lets the dialog offer an older revision when the one the pane names failed", async () => {
    open();
    await ready();
    await select("payments");
    await waitFor(() => expect(railHead()).toContain("61 → 62"));

    const footer = screen.getByRole("button", { name: "Roll back to 61" });
    await userEvent.click(footer);
    const dialog = await screen.findByRole("dialog");
    expect((hintedField("Target revision") as HTMLInputElement).value).toBe("60");
    // The hint says which revision it chose and what helm reports about it —
    // core's word, not this screen's.
    expect(within(dialog).getByText(/Revision 60 .*it reads superseded/)).toBeTruthy();
  });

  /** A refused history is the dialog's own degrade, not a broken dialog. */
  it("still opens the rollback dialog when the revisions cannot be read", async () => {
    core.getHelmRelease.mockResolvedValue({ error: "boom" });
    open();
    await ready();
    await userEvent.click(
      within(rowFor("checkout", "checkout")).getByRole("button", { name: "Roll back checkout" }),
    );

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("Roll back checkout")).toBeTruthy();
    expect((hintedField("Target revision") as HTMLInputElement).value).toBe("");
  });

  /**
   * **The defect.** `helm upgrade <rel> <chart>` with neither `-f` nor
   * `--reuse-values` applies the chart's DEFAULTS: every value the release was
   * installed with is discarded. A dialog opened on an empty editor is that
   * command, one click away, with nothing on screen saying so.
   */
  it("opens the upgrade dialog on the values the release is actually running", async () => {
    open();
    await ready();
    await userEvent.click(
      within(rowFor("checkout", "checkout")).getByRole("button", { name: "Upgrade checkout" }),
    );

    await screen.findByRole("dialog");
    await waitFor(() =>
      expect(document.querySelector(".cm-content")?.textContent).toContain("replicaCount: 12"),
    );
    expect(document.querySelector(".cm-content")?.textContent).toContain("cpu: 500m");
  });

  /**
   * The degrade, and the whole point of the fix: a refused `helm get values`
   * must NOT fall through to an empty editor, because an empty editor is the
   * defect. `--reuse-values` keeps what is on the release, and the reader is
   * told why the box is empty.
   */
  it("tells helm to reuse the release's values when srelens cannot read them", async () => {
    core.getHelmRelease.mockResolvedValue({
      error: "ApiError: Unauthorized (Status { code: 401 })",
    });
    open();
    await ready();
    await userEvent.click(
      within(rowFor("checkout", "checkout")).getByRole("button", { name: "Upgrade checkout" }),
    );

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(/could not read/i)).toBeTruthy();
    // core's classification, not the raw Rust string.
    expect(within(dialog).getByText(/rejected your credentials/i)).toBeTruthy();
    expect(document.querySelector(".copy-command-text")?.textContent).toContain("--reuse-values");
  });

  /**
   * **A verified escape: deleting `extraKubeconfigs` from this screen left all
   * 31 tests passing.**
   *
   * The store's own suite proves the store forwards them to core; nothing
   * proved the SCREEN supplied them. On a machine whose contexts come from
   * srelens-imported files or from `KUBECONFIG` rather than `~/.kube/config`,
   * every operation started here would fail to build a client — and every
   * assertion in this file would still be green.
   *
   * The argv is pinned in the same breath, and against the command the dialog
   * SHOWED the reader rather than against a copy of the dialog's own format:
   * a copy button that prints one command while srelens runs another is the
   * defect worth catching, and this cannot pass unless the two agree.
   */
  it("runs the argv it showed the reader, with this window's kubeconfigs", async () => {
    open();
    await ready();
    await userEvent.click(
      within(rowFor("checkout", "checkout")).getByRole("button", { name: "Uninstall checkout" }),
    );

    const dialog = await screen.findByRole("dialog");
    await userEvent.type(within(dialog).getByLabelText("Type checkout to confirm"), "checkout");
    const shown = document.querySelector(".copy-command-text")?.textContent ?? "";
    expect(shown).toContain("uninstall checkout");

    await userEvent.click(within(dialog).getByRole("button", { name: "Uninstall" }));
    await waitFor(() => expect(core.startHelmOp).toHaveBeenCalled());

    const [context, args, , , extraKubeconfigs] = core.startHelmOp.mock.calls[0] as [
      string,
      string[],
      unknown,
      unknown,
      string[],
    ];
    expect(context).toBe("prod-eu");
    expect(`helm ${args.join(" ")}`).toBe(shown);
    expect(extraKubeconfigs).toEqual(FILES);
  });

  it("opens the values editor as an upgrade of the selected release", async () => {
    open();
    await ready();
    await select("payments");
    await waitFor(() => expect(railHead()).toContain("payments"));

    await userEvent.click(screen.getByRole("button", { name: "Values editor" }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("Upgrade payments")).toBeTruthy();
    // The footer button is named for the editor, so it had better open on the
    // release's own values — it is the same upgrade path as the row action.
    await waitFor(() =>
      expect(document.querySelector(".cm-content")?.textContent).toContain("replicas: 4"),
    );
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

    await select("checkout", "checkout");
    await waitFor(() => expect(railHead()).toContain("checkout · 118 → 119"));
    expect(railHead()).not.toContain("payments");
  });

  /**
   * The namespace half of the row key, which is the half that is load-bearing.
   *
   * `checkout/checkout` and `staging/checkout` are two releases with one name.
   * A name-only key resolves both to whichever came back first, so clicking
   * staging's row would diff PRODUCTION's release — and the footer's
   * `Roll back to N`, which reads the same lookup, would then offer to operate
   * on it.
   */
  it("follows the row the reader clicked when two releases share a name", async () => {
    open();
    await ready();

    await select("checkout", "staging");
    await waitFor(() => expect(railHead()).toContain("checkout · 6 → 7"));
    expect(railHead()).not.toContain("119");
    // The footer acts on the row that was clicked, not on the one the lookup
    // happened to find.
    expect(screen.getByRole("button", { name: "Roll back to 6" })).toBeTruthy();

    await select("checkout", "checkout");
    await waitFor(() => expect(railHead()).toContain("checkout · 118 → 119"));
    expect(screen.getByRole("button", { name: "Roll back to 118" })).toBeTruthy();
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
  /** Start an upgrade of production's `checkout` from the table, and arm it. */
  async function upgradeCheckout() {
    const armed = armOperation();
    await select("checkout", "checkout");
    await waitFor(() => expect(railHead()).toContain("118 → 119"));
    await userEvent.click(
      within(rowFor("checkout", "checkout")).getByRole("button", { name: "Upgrade checkout" }),
    );
    await screen.findByRole("dialog");
    await userEvent.click(screen.getByRole("button", { name: "Upgrade" }));
    await waitFor(() => expect(core.startHelmOp).toHaveBeenCalled());
    return armed;
  }

  /** What helm left behind: `checkout` moved on, at whatever status it reached. */
  function moved(revision: number, status: string) {
    return {
      releases: RELEASES.map((r) =>
        r.name === "checkout" && r.namespace === "checkout" ? { ...r, revision, status } : r,
      ),
    };
  }

  it("refreshes the revision when an operation finishes, so the diff moves on", async () => {
    open();
    await ready();
    const armed = await upgradeCheckout();

    // The upgrade lands: helm moved the release to 120.
    core.listHelmReleases.mockResolvedValue(moved(120, "deployed"));
    await armed.fire(null);

    await waitFor(() => expect(railHead()).toContain("119 → 120"));
  });

  /**
   * **A failed operation refreshes too.**
   *
   * `helm upgrade --wait` writes revision 120 and THEN gives up waiting for it
   * to come up. The revision exists, at `failed`. A refresh that only listened
   * for clean exits would leave the table reading `119 / deployed` over a
   * release that is neither, with the failure banner as the reader's only clue
   * that anything moved at all — which is the stale pair this effect exists to
   * prevent, arriving by the other door.
   */
  it("refreshes when an operation fails, because a failed upgrade still writes a revision", async () => {
    open();
    await ready();
    const armed = await upgradeCheckout();

    core.listHelmReleases.mockResolvedValue(moved(120, "failed"));
    await armed.fire("timed out waiting for the condition");

    const row = () => rowFor("checkout", "checkout");
    await waitFor(() => expect(cell(row(), "Rev")).toBe("120"));
    expect(verdict(row())).toEqual({ word: "failed", kind: "danger" });
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

describe("Helm — when to list again", () => {
  it("lists once on a mount that already has a settled operation behind it", async () => {
    // The ops store is module-level and outlives the screen: an upgrade
    // started here, finished while the reader was on another tab, and still
    // sitting in the store when they come back.
    await settleElsewhere(CTX.name);
    core.listHelmReleases.mockClear();

    open();
    await ready();
    await act(async () => {});

    // One listing, not the mount's racing the settle effect's.
    expect(core.listHelmReleases).toHaveBeenCalledTimes(1);
  });

  it("lists again for this cluster's operations and no one else's", async () => {
    open();
    await ready();
    core.listHelmReleases.mockClear();

    // Another cluster's upgrade changes nothing in the list on screen.
    await settleElsewhere("edge-apac");
    expect(core.listHelmReleases).not.toHaveBeenCalled();

    // This one's does.
    await settleElsewhere(CTX.name);
    await waitFor(() => expect(core.listHelmReleases).toHaveBeenCalledTimes(1));
  });

  /**
   * The re-list guard is keyed on the CLUSTER, and a namespace change must not
   * re-arm it: an operation that settles after the reader has narrowed the
   * view still has to refresh the list — once, scoped to what they are now
   * looking at.
   */
  it("keeps refreshing on a settled operation after the namespace moved", async () => {
    open();
    await ready();
    await pickOnly("checkout");
    await waitFor(() => expect(drawn()).toEqual(["checkout/checkout", "checkout/redis-session"]));
    core.listHelmReleases.mockClear();

    await settleElsewhere(CTX.name);
    await waitFor(() => expect(core.listHelmReleases).toHaveBeenCalledTimes(1));
    // Scoped to the namespace on screen, not back to the whole cluster.
    expect(listedNamespaces()).toEqual(["checkout"]);
  });
});

/**
 * **The defect the user reported: "namespace selector is missing in helm
 * releases".**
 *
 * This screen listed every namespace with a Namespace column and no way to
 * filter — a regression against classic, and the one ported screen without the
 * control. On the reporter's own cluster that is 383 releases across dozens of
 * namespaces in one undifferentiated list.
 *
 * The selection is the SHARED, per-cluster one every other screen uses, not a
 * Helm-local filter: a namespace picked on Workloads carries to Helm and back.
 */
describe("Helm — the namespace selector", () => {
  it("offers the cluster's namespaces in the filter bar above the table", async () => {
    open();
    await ready();
    const bar = screen.getByRole("search", { name: /Filter/ });
    expect(within(bar).getByRole("combobox", { name: "Namespaces" })).toBeTruthy();
  });

  /**
   * **The backend call is scoped, not just the render.** Fetching 383 releases
   * to draw two is what this screen did on every keystroke of the reader's
   * attention; classic has always scoped the call for a single selection.
   */
  it("scopes the listing to a single selected namespace", async () => {
    open();
    await ready();
    core.listHelmReleases.mockClear();

    await pickOnly("checkout");

    await waitFor(() => expect(listedNamespaces()).toEqual(["checkout"]));
    expect(drawn()).toEqual(["checkout/checkout", "checkout/redis-session"]);
  });

  /**
   * The multi-select case, which is the one helm itself cannot answer: `helm
   * list --namespace` takes one namespace, so several means fetching every
   * namespace and narrowing here — classic's own rule.
   */
  it("fetches every namespace and narrows here when several are selected", async () => {
    open();
    await ready();
    await pickOnly("checkout");
    await waitFor(() => expect(drawn()).toEqual(["checkout/checkout", "checkout/redis-session"]));
    core.listHelmReleases.mockClear();

    act(() => setNamespaces(CTX.stableId, ["checkout", "staging"]));

    await waitFor(() =>
      expect(drawn()).toEqual([
        "checkout/checkout",
        "checkout/redis-session",
        "staging/checkout",
      ]),
    );
    // One listing, unscoped — and the platform and payments releases it
    // returned are gone from the table.
    expect(listedNamespaces()).toEqual([null]);
    expect(rowFor("ingress-nginx")).toBeUndefined();
  });

  /**
   * The other half of that rule: narrowing a whole-cluster listing to several
   * namespaces asks helm for nothing new. The releases are already here — the
   * only thing that changed is how many of them are drawn.
   */
  it("does not re-list when the listing it already has covers the new selection", async () => {
    open();
    await ready();
    core.listHelmReleases.mockClear();

    act(() => setNamespaces(CTX.stableId, ["checkout", "staging"]));

    await waitFor(() => expect(drawn()).toHaveLength(3));
    expect(core.listHelmReleases).not.toHaveBeenCalled();
  });

  /**
   * **Per cluster, in the shared store — not per screen.** Local state here
   * would pass every assertion above and lose the reader's namespace the
   * moment they walked to Workloads and back.
   */
  it("writes the pick to the cluster's shared selection", async () => {
    open();
    await ready();

    await pickOnly("payments");

    await waitFor(() => expect(getView().namespaces.prod).toEqual(["payments"]));
  });

  it("opens on the namespace another screen on this cluster already chose", async () => {
    setNamespaces(CTX.stableId, ["payments"]);

    open();

    // The FIRST listing is already scoped: the remembered selection is read
    // before anything is fetched, not applied to rows that arrived unscoped.
    await waitFor(() => expect(drawn()).toEqual(["payments/payments"]));
    expect(listedNamespaces()).toEqual(["payments"]);
  });

  /** One listing per change of mind — not the mount's racing the re-list. */
  it("lists exactly once when the namespace changes", async () => {
    open();
    await ready();
    core.listHelmReleases.mockClear();

    await pickOnly("platform");
    await waitFor(() => expect(drawn()).toEqual(["platform/ingress-nginx"]));
    await act(async () => {});

    expect(core.listHelmReleases).toHaveBeenCalledTimes(1);
  });

  /**
   * **A late answer must not paint over a fresh one.** The reader narrows to
   * `checkout`, the cluster is slow, they widen back to everything — and the
   * two-row answer for `checkout` lands after the five-row answer for the
   * whole cluster. Without the sequence guard the table ends up showing two
   * rows under a head that says the cluster has two.
   */
  it("discards a listing that arrives after the reader moved on", async () => {
    const pending = new Map<string, (value: { releases: HelmReleaseSummary[] }) => void>();
    core.listHelmReleases.mockImplementation(
      (_ctx: string, namespace?: string | null) =>
        new Promise<{ releases: HelmReleaseSummary[] }>((resolve) => {
          pending.set(namespace ?? "", resolve);
        }),
    );

    setNamespaces(CTX.stableId, ["checkout"]);
    open();
    await waitFor(() => expect(pending.has("checkout")).toBe(true));

    act(() => setNamespaces(CTX.stableId, []));
    await waitFor(() => expect(pending.has("")).toBe(true));

    // The fresh answer first…
    await act(async () => {
      pending.get("")?.({ releases: RELEASES });
    });
    await ready();

    // …then the stale one, for a namespace nobody is looking at any more.
    await act(async () => {
      pending.get("checkout")?.({ releases: [CHECKOUT, REDIS] });
    });

    expect(drawn()).toHaveLength(5);
    expect(rowFor("ingress-nginx")).toBeTruthy();
    expect(heads()[0]).toBe("Releases · 5 in this cluster");
  });

  /**
   * A head that counts the whole cluster above a table showing one namespace
   * is a string asserting more than the view holds — the same class of defect
   * this branch keeps finding.
   */
  it("heads the pane with what the reader is looking at, not with the cluster's total", async () => {
    open();
    await ready();
    expect(heads()[0]).toBe("Releases · 5 in this cluster");

    await pickOnly("checkout");
    await waitFor(() => expect(heads()[0]).toBe("Releases · 2 in checkout"));

    act(() => setNamespaces(CTX.stableId, ["checkout", "staging"]));
    await waitFor(() => expect(heads()[0]).toBe("Releases · 3 in 2 namespaces"));
  });

  it("says a namespace has no releases without blaming a failure", async () => {
    setNamespaces(CTX.stableId, ["platform"]);
    core.listHelmReleases.mockResolvedValue({ releases: [] });

    open();

    expect(await screen.findByText("No Helm releases")).toBeTruthy();
    expect(
      screen.getByText("prod-eu has no Helm releases in the namespaces you are looking at."),
    ).toBeTruthy();
    expect(screen.queryByText(/Could not list/)).toBeNull();
  });

  it("filters the releases on screen by text, and says which filter is hiding them", async () => {
    open();
    await ready();

    await userEvent.type(screen.getByRole("searchbox", { name: "Filter releases" }), "redis");
    await waitFor(() => expect(drawn()).toEqual(["checkout/redis-session"]));

    await userEvent.clear(screen.getByRole("searchbox", { name: "Filter releases" }));
    await userEvent.type(screen.getByRole("searchbox", { name: "Filter releases" }), "zzz");
    expect(await screen.findByText("No Helm releases match this filter")).toBeTruthy();
    expect(screen.getByText("Clear the filter to see all 5.")).toBeTruthy();
  });

  it("follows the namespace a restricted credential is scoped to", async () => {
    useNamespaceOptions.mockReturnValue({ namespaces: ["payments"], scope: "payments", error: "" });

    open();

    // Written to the shared store, so every screen on this cluster follows.
    await waitFor(() => expect(getView().namespaces.prod).toEqual(["payments"]));
    await waitFor(() => expect(drawn()).toEqual(["payments/payments"]));
    expect(listedNamespaces()).toContain("payments");
  });

  it("shows the picker as loading rather than empty before the namespaces arrive", async () => {
    useNamespaceOptions.mockReturnValue({ namespaces: null, scope: "", error: "" });

    open();
    await ready();

    expect(screen.queryByRole("combobox", { name: "Namespaces" })).toBeNull();
    const placeholder = screen.getByRole("button", { name: "Namespaces" }) as HTMLButtonElement;
    expect(placeholder.disabled).toBe(true);
    expect(within(placeholder).getByRole("status", { name: "Loading namespaces" })).toBeTruthy();
  });

  it("warns when the namespaces cannot be listed, without hiding the picker or the releases", async () => {
    useNamespaceOptions.mockReturnValue({
      namespaces: NAMESPACES,
      scope: "",
      error: "namespaces: etcd timeout",
    });

    open();

    expect(await screen.findByText("Namespaces could not be listed")).toBeTruthy();
    // core's classification, not the apiserver's own words.
    expect(screen.getByText(/didn't respond in time/)).toBeTruthy();
    expect(screen.getByRole("combobox", { name: "Namespaces" })).toBeTruthy();
    await waitFor(() => expect(drawn()).toHaveLength(5));
  });

  it("explains a remembered namespace that is gone, and offers the way back", async () => {
    setNamespaces(CTX.stableId, ["deleted-ns"]);

    open();

    expect(await screen.findByText("Remembered namespaces are gone")).toBeTruthy();
    expect(screen.getByText("deleted-ns no longer exist on this cluster.")).toBeTruthy();

    await userEvent.click(screen.getByRole("button", { name: "Show all namespaces" }));
    await waitFor(() => expect(getView().namespaces.prod).toBeUndefined());
    await waitFor(() => expect(drawn()).toHaveLength(5));
  });
});
