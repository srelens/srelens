import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

/**
 * **Only the capability wrappers and the platform read are replaced.**
 *
 * `describeError`, `plural` and `contextDisplayName` stay real, so the failure
 * copy and both halves of the count are core's own arithmetic rather than a
 * copy of it here. `latencyLabel` and `viaOf` stay real for the same reason —
 * they are what the connections table and this screen have to agree on.
 */
const core = vi.hoisted(() => ({
  listContexts: vi.fn(),
  connectCluster: vi.fn(),
  isTauri: vi.fn(),
  pickKubeconfigFiles: vi.fn(),
  saveKubeconfigFiles: vi.fn(),
  savePastedKubeconfig: vi.fn(),
}));
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  ...core,
}));

import { describeError, type ClusterContext, type ClusterInfo } from "@srelens/core";
import { Connect } from "./Connect";
import { ClusterTable } from "./connections/ClusterTable";
import { getKubeconfigFiles, resetContexts, setContexts, setKubeconfigFiles } from "../lib/clusters";
import { getProbe, resetProbes } from "../lib/probe";
import { defaultState } from "../lib/tabs";
import * as store from "../lib/tabsStore";
import { resetView } from "../lib/workspace";
import { latencyLabel } from "./connections/clusterText";

const ROUTE = "/connect";

/** The kubeconfig two of the three contexts were declared in. */
const CONFIG = "/home/dana/.kube/config";
/** A second file, which is what makes the file half of the count a real count. */
const EDGE_FILE = "/home/dana/work/edge.yaml";

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

const EDGE: ClusterContext = {
  name: "edge-1",
  stableId: "edge-1",
  cluster: "edge",
  server: "https://edge-1.example:6443",
  isCurrent: false,
  sourceFile: EDGE_FILE,
  authKind: "client certificate",
};

/**
 * A local cluster with **no source file at all** — a synthesized kubeconfig
 * carries `sourceFile: ""`, which core says is the empty string rather than a
 * path. It is the row that separates "count the rows' files" from "count the
 * rows": four contexts here still come from two files.
 */
const LOCAL: ClusterContext = {
  name: "kind-lab",
  stableId: "kind-lab",
  cluster: "kind-lab",
  server: "https://127.0.0.1:52001",
  isCurrent: false,
  isLocal: true,
  provider: "kind",
  sourceFile: "",
  authKind: "client certificate",
};

/** The three default contexts: two in one file, one in another. */
const THREE = [PROD, STAGING, EDGE];

/**
 * **No stored kubeconfig files by default**, which is web mode's own shape
 * (`Window.tsx` hands it `[]`) and the shape that separates a count of the
 * stored list from a count of what the rows say.
 */
const FILES: string[] = [];

/** What each cluster answers, and how long it takes about it. */
const REACHABLE: Record<string, ClusterInfo> = {
  "prod-eu": { context: "prod-eu", reachable: true, version: "v1.31.2" },
  "staging-eu": { context: "staging-eu", reachable: true, version: "v1.30.6" },
  "edge-1": { context: "edge-1", reachable: true, version: "v1.29.4" },
  "kind-lab": { context: "kind-lab", reachable: true, version: "v1.31.0" },
};
const LATENCY: Record<string, number> = {
  "prod-eu": 41,
  "staging-eu": 12,
  "edge-1": 7,
  "kind-lab": 3,
};

/**
 * A listing failure with a machine string in it, classified by `describeError`
 * (it matches its connect branch) — the only shape that can tell "reported
 * through `describeError`" apart from "printed the backend's string".
 */
const LIST_FAILURE = "ServiceError: connection refused: ECONNREFUSED 10.0.5.2:6443";

/**
 * A frozen clock, advanced only by the `connectCluster` double: `probeCluster`
 * times its own round trip off `Date.now`, so this is the only way a latency
 * assertion can be an exact number rather than whatever the machine took.
 */
let clock = 0;

beforeEach(() => {
  vi.clearAllMocks();
  clock = 1_700_000_000_000;
  vi.spyOn(Date, "now").mockImplementation(() => clock);
  core.listContexts.mockResolvedValue({ contexts: THREE });
  core.connectCluster.mockImplementation(async (name: string) => {
    clock += LATENCY[name] ?? 0;
    return REACHABLE[name] ?? { context: name, reachable: false, error: "no route to host" };
  });
  // Web by default, so a desktop-only control has to be asked for explicitly.
  core.isTauri.mockReturnValue(false);
  core.pickKubeconfigFiles.mockResolvedValue([]);
  resetProbes();
  resetContexts();
  setContexts(THREE);
  setKubeconfigFiles(FILES);
  store.setState(defaultState(THREE));
  resetView();
});

afterEach(() => {
  vi.restoreAllMocks();
});

function open() {
  store.openTab(ROUTE);
  return render(<Connect route={ROUTE} />);
}

/** The count as it is drawn, read from its own node rather than from the page. */
function countText(): string {
  return screen.getByTestId("connect-count").textContent ?? "";
}

/** One row's cells, by test id — never a read of the whole row, which would let
 *  the mark's initials and the badge's word ride along with the name. */
function row(stableId: string) {
  return {
    name: screen.getByTestId(`connect-name-${stableId}`),
    detail: screen.queryByTestId(`connect-detail-${stableId}`),
    status: screen.getByTestId(`connect-status-${stableId}`),
    latency: screen.queryByTestId(`connect-latency-${stableId}`),
  };
}

describe("Connect", () => {
  describe("what §24 puts on the page", () => {
    it("draws the eyebrow, both headline lines and the lede", async () => {
      core.isTauri.mockReturnValue(true);
      open();
      await screen.findByText("Contexts found");

      expect(screen.getByText("srelens · local-first")).toBeTruthy();
      expect(screen.getByText("Pick a cluster.")).toBeTruthy();
      expect(screen.getByText("The room is already reading it.")).toBeTruthy();
      expect(
        screen.getByText(
          "srelens uses the credentials already in your kubeconfig and talks to the API server directly. That file stays on this machine, and no srelens service sits between you and your clusters.",
        ),
      ).toBeTruthy();
    });

    /**
     * **The absolute promise is gone, and its absence is half the assertion.**
     *
     * The lede used to end "Nothing about your clusters leaves this machine",
     * and srelens cannot keep that. `srelens --mcp-stdio` serves
     * `k8s.listContexts` as a READ-ONLY capability, and only a mutating one is
     * confirm-gated — `assert_mutating_capabilities_are_gated`
     * (`crates/mcp/src/completeness.rs:36-45`) fails the build for a mutating
     * capability that is not, and says nothing about a read. So a connected
     * agent, typically a remote model, can read every cluster name, server,
     * source file and credential kind on this page with no prompt at all. The
     * reader configured that themselves; the sentence promised it could not
     * happen.
     *
     * What is kept is what holds unconditionally, and it is worth keeping: the
     * kubeconfig is not uploaded, and there is no srelens service between the
     * reader and their API servers. Pinned as the ABSENCE of the false
     * sentence as well as the presence of the true one, because a narrowed
     * privacy claim is exactly the copy a well-meaning revert restores — and a
     * presence-only test would pass with both sentences on the page.
     */
    it("does not promise a desktop reader that nothing about their clusters leaves the machine", async () => {
      core.isTauri.mockReturnValue(true);
      open();
      await screen.findByText("Contexts found");

      expect(screen.queryByText(/Nothing about your clusters leaves this machine/)).toBeNull();
      expect(screen.queryByText(/leaves this machine/)).toBeNull();
      expect(screen.getByText(/That file stays on this machine/)).toBeTruthy();
      expect(screen.getByText(/no srelens service sits between you and your clusters/)).toBeTruthy();
    });

    /**
     * **The lede is a claim about where the kubeconfig is, so it cannot be one
     * string.**
     *
     * §24's own words are true of the desktop and false of the web build, on
     * the same page as {@link WEB_ONLY}'s "This build talks to a shared
     * server": in web mode the kubeconfig is the SERVER's, the clusters are
     * read on the server's host, and web mode over-listing the server's
     * contexts is a filed bug (#347). The screen already branched its
     * empty-state hint for exactly this; the lede did not.
     *
     * Asserted as the absence of the specific false sentence rather than as
     * the presence of the true one alone: a branch that added the web copy and
     * left the local-first claim standing beside it would satisfy a
     * presence-only test.
     */
    it("does not tell a web reader their clusters never leave this machine", async () => {
      open();
      await screen.findByText("Contexts found");

      expect(screen.queryByText(/Nothing about your clusters leaves this machine/)).toBeNull();
      expect(screen.queryByText(/already in your kubeconfig/)).toBeNull();
      expect(
        screen.getByText(
          "srelens uses the credentials in the kubeconfig this server was started with, and talks to the API server directly from the server's host. The clusters listed below are the ones that server can see, not the ones on the machine you are reading this on.",
        ),
      ).toBeTruthy();
    });

    it("does not call a shared-server build local-first", async () => {
      open();
      await screen.findByText("Contexts found");

      expect(screen.queryByText("srelens · local-first")).toBeNull();
      expect(screen.getByText("srelens · shared server")).toBeTruthy();
    });

    it("is full-bleed: no screen toolbar over it", async () => {
      const { container } = open();
      await screen.findByText("Contexts found");
      // `Screen` draws a `.toolbar` with the route's title in it. §24 has none —
      // the headline IS the head of this page. Asserted on the class the kit's
      // Toolbar actually carries, so wrapping this in a `Screen` fails here.
      expect(container.querySelector(".toolbar")).toBeNull();
    });

    it("rises in, centred at 860px", async () => {
      open();
      await screen.findByText("Contexts found");
      const column = screen.getByTestId("connect-column");
      expect(column.className).toContain("rise");
      expect(column.className).toContain("max-w-[860px]");
      expect(column.className).toContain("mx-auto");
    });
  });

  describe("the count", () => {
    it("counts the contexts it lists, and the files they came from", async () => {
      open();
      await screen.findByText("Contexts found");

      // Pinned against the rows that are actually on the page, not against the
      // fixture's own length: a count read from a different array than the rows
      // is the defect this screen inherits from §24's own mock (`5 in 2 files`
      // over eight rows).
      const rows = screen.getAllByTestId("connect-context");
      expect(rows).toHaveLength(3);
      expect(countText()).toBe(`${rows.length} in 2 files`);
    });

    it("counts only the files the rows name, and says `file` for one of them", async () => {
      setContexts([PROD, STAGING]);
      open();
      await screen.findByText("Contexts found");

      const rows = screen.getAllByTestId("connect-context");
      expect(rows).toHaveLength(2);
      expect(countText()).toBe(`${rows.length} in 1 file`);
    });

    it("does not count a file for a cluster that names none", async () => {
      setContexts([...THREE, LOCAL]);
      open();
      await screen.findByText("Contexts found");

      // Four rows, still two files: the local cluster's `sourceFile` is `""`,
      // and an empty string counted as a source is a file the reader cannot go
      // and look at.
      const rows = screen.getAllByTestId("connect-context");
      expect(rows).toHaveLength(4);
      expect(countText()).toBe(`${rows.length} in 2 files`);
    });

    it("counts the rows it has after a listing changes them", async () => {
      core.isTauri.mockReturnValue(true);
      core.pickKubeconfigFiles.mockResolvedValue([EDGE_FILE]);
      // The second listing answers with one more context, from one more file.
      core.listContexts.mockResolvedValue({ contexts: [...THREE, LOCAL] });
      setContexts([PROD, STAGING]);
      open();
      await screen.findByText("Contexts found");
      expect(countText()).toBe("2 in 1 file");

      await userEvent.click(screen.getByRole("button", { name: /Add a kubeconfig file/i }));

      await waitFor(() => expect(screen.getAllByTestId("connect-context")).toHaveLength(4));
      expect(countText()).toBe(`${screen.getAllByTestId("connect-context").length} in 2 files`);
    });

    it("puts no count over a card with nothing in it", async () => {
      setContexts([]);
      open();
      await screen.findByText("Contexts found");
      // A count of nothing asserted as a fact is the same fault as a count that
      // disagrees with its rows.
      expect(screen.queryByTestId("connect-count")).toBeNull();
    });
  });

  describe("the rows", () => {
    it("names each context and says which file it came from", async () => {
      open();
      await screen.findByText("Contexts found");

      expect(row("prod-eu").name.textContent).toBe("prod-eu");
      expect(row("prod-eu").detail?.textContent).toBe(CONFIG);
      expect(row("edge-1").detail?.textContent).toBe(EDGE_FILE);
    });

    it("says how a local cluster is reached, which is not a file", async () => {
      setContexts([LOCAL]);
      open();
      await screen.findByText("Contexts found");
      // `viaOf`'s answer for a local cluster: the tool that made it and the
      // endpoint it listens on. The same helper the connections table uses.
      expect(row("kind-lab").detail?.textContent).toBe("kind · 127.0.0.1:52001");
    });

    it("badges each row from its own probe, and never calls one healthy", async () => {
      core.connectCluster.mockImplementation(async (name: string) => {
        clock += LATENCY[name] ?? 0;
        if (name === "edge-1") return { context: name, reachable: false, error: "no route to host" };
        return REACHABLE[name];
      });
      open();

      await waitFor(() => expect(row("prod-eu").status.textContent).toBe("reachable"));
      await waitFor(() => expect(row("edge-1").status.textContent).toBe("unreachable"));

      // Decision 3: the probe reports whether the API server answered, so no
      // row may claim a health verdict nothing checked.
      expect(screen.queryByText(/healthy/i)).toBeNull();
      expect(screen.queryByText(/degraded/i)).toBeNull();
    });

    it("says `no reading` for a cluster nothing has answered for yet", async () => {
      /**
       * A cluster that never answers: its own row says so and no other row
       * waits.
       *
       * **The cluster that hangs is the FIRST one in the list, deliberately.**
       * It was `edge-1`, which is last in `THREE` — and with it last, a screen
       * that read the clusters one after another satisfied this test on fixture
       * order alone. Verified rather than assumed: a serial probe loop passed
       * all 26 tests. Hanging the first one is what makes "no other row waits"
       * the thing being proved, and it is the case that matters — a laptop
       * kubeconfig whose first context is a VPN-only production cluster would
       * otherwise spend a full timeout before any other row said anything.
       */
      core.connectCluster.mockImplementation(async (name: string) => {
        if (name === "prod-eu") return new Promise<ClusterInfo>(() => {});
        clock += LATENCY[name] ?? 0;
        return REACHABLE[name];
      });
      open();

      await waitFor(() => expect(row("edge-1").status.textContent).toBe("reachable"));
      expect(row("prod-eu").status.textContent).toBe("no reading");
      // And no invented duration for it — decision 4.
      expect(row("prod-eu").latency).toBeNull();
    });

    it("draws the round trip through the one formatter", async () => {
      open();
      await waitFor(() => expect(row("prod-eu").latency?.textContent).toBeTruthy());

      /**
       * **Not a hardcoded `41 ms`.** The clock above is one shared counter that
       * the `connectCluster` double advances the moment it is CALLED, so a
       * per-cluster figure only comes out if the screen probes the clusters one
       * at a time — and it deliberately does not (see the test above: a cluster
       * that never answers must hold up no other row, which on a real
       * kubeconfig is the difference between one timeout and one per cluster).
       * Probed at once, `prod-eu`'s reading is 41 + 12 + 7 = 60 ms: an artifact
       * of the fake clock, not of the screen.
       *
       * What the name of this test is actually about survives, and is what is
       * pinned: every row draws `latencyLabel`'s own answer for its own probe.
       * A screen printing `${probe.latencyMs}ms`, one drawing another row's
       * reading, or a second formatter that rounds a sub-millisecond reading to
       * `0 ms` all fail here.
       */
      for (const id of ["prod-eu", "staging-eu", "edge-1"]) {
        const label = latencyLabel(getProbe(id));
        expect(label).toMatch(/^\d+ ms$/);
        expect(row(id).latency?.textContent).toBe(label);
      }
      // Each row's own reading, not one number copied down the list.
      expect(row("prod-eu").latency?.textContent).not.toBe(row("edge-1").latency?.textContent);
    });

    it("opens the cluster the row stands for", async () => {
      open();
      await screen.findByText("Contexts found");

      await userEvent.click(screen.getByRole("button", { name: /Open edge-1/i }));

      expect(store.activeCluster()).toBe("edge-1");
      /**
       * The tab and the cluster, as the store actually models them.
       *
       * `getState().tabs` was `undefined` — the state is
       * `{ workspaces, currentId }` and the tabs hang off the current workspace
       * — so the original assertion threw instead of checking anything. And a
       * `Tab` carries no `clusterName`: `makeTab` spends the option on the
       * tab's `sub` and the cluster in focus is the WORKSPACE's, which is why
       * the membership line below is the load-bearing one — `setActiveCluster`
       * refuses an id the workspace does not hold, so a row for a context no
       * workspace has would otherwise do nothing at all, silently.
       */
      expect(store.currentWorkspace().tabs.some((t) => t.route === "/overview")).toBe(true);
      expect(store.currentWorkspace().clusters).toContain("edge-1");
    });

    it("caps the strings that have no bound, on the row and inside it", async () => {
      open();
      await screen.findByText("Contexts found");

      // `min-width: auto` on a flex child refuses to shrink below its content,
      // so a 70-character kubeconfig path in an 860px card pushes the badge off
      // the end. Eight defects on this migration, none of them visible in
      // jsdom — hence these class assertions.
      const rows = screen.getAllByTestId("connect-context");
      for (const drawn of rows) expect(drawn.className).toContain("min-w-0");

      const text = screen.getByTestId("connect-text-prod-eu");
      expect(text.className).toContain("min-w-0");

      // `block` is what makes `truncate`'s overflow apply at all, and the cap is
      // what stops the intrinsic width being the whole string.
      for (const cell of [row("prod-eu").name, row("prod-eu").detail]) {
        expect(cell?.className).toContain("block");
        expect(cell?.className).toContain("truncate");
        expect(cell?.className).toContain("max-w-");
      }

      // The badge and the reading keep their own width whatever the name does.
      expect(row("prod-eu").status.parentElement?.className).toContain("shrink-0");
    });
  });

  /**
   * **One set of clusters, one order, across both screens.**
   *
   * `/connections` groups its rows by credential source and `/connect` listed
   * in raw `listContexts` order, so the same seventeen clusters came out in two
   * different orders on two screens a reader moves between in one click. The
   * order itself is `bySource`'s, which now lives in `clusterText` beside the
   * status words — a second sort written here is how the two screens would
   * start disagreeing again.
   */
  describe("the order it lists in", () => {
    /**
     * Interleaved ON PURPOSE: the local cluster sits between two kubeconfig
     * contexts, so the expected order is not the order it was listed in. With
     * the local cluster last, a screen that applied no grouping at all would
     * satisfy this on fixture order alone.
     */
    const MIXED = [PROD, LOCAL, STAGING];

    it("lists kubeconfig contexts ahead of local clusters, whatever order they were listed in", async () => {
      setContexts(MIXED);
      open();
      await screen.findByText("Contexts found");

      expect(screen.getAllByTestId(/^connect-name-/).map((el) => el.textContent)).toEqual([
        "prod-eu",
        "staging-eu",
        "kind-lab",
      ]);
    });

    it("lists them in the same order the connections table does", async () => {
      setContexts(MIXED);
      open();
      await screen.findByText("Contexts found");
      const here = screen.getAllByTestId(/^connect-name-/).map((el) => el.textContent);

      // The other screen, from the same contexts. Rendered into its own
      // container so the two lists cannot read each other's rows.
      const table = render(
        <ClusterTable
          rows={MIXED.map((context) => ({ context, probe: { state: "unread" as const } }))}
          onOpen={() => {}}
        />,
      );
      const there = within(table.container)
        .getAllByTestId(/^cluster-name-/)
        .map((el) => el.textContent);

      expect(here).toEqual(there);
      // And not the raw listing order — which is what makes the line above a
      // claim about grouping rather than about two screens both doing nothing.
      expect(here).not.toEqual(MIXED.map((context) => context.name));
    });
  });

  describe("the two doors", () => {
    it("offers neither door to a build with no filesystem", async () => {
      open();
      await screen.findByText("Contexts found");

      expect(screen.queryByText(/add a kubeconfig file/i)).toBeNull();
      expect(screen.queryByText(/paste a context/i)).toBeNull();
      // Said once, rather than leaving a reader to wonder at an absent control.
      expect(screen.getByText(/on the desktop/i)).toBeTruthy();
    });

    it("offers both on the desktop, and says nothing about the desktop there", async () => {
      core.isTauri.mockReturnValue(true);
      open();
      await screen.findByText("Contexts found");

      expect(screen.getByRole("button", { name: /Add a kubeconfig file/i })).toBeTruthy();
      expect(screen.getByRole("button", { name: /Paste a context/i })).toBeTruthy();
      expect(screen.queryByText(/on the desktop/i)).toBeNull();
    });

    it("remembers a picked kubeconfig file, tells the backend, and lists again", async () => {
      core.isTauri.mockReturnValue(true);
      core.pickKubeconfigFiles.mockResolvedValue([EDGE_FILE]);
      core.listContexts.mockResolvedValue({ contexts: THREE });
      setContexts([PROD, STAGING]);
      setKubeconfigFiles([CONFIG]);
      open();
      await screen.findByText("Contexts found");

      await userEvent.click(screen.getByRole("button", { name: /Add a kubeconfig file/i }));

      // All three writes: the one that survives a restart, the one every core
      // call in this window reads, and the listing that puts the file's
      // contexts on the screen.
      await waitFor(() => expect(core.saveKubeconfigFiles).toHaveBeenCalledWith([CONFIG, EDGE_FILE]));
      expect(getKubeconfigFiles()).toEqual([CONFIG, EDGE_FILE]);
      expect(core.listContexts).toHaveBeenCalledWith([CONFIG, EDGE_FILE]);
      await waitFor(() => expect(screen.getAllByTestId("connect-context")).toHaveLength(3));
    });

    it("says why a kubeconfig file could not be added", async () => {
      core.isTauri.mockReturnValue(true);
      core.pickKubeconfigFiles.mockRejectedValue(new Error("no such file or directory"));
      open();
      await screen.findByText("Contexts found");

      await userEvent.click(screen.getByRole("button", { name: /Add a kubeconfig file/i }));

      await screen.findByText("Could not add that kubeconfig file");
      // Through `describeError`, never the backend's raw string on its own.
      expect(screen.getByText(describeError("no such file or directory").detail)).toBeTruthy();
    });

    it("writes a pasted context to a file, and lists again", async () => {
      core.isTauri.mockReturnValue(true);
      core.savePastedKubeconfig.mockResolvedValue("/app/kubeconfigs/pasted.yaml");
      core.listContexts.mockResolvedValue({ contexts: THREE });
      setContexts([PROD, STAGING]);
      setKubeconfigFiles([CONFIG]);
      open();
      await screen.findByText("Contexts found");

      await userEvent.click(screen.getByRole("button", { name: /Paste a context/i }));
      const yaml = "apiVersion: v1\nkind: Config\n";
      await userEvent.type(screen.getByLabelText("Kubeconfig YAML"), yaml);
      await userEvent.click(screen.getByRole("button", { name: "Save" }));

      await waitFor(() => expect(core.savePastedKubeconfig).toHaveBeenCalledWith(yaml, undefined));
      expect(core.saveKubeconfigFiles).toHaveBeenCalledWith([
        CONFIG,
        "/app/kubeconfigs/pasted.yaml",
      ]);
      expect(core.listContexts).toHaveBeenCalledWith([CONFIG, "/app/kubeconfigs/pasted.yaml"]);
      await waitFor(() => expect(screen.getAllByTestId("connect-context")).toHaveLength(3));
    });

    /**
     * **Where the file actually goes.**
     *
     * The hint said "beside your other kubeconfigs". `savePastedKubeconfig`
     * writes to `app_config_dir()/kubeconfigs/<stem>-<ts>.yaml` — srelens's
     * own folder — while the reader's kubeconfig is `~/.kube/config`. A reader
     * who wanted to find, edit or delete that context went to `~/.kube` and
     * found nothing. The frozen classic app words the same operation
     * correctly (`SettingsView.tsx`: "Saved securely in the srelens app
     * configuration directory"); the migration turned a true sentence into a
     * false one.
     */
    it("says where a pasted context is written, which is not beside the reader's kubeconfig", async () => {
      core.isTauri.mockReturnValue(true);
      open();
      await screen.findByText("Contexts found");

      await userEvent.click(screen.getByRole("button", { name: /Paste a context/i }));
      expect(screen.queryByText(/beside your other kubeconfigs/i)).toBeNull();
      expect(
        screen.getByText(
          "Written to a file of its own in the srelens app configuration folder, not into your own kubeconfig. srelens reads it in place from then on.",
        ),
      ).toBeTruthy();
    });

    it("will not save an empty paste", async () => {
      core.isTauri.mockReturnValue(true);
      open();
      await screen.findByText("Contexts found");

      await userEvent.click(screen.getByRole("button", { name: /Paste a context/i }));
      const save = screen.getByRole("button", { name: "Save" });
      expect(save.hasAttribute("disabled")).toBe(true);

      await userEvent.click(save);
      expect(core.savePastedKubeconfig).not.toHaveBeenCalled();
    });

    it("says why a pasted context could not be saved, and keeps what was typed", async () => {
      core.isTauri.mockReturnValue(true);
      core.savePastedKubeconfig.mockRejectedValue(new Error("permission denied"));
      open();
      await screen.findByText("Contexts found");

      await userEvent.click(screen.getByRole("button", { name: /Paste a context/i }));
      await userEvent.type(screen.getByLabelText("Kubeconfig YAML"), "kind: Config");
      await userEvent.click(screen.getByRole("button", { name: "Save" }));

      await screen.findByText("Could not save that context");
      // The dialog stays open with the paste still in it: a reader who lost a
      // pasted kubeconfig to a failed write has to go and find it again.
      expect((screen.getByLabelText("Kubeconfig YAML") as HTMLTextAreaElement).value).toBe(
        "kind: Config",
      );
    });
  });

  describe("the states before the rows", () => {
    it("says what to do when the kubeconfig has nothing in it", async () => {
      setContexts([]);
      open();
      expect(await screen.findByText(/no contexts/i)).toBeTruthy();
    });

    it("waits rather than claiming the kubeconfig is empty", async () => {
      // The store's boot state: nothing listed, and no listing finished yet.
      resetContexts();
      open();
      await screen.findByText("Contexts found");
      expect(screen.queryByText(/no contexts/i)).toBeNull();
      expect(screen.getByLabelText("Listing your contexts")).toBeTruthy();
    });

    it("says why the listing refused, and lists again on request", async () => {
      setContexts([], LIST_FAILURE);
      core.listContexts.mockResolvedValue({ contexts: THREE });
      open();

      await screen.findByText("Could not read your kubeconfig");
      expect(screen.getByText(describeError(LIST_FAILURE).detail)).toBeTruthy();
      /**
       * The machine string is folded away, not printed as the sentence.
       *
       * Asserted on the two slots rather than with `queryByText(LIST_FAILURE)`:
       * `RawError` keeps the original in a CLOSED `<details>`, which is exactly
       * where it belongs, and `queryByText` cannot tell that from a paragraph —
       * so the original assertion demanded the string be thrown away. (Task 7
       * settled the same point.) What matters is which slot it is in.
       */
      const card = screen.getByRole("alert");
      expect(card.querySelector('[data-slot="detail"]')?.textContent).not.toContain(LIST_FAILURE);
      expect(card.querySelector('[data-slot="raw"] pre')?.textContent).toBe(LIST_FAILURE);

      await userEvent.click(screen.getByRole("button", { name: /Try again/i }));
      await waitFor(() => expect(screen.getAllByTestId("connect-context")).toHaveLength(3));
    });
  });

  describe("the footer strip", () => {
    it("carries the sparkle and its copy", async () => {
      core.isTauri.mockReturnValue(true);
      const strip = (open(), await screen.findByTestId("connect-footer"));
      expect(strip.querySelector("svg")).toBeTruthy();
      expect(
        within(strip).getByText(
          "srelens reads each cluster directly, with the credentials already in your kubeconfig, and never copies or uploads that file. Connect an agent to srelens over MCP and it can read this list, and the clusters on it, without asking first; every change it makes stops at a confirmation prompt. Asking the console about a cluster in plain language is not in this design yet.",
        ),
      ).toBeTruthy();
    });

    /**
     * **The agent's READ access gets its own clause, and the old absolute
     * claim's absence is pinned beside it.**
     *
     * The strip used to end "and sends that file nowhere" — a claim about the
     * kubeconfig, which is true, wrapped around an absolute the neighbouring
     * lede also made. `SourcesRail`'s own section already tells the reader that
     * every agent CHANGE stops at a confirmation prompt; nothing anywhere told
     * them the agent can READ freely, which is the half of the MCP story that
     * makes "nothing leaves this machine" false. So the true, narrow claim
     * about the file stays ("never copies or uploads that file"), and the read
     * is said plainly next to it.
     *
     * The confirmation half is worded to agree with the rail rather than to
     * paraphrase it: two panes one click apart must not describe one gate two
     * ways.
     */
    it("tells the desktop reader the agent may read freely and change nothing unasked", async () => {
      core.isTauri.mockReturnValue(true);
      const strip = (open(), await screen.findByTestId("connect-footer"));
      expect(within(strip).queryByText(/sends that file nowhere/)).toBeNull();
      expect(within(strip).getByText(/never copies or uploads that file/)).toBeTruthy();
      expect(within(strip).getByText(/read this list, and the clusters on it, without asking first/)).toBeTruthy();
      expect(within(strip).getByText(/every change it makes stops at a confirmation prompt/)).toBeTruthy();
    });

    /**
     * The strip used to promise "ask the console about it in plain language.
     * srelens reads the cluster to answer" — and the console answers nothing
     * in this design: `shell/Console.tsx` renders "The agent is not in the new
     * design yet". Pinned as the absence of the promise, because the honest
     * replacement is a sentence a future edit could drop while re-adding the
     * old one.
     */
    it("does not promise a console that answers", async () => {
      core.isTauri.mockReturnValue(true);
      const strip = (open(), await screen.findByTestId("connect-footer"));
      expect(within(strip).queryByText(/reads the cluster to answer/)).toBeNull();
      expect(within(strip).getByText(/not in this design yet/)).toBeTruthy();
    });

    it("does not call the server's kubeconfig the reader's", async () => {
      const strip = (open(), await screen.findByTestId("connect-footer"));
      expect(within(strip).queryByText(/your kubeconfig/)).toBeNull();
      expect(
        within(strip).getByText(
          "srelens reads each cluster directly from this server, with the credentials in the kubeconfig it was started with. Asking the console about a cluster in plain language is not in this design yet.",
        ),
      ).toBeTruthy();
    });
  });
});
