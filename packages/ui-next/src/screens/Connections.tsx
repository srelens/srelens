import { useEffect, useMemo, useRef, useState } from "react";
import {
  clusterFacts,
  isTauri,
  listContexts,
  pickKubeconfigFiles,
  plural,
  saveKubeconfigFiles,
  type ClusterContext,
  type ClusterFacts,
} from "@srelens/core";
import { Button, EmptyState, LoadingState, Screen } from "@srelens/ui-kit";
import {
  getContexts,
  getKubeconfigFiles,
  setContexts,
  setKubeconfigFiles,
  useContexts,
  useContextsError,
  useContextsStatus,
} from "../lib/clusters";
import { FailureAlert, FailureState } from "../lib/errorCopy";
import { getProbe, probeCluster, useProbes, type Probe } from "../lib/probe";
import { describe } from "../lib/routes";
import { openTab, setActiveCluster, setWorkspaceClusters, useTabs } from "../lib/tabsStore";
import { ClusterTable, type ClusterRow } from "./connections/ClusterTable";
import { SourcesRail } from "./connections/SourcesRail";

/**
 * **The route the `Add connection` control opens.**
 *
 * `/connect` is in `lib/routes`'s app-scoped table, which gives it a title and
 * a tab kind, and in that file's `SCREENS` map, which is what renders the
 * first-run card rather than the `Placeholder`. Both entries live there and
 * nothing about them is restated here: a route invented in this file would be a
 * second name for the one that table holds.
 */
const CONNECT = "/connect";

/**
 * A cluster nothing has read yet.
 *
 * Module-level so the identity is stable: it is spread across every row whose
 * probe the store has no entry for, and a fresh object per render would defeat
 * the `useMemo` below on every notification.
 */
const UNREAD: Probe = { state: "unread" };

/**
 * `/connections` — §6's screen: every cluster srelens can see, what the last
 * reading of it said, and which file each one was read from.
 *
 * **The two panes are components, and this composes them.** `ClusterTable` and
 * `SourcesRail` fetch nothing and hold no state; this screen owns the contexts,
 * the probes, the facts and the stored file list, which is what lets both panes
 * be drawn from a fixture. The rail draws its own 292px `aside`, so it is
 * dropped in beside the table with no wrapper — the `ReleasePane` arrangement
 * `Helm` uses.
 *
 * **The contexts come from the store, not from a listing of this screen's
 * own.** `Window` lists them at boot and every screen reads that one answer, so
 * arriving here draws the table immediately rather than re-asking for a list
 * the window already has. `Refresh all` is the one thing that lists again, and
 * it writes the answer back through `setContexts` so the rail, the status bar
 * and every other screen see the same list this one is drawing.
 *
 * **Four things this screen is careful about**, each of which has shipped as a
 * defect on a screen in this migration:
 *
 * 1. The sub-count counts what is on the screen — see the `sub` below.
 *    `Releases · 383 in this cluster` over six rows is the fault, and both
 *    halves here are read from the same arrays the two panes are given.
 * 2. A probe never blocks a row. Every cluster is drawn `unread` on the first
 *    paint and each reading arrives on its own — twenty clusters do not queue,
 *    and one that never answers holds up none of the others.
 * 3. A late answer cannot paint over a fresh one, on both round trips. Helm's
 *    `listSeq` idiom, twice: once on the listing and once on the readings.
 * 4. `onAddFile` is passed whenever there is a filesystem to browse. The rail
 *    reads its absence as "there is none" and prints that instead of the
 *    control, so forgetting it here would lose the button on the desktop with
 *    nothing on screen saying why.
 */
export function Connections({ route }: { route: string }) {
  // The routes table's own title, so the tab strip and the `h1` cannot drift.
  const title = describe(route).title;

  const contexts = useContexts();
  const status = useContextsStatus();
  const listError = useContextsError();
  const probes = useProbes();
  /**
   * The kubeconfig paths this window was started with, read at render the way
   * `Helm` reads them.
   *
   * Not reactive, and it does not need to be: the only thing that changes it is
   * {@link addFile} below, which re-lists in the same breath — and the listing
   * is what re-renders this screen with the new list in hand.
   */
  const files = getKubeconfigFiles();
  const { workspace } = useTabs();

  /**
   * Provider and region per cluster, from the second round trip, **keyed by
   * `stableId`**.
   *
   * The same key the probes are under and the same key the rows read. Not the
   * context NAME, which is what `clusterFacts` is CALLED with: in production a
   * stableId is the declaring file plus the name (it gains that prefix as soon
   * as two kubeconfigs declare the same context), so keying the answers by name
   * would file every cluster's facts under something no row looks up and empty
   * the second line of every row on every platform. The suite carries a fixture
   * whose stableId differs from its name so the two cannot be conflated.
   */
  const [facts, setFacts] = useState<Record<string, ClusterFacts>>({});
  /**
   * The same record, readable synchronously.
   *
   * The decision "do we already have this cluster's facts" is made inside an
   * async read, where the `facts` state variable is whatever it was when that
   * read started. `putFacts` is the only writer of either.
   */
  const known = useRef<Record<string, ClusterFacts>>({});
  /** A listing asked for by the reader, still out. */
  const [busy, setBusy] = useState(false);
  /** Why a kubeconfig file could not be added, when one could not. */
  const [addError, setAddError] = useState<unknown>(null);

  /**
   * Which listing is the current one — `Helm`'s `listSeq`, for the same reason.
   *
   * A reader who hits `Refresh all` twice must be left looking at the second
   * answer whatever order the two come back in. Without this, the first
   * listing's late answer paints over the second's and the table shows a list
   * the reader has already moved on from.
   */
  const listSeq = useRef(0);

  /**
   * **Every guard below is per CLUSTER, and that is the whole design.**
   *
   * There was a round counter here, bumped once per listing, and it was the
   * wrong unit for the work: a probe and a facts read belong to one cluster,
   * and a re-listing says the LIST may have changed — not that a reading of a
   * cluster still on it is worthless. Judging per-cluster work by a round
   * number cost the facts outright. `listContexts` is a local file read and
   * answers in a millisecond, so pressing `Refresh all` while twenty clusters
   * are still connecting started a round that skipped every one of them (a read
   * was already out) and abandoned every reader of the round before it (its
   * round number had moved). Twenty rows settled `reachable` with no provider
   * and no region, permanently, until a second `Refresh all` after the probes
   * had landed. Nothing on screen said so — a row with no facts line is the
   * ordinary first paint.
   *
   * It also made the screen depend on an invariant the store knows nothing
   * about: "every write to the contexts store comes paired with `forceNext`".
   * Any other writer — `Window` re-listing after a kubeconfig change — re-ran
   * the effect, dropped whatever was in flight and then refused to ask again.
   *
   * So there is no round counter. A read is judged by two questions that are
   * both about the cluster: is there already a read of THIS cluster out (join
   * it), and is this the newest read of THIS cluster (only then may it write).
   */

  /**
   * The facts read in flight per cluster, so every other reader joins it.
   *
   * **This is the whole ordering guard, and it works by there never being two.**
   * The same shape as `probeCluster`'s own join, for the same reason: two reads
   * of one cluster answering out of order would draw the older answer over the
   * newer one, and no check after the fact can put that right. So a second read
   * of a cluster is not started — a reader that wants the facts joins the read
   * already out and gets its answer. With at most one read per cluster in
   * flight, the writes cannot arrive out of order and there is nothing left for
   * a sequence number to guard. (There was one here. It could not fire, and an
   * unfireable guard is worse than none: it reads as protection.)
   */
  const factsOut = useRef<Map<string, Promise<void>>>(new Map());

  /** The one writer of the facts, state and synchronous mirror together. */
  function putFacts(id: string, answer: ClusterFacts) {
    known.current = { ...known.current, [id]: answer };
    setFacts(known.current);
  }

  /**
   * Whether the next round re-reads every cluster or only the unread ones.
   *
   * Arriving at this screen respects what the store already knows — `Window`
   * probes the workspace's clusters at launch, and re-reading twenty clusters
   * every time the reader flips back to this tab is a round trip per cluster
   * for an answer already in hand. `Refresh all` sets this, and then every
   * cluster is read again: that is the whole of what the control promises.
   */
  const forceNext = useRef(false);

  /**
   * List the contexts again, and read every cluster on the new list.
   *
   * The answer is written back through `setContexts`, so this screen is not
   * keeping a second copy of the window's cluster list.
   */
  async function reload() {
    const seq = ++listSeq.current;
    setBusy(true);
    const outcome = await listContexts(getKubeconfigFiles());
    // Superseded. Dropped whole — the list AND the reason it is short are one
    // fact, and installing half of a stale answer is worse than installing none.
    if (seq !== listSeq.current) return;
    setBusy(false);
    forceNext.current = true;
    /**
     * **A fresh array, deliberately.**
     *
     * It is what tells the effect below that a listing has happened, and the
     * two cases that need saying are the ones where the members alone do not
     * say it: a listing that came back with the same contexts, and a listing
     * that FAILED — where the rows already on screen are kept rather than
     * thrown away, because a refresh that could not be made took nothing away
     * from the reader. The store's own writer installs the list and the reason
     * together, so both arrive in one write and no render can catch one without
     * the other.
     */
    setContexts([...(outcome.contexts ?? getContexts())], outcome.error ?? "");
  }

  /**
   * Read every cluster on the current list, each on its own.
   *
   * **Nothing here is awaited in series and nothing gates the render.** The
   * table is drawn from `contexts` the moment they exist, with every row
   * `unread`, and each reading arrives as its own store notification. A cluster
   * that never answers leaves its own row saying `no reading` and delays no
   * other row.
   *
   * The facts are the second round trip, and only for a cluster that answered:
   * `provider` and `region` come from the API server, so asking an unreachable
   * cluster for them buys a second timeout for a row that already says why it
   * is empty.
   */
  useEffect(() => {
    const force = forceNext.current;
    forceNext.current = false;

    async function read(context: ClusterContext) {
      const id = context.stableId;
      /**
       * **Awaited, never skipped.**
       *
       * `probeCluster` joins the read already out for this cluster rather than
       * starting a second (see its own note — the rule has to live there
       * because `Window` probes too and neither caller can see the other's
       * guard). So this `await` resolves when the reading is IN, whether this
       * call took it or joined it, and the facts below are asked for either
       * way. Walking away from a cluster whose read was already out is what
       * left the facts unfetched whenever a read spanned two listings.
       */
      if (force || getProbe(id).state === "unread") await probeCluster(context);
      // Not reachable: `provider` and `region` come from the API server, so
      // asking buys a second timeout for a row that already says why it is
      // empty. A later reading that DOES answer comes back through here.
      if (getProbe(id).state !== "reachable") return;

      const out = factsOut.current.get(id);
      /**
       * A read already out is the answer arriving; joining it is what stops a
       * second listing asking again for what the first is already fetching.
       *
       * **Joined even by a forced read**, which is not obvious. A facts read in
       * flight was started when this cluster's reading landed, moments ago —
       * there is no such thing as a stale one, so `Refresh all` has nothing to
       * gain from a second round trip and the reader would pay for twenty of
       * them. What `force` overrides is the line below: facts already IN HAND
       * are re-read, because those can be as old as the window.
       */
      if (out) return out;
      if (!force && known.current[id] !== undefined) return;

      const run = (async () => {
        // `clusterFacts` never rejects; a failure comes back as empty facts
        // plus a reason, and the cell simply has nothing to add to the row.
        const answer = await clusterFacts(context.name);
        // Keyed by stableId — see `facts`. `clusterFacts` takes the NAME.
        putFacts(id, answer);
      })();
      // Set before this reader yields, which is what makes the join above
      // reliable rather than a matter of scheduling luck.
      factsOut.current.set(id, run);
      try {
        await run;
      } finally {
        factsOut.current.delete(id);
      }
    }

    for (const context of contexts) void read(context);
  }, [contexts]);

  /** The rows both panes are drawn from — the one array, so they cannot differ. */
  const rows = useMemo<ClusterRow[]>(
    () =>
      contexts.map((context) => {
        const probe = probes[context.stableId] ?? UNREAD;
        const known = facts[context.stableId];
        return known ? { context, probe, facts: known } : { context, probe };
      }),
    [contexts, probes, facts],
  );

  /**
   * How many files the rail is listing — **its rule, not the stored list's
   * length.**
   *
   * `files` is not the whole truth about where clusters are read from: web mode
   * stores none at all (`Window.tsx` hands it `[]`), so counting it would put
   * `0 sources` over a rail drawing one row for the kubeconfig twelve contexts
   * came out of. And a stored path that yielded nothing is still a source the
   * rail lists, so counting only the contexts' files would be short the other
   * way.
   *
   * This mirrors `SourcesRail`'s own `fileRows`: every stored path, plus every
   * path a context came from, deduped by exact string, ignoring the empty one a
   * synthesized cluster carries. The suite pins the two together by counting
   * the rows the rail actually drew rather than by re-deriving the number, so a
   * drift between here and there fails a test instead of quietly putting a
   * count over rows that disagree with it.
   */
  const sources = useMemo(() => {
    const paths = new Set<string>();
    for (const path of files) if (path.trim() !== "") paths.add(path);
    for (const row of rows) if (row.context.sourceFile.trim() !== "") paths.add(row.context.sourceFile);
    return paths.size;
  }, [files, rows]);

  /**
   * §6's sub: `<n> clusters · <n> sources`.
   *
   * **Absent until there is something to count, and this guard is what makes it
   * so.** A number over a spinner, an error card or the `/connect` nudge would
   * be a count of nothing asserted as a fact — the same fault as a count that
   * disagrees with its rows, which is what `Releases · 383 in this cluster`
   * over six rows was.
   *
   * Every `Screen` below is handed `eyebrow={sub}`, deliberately: the rule
   * lives here, in one expression, rather than in which of four call sites
   * remembered to leave the prop off. It was the other way round and the guard
   * was dead code — the early returns were doing the work, so removing this
   * `rows.length > 0` cost nothing and no test noticed.
   */
  const sub =
    rows.length > 0 ? `${plural(rows.length, "cluster")} · ${plural(sources, "source")}` : undefined;

  /**
   * Open a cluster: put it in this workspace, focus it, and open its overview.
   *
   * The workspace step is not a flourish. This screen lists every context on
   * the machine, including ones no workspace holds, and `setActiveCluster`
   * refuses an id the workspace does not have — so without it `Open` on exactly
   * those rows would do nothing at all, silently.
   */
  function open(stableId: string) {
    const context = contexts.find((c) => c.stableId === stableId);
    if (!context) return;
    if (!workspace.clusters.includes(stableId)) {
      setWorkspaceClusters(workspace.id, [...workspace.clusters, stableId]);
    }
    setActiveCluster(stableId);
    openTab("/overview", { clusterName: context.name });
  }

  /**
   * Browse for a kubeconfig file, remember it, and list again.
   *
   * Three writes, and all three are needed: `saveKubeconfigFiles` is what makes
   * the file survive a restart, `setKubeconfigFiles` is what every core call in
   * this window reads (the backend cannot build a client for a context from a
   * file it has not been told about), and the listing is what puts that file's
   * contexts on the screen.
   */
  async function addFile() {
    setAddError(null);
    let picked: string[];
    try {
      picked = await pickKubeconfigFiles();
    } catch (error) {
      // Through `describeError` where it is shown, like every other failure on
      // this screen — nothing is written when the picker itself refused.
      setAddError(error);
      return;
    }
    if (picked.length === 0) return;
    const next = [...new Set([...getKubeconfigFiles(), ...picked])];
    saveKubeconfigFiles(next);
    setKubeconfigFiles(next);
    await reload();
  }

  /**
   * The desktop's own question, asked once, here.
   *
   * The rail takes a callback rather than asking itself, so both of its states
   * are reachable from a fixture — `Toolbox.tsx:188` makes the same split for
   * its install column.
   */
  const desktop = isTauri();

  const actions = (
    <>
      <Button
        type="button"
        variant="secondary"
        size="sm"
        // Not disabled while a listing is out. A reader who asks twice is
        // exactly the case `listSeq` exists for, and a control that locks
        // itself for the length of an unreachable cluster's timeout is a
        // control the reader cannot use when they most want to.
        aria-busy={busy || undefined}
        onClick={() => void reload()}
      >
        Refresh all
      </Button>
      <Button type="button" variant="primary" size="sm" onClick={() => openTab(CONNECT)}>
        Add connection
      </Button>
    </>
  );

  /**
   * A reader with no clusters is sent to `/connect`, not left at an empty
   * table.
   *
   * The table's own `emptyText` covers the narrow case of a list that came back
   * short; this is the first-run one, where an empty panel with six column
   * headers over it says only that srelens has nothing and not what to do next.
   */
  if (status === "loaded" && rows.length === 0) {
    return (
      <Screen title={title} eyebrow={sub} fill actions={actions}>
        <EmptyState
          className="flex-1"
          title="No clusters yet"
          hint="srelens reads your kubeconfig files in place. Connect a cluster and it appears here, with the file it came from beside it."
          action={
            <Button type="button" variant="primary" size="sm" onClick={() => openTab(CONNECT)}>
              Connect a cluster
            </Button>
          }
        />
      </Screen>
    );
  }

  if (status === "loading" && rows.length === 0) {
    return (
      <Screen title={title} eyebrow={sub} fill actions={actions}>
        <LoadingState label="Loading clusters" className="flex-1" />
      </Screen>
    );
  }

  // Nothing listed and a reason for it. The rows are the only thing this screen
  // is about, so with none of them the failure takes the whole body.
  if (rows.length === 0) {
    return (
      <Screen title={title} eyebrow={sub} fill actions={actions}>
        <FailureState
          className="my-auto"
          title="Could not list your clusters"
          error={listError}
          onRetry={() => void reload()}
        />
      </Screen>
    );
  }

  return (
    <Screen title={title} eyebrow={sub} fill actions={actions}>
      {addError !== null && (
        // `sev`: the reader asked for a file to be added and it was not. This
        // is not a remark about the rows below it.
        <FailureAlert
          tone="sev"
          className="mx-3 mt-3"
          title="Could not add that kubeconfig file"
          error={addError}
        />
      )}

      {/* A refresh that failed over rows that are still there — a warning, not
          a stop, per `errorCopy`'s rule. The rows are real; what this says is
          that they may be older than the reader just asked for. */}
      {status === "failed" && (
        <FailureAlert className="mx-3 mt-3" title="Could not refresh your clusters" error={listError} />
      )}

      <div className="flex min-h-0 flex-1">
        {/* `min-w-0`, and it is load-bearing. A flex item's implicit
            `min-width: auto` refuses to shrink below its content, so without it
            a table carrying a 70-character kubeconfig path pushes the rail's
            fixed 292px off the window and the whole screen scrolls sideways.
            Eight defects on this migration, and jsdom sees none of them — hence
            the class assertion in the suite. */}
        <div data-slot="connections-main" className="flex min-h-0 min-w-0 flex-1 flex-col">
          {/* The scroll container is the caller's, which is this: `ClusterTable`
              draws no box of its own, the way `Helm` hands its own table a
              `scroll min-h-0 flex-1` wrapper. */}
          <ClusterTable rows={rows} onOpen={open} className="scroll min-h-0 min-w-0 flex-1" />
        </div>

        {/* No wrapper: the rail is its own `aside.side-rail` at 292px. The same
            `rows` the table has, so a reading cannot read one way here and
            another way six inches to the left. */}
        <SourcesRail
          rows={rows}
          files={files}
          // **Passed whenever there is a filesystem to browse.** Absent, the
          // rail says why there is no control rather than drawing a dead one.
          onAddFile={desktop ? () => void addFile() : undefined}
        />
      </div>
    </Screen>
  );
}
