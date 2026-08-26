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
 * **The route the `Add connection` control opens — and the seam this screen
 * leaves for it.**
 *
 * `/connect` is already in `lib/routes`'s app-scoped table (it has a title and
 * a tab kind), and it has no entry in that file's `SCREENS` map yet, so opening
 * it today lands on the `Placeholder`. That is deliberate: the tab, its title
 * and both ways in exist and are tested from here, and the next task adds one
 * line — `"/connect": Connect` in `SCREENS` — with nothing on this screen to
 * change. An import of a component that does not exist would not compile, and
 * a route invented here would be a second name for the one that table holds.
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

  /** Provider and region per cluster, from the second round trip. */
  const [facts, setFacts] = useState<Record<string, ClusterFacts>>({});
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
   * Which round of readings the work in flight belongs to.
   *
   * The probes and the facts are per cluster and land one at a time, so a round
   * that has been superseded — the reader re-listed while it was out — must not
   * write what it finds. Bumped once per run of the effect below, which is once
   * per listing.
   */
  const readSeq = useRef(0);

  /**
   * The clusters being read right now.
   *
   * **This is what keeps two unordered writes of one cluster's reading from
   * happening at all.** `probeCluster` writes to a module store this screen
   * cannot un-write, so a sequence check after the fact could not undo a stale
   * answer — a 30-second timeout from the first round would land on top of the
   * second round's `12 ms` and the row would say `unreachable` about a cluster
   * that had answered. A reading already on its way IS the fresh reading, so a
   * second one is not started for it.
   */
  const reading = useRef<Set<string>>(new Set());

  /** The clusters whose facts have been asked for in this round. */
  const factsAsked = useRef<Set<string>>(new Set());

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
    if (force) factsAsked.current.clear();
    const seq = ++readSeq.current;

    async function read(context: ClusterContext) {
      const id = context.stableId;
      // Its own reader will follow through to the facts — see `reading`.
      if (reading.current.has(id)) return;
      if (force || getProbe(id).state === "unread") {
        reading.current.add(id);
        try {
          await probeCluster(context);
        } finally {
          reading.current.delete(id);
        }
        if (seq !== readSeq.current) return;
      }
      if (getProbe(id).state !== "reachable") return;
      if (factsAsked.current.has(id)) return;
      factsAsked.current.add(id);
      // `clusterFacts` never rejects; a failure comes back as empty facts plus
      // a reason, and the cell simply has nothing to add to the row.
      const answer = await clusterFacts(context.name);
      if (seq !== readSeq.current) return;
      setFacts((current) => ({ ...current, [id]: answer }));
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
   * **Absent until there is something to count.** A number over a spinner, an
   * error card or the `/connect` nudge would be a count of nothing asserted as
   * a fact — the same fault as a count that disagrees with its rows, which is
   * what `Releases · 383 in this cluster` over six rows was.
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
      <Screen title={title} fill actions={actions}>
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
      <Screen title={title} fill actions={actions}>
        <LoadingState label="Loading clusters" className="flex-1" />
      </Screen>
    );
  }

  // Nothing listed and a reason for it. The rows are the only thing this screen
  // is about, so with none of them the failure takes the whole body.
  if (rows.length === 0) {
    return (
      <Screen title={title} fill actions={actions}>
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
