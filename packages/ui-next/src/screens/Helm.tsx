import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import {
  ageFromTimestamp,
  helmStatus,
  listHelmReleases,
  type ClusterContext,
  type HelmReleaseSummary,
} from "@srelens/core";
import { Button, LoadingState, Screen, StatusPill, Table, type Column } from "@srelens/ui-kit";
import { useConsole } from "../console";
import { getKubeconfigFiles, useActiveContext } from "../lib/clusters";
import { FailureState } from "../lib/errorCopy";
import { getHelmOps, subscribeHelmOps, type HelmOpKind } from "../lib/helmOps";
import { Icons } from "../lib/icons";
import { describe } from "../lib/routes";
import { HelmOpDialog } from "./helm/HelmOpDialog";
import { ReleasePane, type PaneRelease } from "./helm/ReleasePane";
import { NoClusterScreen } from "./resourceShell";

/**
 * §16's header ask: the visible word, and the question actually sent.
 *
 * The question names the release the reader is looking at — §16 writes it for
 * its own fixture (`What did checkout release 119 change?`) and the shape is
 * what generalises. With nothing selected there is no release to name, so the
 * question is about the cluster instead rather than about a release the reader
 * has not chosen.
 */
const EXPLAIN_LABEL = "Explain";

/**
 * The name §16's `Install chart` opens its dialog on.
 *
 * A PROP, never a blank field. `HelmOpDialog` takes the release name as a
 * prop in all four modes, and its uninstall gate compares what was typed to
 * `release` with an explicit `release !== ""` guard — an empty name is a gate
 * that can never be passed. There is no install-mode field for it either, so a
 * blank here would build `helm install "" <chart>`.
 */
const NEW_RELEASE = "new-release";

/** One row of §16's release table, every cell already resolved to what it draws. */
interface ReleaseRow {
  /** Helm scopes a release name to a namespace, so both are the identity. */
  key: string;
  name: string;
  namespace: string;
  /** `ingress-nginx-4.12.0` — §16's Chart cell, which is the two fields joined. */
  chart: string;
  /** The chart's own name, for the dialog's Chart field. */
  chartName: string;
  chartVersion: string;
  revision: number;
  /** Helm's own status word, untouched. Toned by core, never by this file. */
  status: string;
  /** `9d ago`, or `—` when the backend gave no timestamp. */
  updated: string;
  /** The raw stamp, so the column sorts as time rather than as text. */
  updatedAt: string;
}

/** What the dialog is about, decided when it is opened and not before. */
interface Pending {
  kind: HelmOpKind;
  release: string;
  namespace: string;
  chart: string;
  chartVersion: string;
  /** The revision running now, for rollback's own arithmetic. */
  revision?: number;
}

/** Where the release listing stands. */
type ListLoad =
  | { status: "loading" }
  | { status: "ready"; releases: HelmReleaseSummary[]; at: number }
  | { status: "error"; error: string };

/**
 * `/helm` — the design's Helm screen (§16).
 *
 * Split in two the way `Events.tsx` is: with no cluster in focus there is no
 * context name to list against, and a hook cannot be skipped, so the guard is
 * a `return` before any hook runs.
 */
export function Helm({ route }: { route: string }) {
  const context = useActiveContext();
  const title = describe(route, context?.name).title;

  if (!context) {
    return <NoClusterScreen title={title} noun="Helm releases" />;
  }

  return <HelmReleases title={title} context={context} />;
}

/**
 * §16's two panes: the release table, and the fixed 420px pane beside it.
 *
 * **Everything this screen knows about a release comes from
 * `listHelmReleases`.** It makes no `getHelmRelease` call of its own — the
 * pane makes the two the diff needs, and nothing here would be improved by a
 * third. That is also why the rollback dialog is opened with no `history`: the
 * dialog was built to degrade honestly without one ("srelens has no history
 * for this release, so name the revision yourself"), and fetching a release's
 * whole manifest, values and history to pre-fill one number is a round trip
 * per selection for a field the reader can type.
 *
 * **No status word or tone is invented here.** `helmStatus` is the only thing
 * that turns Helm's word into a tone, on the rows and in the pane's badge
 * alike. Task 2 exists so this screen is not the fourth to keep its own table.
 *
 * **The pane is dropped in beside the table with no wrapper.** `ReleasePane`
 * owns its own 420px frame — `aside.side-rail`, head, body and footer — so a
 * `SideRail` around it would be a second rail around the first.
 */
function HelmReleases({ title, context }: { title: string; context: ClusterContext }) {
  // Core takes a context *name*; the workspace holds a `stableId`. The two are
  // never interchangeable — see `lib/clusters`.
  const name = context.name;
  const files = getKubeconfigFiles();
  const { ask } = useConsole();

  const ops = useSyncExternalStore(subscribeHelmOps, getHelmOps, getHelmOps);

  const [list, setList] = useState<ListLoad>({ status: "loading" });
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
  const [pending, setPending] = useState<Pending | null>(null);

  /**
   * List the cluster's releases.
   *
   * Stable across renders — this screen re-renders on every line a running
   * `helm upgrade` prints, and a handler rebuilt each time would re-fire the
   * effects that depend on it.
   */
  const reload = useCallback(async () => {
    const out = await listHelmReleases(name);
    if (out.error !== undefined || !out.releases) {
      setList({ status: "error", error: out.error ?? "helm returned no releases" });
      return;
    }
    setList({ status: "ready", releases: out.releases, at: Date.now() });
  }, [name]);

  useEffect(() => {
    setList({ status: "loading" });
    setSelectedKey(null);
    void reload();
  }, [reload]);

  /**
   * **Re-list once an operation settles**, so the pane stops diffing the pair
   * from before it.
   *
   * A successful `helm upgrade` moves the release to a new revision, and the
   * pane's diff is `revision - 1 → revision` off the row in this list: without
   * this, an upgrade that landed leaves the reader looking at the diff of the
   * revision BEFORE the one they just created, under a status word that is
   * also stale.
   *
   * Every settled operation, not only the successful ones — a failed
   * `helm upgrade` still writes a revision, with `failed` on it, and that is
   * exactly the row a reader needs to see change.
   *
   * Keyed on ids already acted on rather than on the settled ids themselves:
   * dismissing a finished row changes the set of settled operations without
   * anything having happened in the cluster, and a listing per dismissal is a
   * round trip nobody asked for.
   */
  const relisted = useRef(new Set<number>());
  useEffect(() => {
    const settled = ops.filter((o) => o.state !== "running" && !relisted.current.has(o.id));
    if (settled.length === 0) return;
    for (const o of settled) relisted.current.add(o.id);
    void reload();
  }, [ops, reload]);

  const releases = list.status === "ready" ? list.releases : [];
  const at = list.status === "ready" ? list.at : 0;

  const rows = useMemo<ReleaseRow[]>(
    () =>
      releases.map((r) => {
        const age = ageFromTimestamp(r.updated, at);
        return {
          key: `${r.namespace}/${r.name}`,
          name: r.name,
          namespace: r.namespace,
          // §16's cell, and what classic's Helm list has always shown.
          chart: `${r.chart}-${r.chartVersion}`,
          chartName: r.chart,
          chartVersion: r.chartVersion,
          revision: r.revision,
          status: r.status,
          // `— ago` is not a sentence: a release with no timestamp has an
          // unknown age, not an age of nothing.
          updated: age === "—" ? age : `${age} ago`,
          updatedAt: r.updated,
        };
      }),
    [releases, at],
  );

  /**
   * The release the pane is about — looked up in the CURRENT list rather than
   * held in state.
   *
   * This is what makes the revision refresh above reach the pane: the
   * selection is a key, and the numbers behind it are whatever the last
   * listing said. A `PaneRelease` copied into state at click time would still
   * carry the revision the release had when it was clicked.
   */
  const selected = useMemo<PaneRelease | null>(() => {
    const row = rows.find((r) => r.key === selectedKey);
    if (!row) return null;
    return {
      name: row.name,
      namespace: row.namespace,
      revision: row.revision,
      status: row.status,
    };
  }, [rows, selectedKey]);

  const question = selected
    ? `What did ${selected.name} release ${selected.revision} change?`
    : `Which Helm releases on ${name} need attention?`;

  /** Open the dialog on a row, and follow that row in the pane. */
  function operate(kind: HelmOpKind, row: ReleaseRow) {
    setSelectedKey(row.key);
    setPending({
      kind,
      release: row.name,
      namespace: row.namespace,
      // The chart and version the release is ON. §16's own upgrade path is the
      // pane's `Values editor`, which re-renders the SAME chart with new
      // values, so the current pair is the right place for both fields to
      // start; a reader moving version changes one number.
      chart: row.chartName,
      chartVersion: row.chartVersion,
      revision: row.revision,
    });
  }

  /**
   * §16's pane footer: roll back to the diff's left-hand revision.
   *
   * The number the pane hands over is `revision - 1` — the revision the diff
   * on screen is comparing against, which is what makes the button read
   * `Roll back to 118`. **It is deliberately not forced into the dialog.**
   * The dialog's own default target is `lastGoodRevision`: the newest revision
   * helm does not report as failed or unfinished, which on a release whose
   * last two upgrades failed is a different and better number. They answer
   * different questions — "what am I looking at" and "what is safe to return
   * to" — and reconciling them would mean one of the two lying.
   */
  function rollbackFromPane() {
    const row = rows.find((r) => r.key === selectedKey);
    if (row) operate("rollback", row);
  }

  function valuesEditor() {
    const row = rows.find((r) => r.key === selectedKey);
    if (row) operate("upgrade", row);
  }

  const columns: Column<ReleaseRow>[] = [
    {
      key: "name",
      header: "Release",
      render: (row) => <span className="block truncate font-medium">{row.name}</span>,
    },
    {
      key: "namespace",
      header: "Namespace",
      render: (row) => <span className="path block truncate">{row.namespace}</span>,
    },
    {
      key: "chart",
      header: "Chart",
      /**
       * §16 truncates this at 200px, and the cap is what keeps the 420px pane
       * on the window.
       *
       * A cell's intrinsic width is its content's, and `truncate` alone does
       * not change that — `white-space: nowrap` makes the min-content width the
       * whole string. `max-w-[200px]` is what caps the contribution, and
       * `block` is what makes `overflow: hidden` apply at all: `truncate` on an
       * inline box does nothing. A chart named
       * `kube-prometheus-stack-67.2.0` is already past 200px, and jsdom sees
       * none of this — hence the class assertion in the suite.
       */
      render: (row) => (
        <span data-slot="chart-name" className="path block max-w-[200px] truncate">
          {row.chart}
        </span>
      ),
    },
    {
      key: "revision",
      header: "Rev",
      align: "end",
      render: (row) => <span className="tabular-nums">{row.revision}</span>,
    },
    {
      key: "status",
      header: "Status",
      /**
       * Core's verdict, word and tone. `tinted` is the design's asymmetric
       * colouring rule — §16 draws `deployed` plain and everything worse
       * coloured and bold — and `StatusPill` owns which kinds that covers, so
       * this file decides nothing about it.
       */
      render: (row) => {
        const v = helmStatus(row.status);
        return <StatusPill status={v.word} kind={v.health} tinted />;
      },
      // Sorted and searched on the word the reader can see.
      getValue: (row) => helmStatus(row.status).word,
    },
    {
      key: "updated",
      header: "Updated",
      align: "end",
      // The stamp, not the compact age: `2h` sorts below `51m` as text.
      getSortValue: (row) => row.updatedAt,
      render: (row) => <span className="tabular-nums text-muted">{row.updated}</span>,
    },
    {
      // §16's unnamed trailing column: the three operations, in its order.
      key: "actions",
      header: "",
      sortable: false,
      filterable: false,
      align: "end",
      minWidth: 210,
      render: (row) => (
        <div className="flex items-center justify-end gap-1">
          {/* Named per row. Six rows all offering "Upgrade" name nothing at
              all; the accessible name carries the release, which the row
              already shows, so nothing is hidden in it. */}
          <Button
            size="xs"
            variant="secondary"
            aria-label={`Upgrade ${row.name}`}
            onClick={() => operate("upgrade", row)}
          >
            Upgrade
          </Button>
          <Button
            size="xs"
            variant="secondary"
            aria-label={`Roll back ${row.name}`}
            onClick={() => operate("rollback", row)}
          >
            Roll back
          </Button>
          <Button
            size="xs"
            variant="danger"
            aria-label={`Uninstall ${row.name}`}
            onClick={() => operate("uninstall", row)}
          >
            Uninstall
          </Button>
        </div>
      ),
    },
  ];

  const Sparkle = Icons.ask;

  return (
    <Screen
      title={title}
      // §16's sub. The releases are this cluster's; the word says which list
      // the title is about.
      eyebrow={`${name} / releases`}
      fill
      actions={
        <>
          {/* A `Button`, not `AskChip`. `.row-ask` is `opacity: 0` until a
              `.tbl tbody tr` is hovered and a header has no row to hover —
              `Events.tsx` and `Overview.tsx` both say so, and Logs shipped an
              invisible one until #352. The visible word is §16's; the question
              that will actually be sent is the accessible name, which is the
              same split the chip itself makes. */}
          <Button
            type="button"
            size="sm"
            aria-label={`${EXPLAIN_LABEL}: ${question}`}
            title={`${EXPLAIN_LABEL}: ${question}`}
            onClick={() => ask(question)}
          >
            <Sparkle size={12} aria-hidden="true" />
            {EXPLAIN_LABEL}
          </Button>
          <Button
            variant="primary"
            size="sm"
            onClick={() =>
              setPending({
                kind: "install",
                release: NEW_RELEASE,
                // The context's own default namespace, which is where `helm
                // install` with no `--namespace` would have gone anyway.
                namespace: context.namespace || "default",
                chart: "",
                chartVersion: "",
              })
            }
          >
            Install chart
          </Button>
        </>
      }
    >
      {/* Beside the panes rather than inside either, so it opens the same from
          a row, from the pane's footer and from the header. */}
      {pending && (
        <HelmOpDialog
          kind={pending.kind}
          context={name}
          namespace={pending.namespace}
          release={pending.release}
          chart={pending.chart}
          chartVersion={pending.chartVersion}
          revision={pending.revision}
          extraKubeconfigs={files}
          onClose={() => setPending(null)}
        />
      )}

      <div className="flex min-h-0 flex-1">
        {/* `min-w-0`, and it is load-bearing. A flex item's implicit
            `min-width: auto` refuses to shrink below its content, so without
            it a table with a long chart name pushes the 420px pane off the
            window and the whole screen scrolls sideways. Seven defects on this
            migration, none of them visible in jsdom. */}
        <div data-slot="release-main" className="flex min-h-0 min-w-0 flex-1 flex-col">
          <div className="pane-head">
            <span className="min-w-0 truncate">
              {list.status === "ready"
                ? `Releases · ${rows.length} in this cluster`
                : "Releases"}
            </span>
          </div>
          <div className="scroll min-h-0 min-w-0 flex-1">
            {list.status === "loading" ? (
              <LoadingState label="Listing Helm releases" />
            ) : list.status === "error" ? (
              <FailureState
                title={`Could not list Helm releases on ${name}`}
                error={list.error}
                onRetry={() => void reload()}
              />
            ) : (
              <Table
                columns={columns}
                data={rows}
                getRowKey={(row) => row.key}
                selectedKey={selectedKey ?? undefined}
                // §16's own pane is hard-wired to one release and says a row
                // click does nothing. This one follows the selection, which is
                // the only way the diff and the operation output can be about
                // the release the reader is asking about.
                onRowClick={(row) => setSelectedKey(row.key)}
                onRowActivate={(row) => setSelectedKey(row.key)}
                // A cluster with no releases is ordinary, not a failure: there
                // is no filter on this screen to blame and nothing to clear.
                emptyText="No Helm releases"
                emptyHint={`${name} has no Helm releases.`}
              />
            )}
          </div>
        </div>

        {/* No wrapper: `ReleasePane` is its own `aside.side-rail` at 420px.
            `ops` is the store's snapshot passed straight through — filtering or
            mapping it here would hand `useSyncExternalStore` a fresh array per
            render, which is "Maximum update depth exceeded" rather than a waste.
            No `invoke` either: core's own default is a module constant, and an
            inline one would be a new identity every render on a screen that
            re-renders for every line a running upgrade prints — re-firing both
            of the pane's `getHelmRelease` round trips each time. */}
        <ReleasePane
          context={name}
          release={selected}
          ops={ops}
          onRollback={rollbackFromPane}
          onValuesEditor={valuesEditor}
        />
      </div>
    </Screen>
  );
}
