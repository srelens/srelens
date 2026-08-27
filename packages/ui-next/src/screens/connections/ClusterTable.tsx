import { contextDisplayName, plural, type ClusterContext, type ClusterFacts } from "@srelens/core";
import { Badge, Button, Mark, Table, cx, type Column } from "@srelens/ui-kit";
import { useMark } from "../../lib/marks";
import { glyph } from "../../lib/tree";
import type { Probe } from "../../lib/probe";
import { STATUS, bySource, joined, latencyLabel, sourceOf, viaOf } from "./clusterText";

/**
 * One cluster as §6's table draws it: the context, what the last probe said,
 * and the control-plane facts if anything has fetched them yet.
 *
 * `facts` is optional because `clusterFacts` is a second round trip the screen
 * makes per cluster, and the table draws a row before it answers — a cluster
 * whose provider and region are not known yet is the ordinary first paint, not
 * a degraded one.
 */
export interface ClusterRow {
  context: ClusterContext;
  probe: Probe;
  /** Provider and region; absent until fetched. */
  facts?: ClusterFacts;
}

export interface ClusterTableProps {
  rows: readonly ClusterRow[];
  onOpen: (stableId: string) => void;
  className?: string;
}

/**
 * The six things this file used to hold — `joined`, `viaOf`, `latencyLabel`,
 * the `STATUS` table, `sourceOf` and `bySource` — now live in `./clusterText`,
 * because the Sources rail and §24's first-run card render the same facts: a
 * second latency formatter is how the absent-not-zero rule gets lost, a second
 * status table is how one screen starts calling a cluster something the other
 * never would, and a second sort is how `/connect` came to list one set of
 * clusters in a different order from this table. `latencySort` below stays
 * here: it is about how THIS table orders a column, which the rail has no
 * opinion about.
 *
 * The number the `Latency` column SORTS on — never the text beside it.
 */
function latencySort(probe: Probe): number {
  /**
   * A cluster with no reading sorts **after every reading when the column is
   * ascending, and before them when it is reversed** — never as the text `—`
   * happens to collate, which is the thing this exists to prevent.
   *
   * It said "sorts last however the column is turned", and that was wrong.
   * `Table` compares two numbers as `left - right` and then multiplies the
   * result by `-1` for a descending column, so a sentinel that is larger than
   * every reading leads the list when the reader reverses it. Nothing here can
   * change that: a `getSortValue` returns a value, and the direction is applied
   * to the comparison afterwards. Sorting a column of missing values to one end
   * in both directions would be the kit's to offer, and it offers no such
   * thing — so the honest fix is to say what happens. Pinned in both directions
   * in the suite, so the next reader meets the behaviour rather than the claim.
   *
   * `Number.MAX_VALUE` rather than `Number.POSITIVE_INFINITY`, and it matters
   * for the ordinary first paint where several clusters have no reading yet:
   * `Infinity - Infinity` is `NaN`, and two unread rows then compared as NaN.
   * The kit falls through to a stable index compare for any falsy result, so
   * NaN happened to work — by accident, and invisibly. `MAX_VALUE - MAX_VALUE`
   * is `0`, which reaches that same branch on purpose.
   */
  return latencyLabel(probe) === null ? Number.MAX_VALUE : (probe.latencyMs ?? 0);
}

/**
 * §6's `Cluster` cell: the mark, the display name, and the second line.
 *
 * A component rather than inline JSX because it reads the cluster's mark from
 * the marks store, and a hook cannot be called inside a column's `render`
 * callback — `Fleet` makes the same split for the same reason. One component
 * per row is also what keeps each row's subscription its own.
 */
function ClusterCell({ row }: { row: ClusterRow }) {
  const { context, probe, facts } = row;
  const mark = useMark(context.stableId, context.name);

  /**
   * The name the reader gave this context, or the context's own.
   *
   * `contextDisplayName` takes a profile, and ui-next has no profiles store
   * yet — per-cluster appearance here is the marks store, which is a different
   * record — so this resolves to the context name today. It is called anyway,
   * and deliberately: it is the one place a profile has to be handed over when
   * that store arrives, and inlining `context.name` would hide it.
   */
  const name = contextDisplayName(context.name);

  /**
   * §6's second line, from whichever of the three parts exist.
   *
   * `facts.provider` and `facts.region` are `""` when the cluster named none
   * (core says so on `ClusterFacts`), `facts` itself is absent until the fetch
   * lands, and `probe.version` is absent until the cluster answers. So all
   * three can be missing, and when they all are there is no line at all —
   * an empty row of text under a name is a gap a reader tries to read.
   */
  const detail = joined([facts?.provider, probe.version, facts?.region]);

  return (
    // `min-w-0` on the flex row and on the text column inside it. A flex
    // item's implicit `min-width: auto` refuses to shrink below its content,
    // so without these the caps below never engage and a long display name
    // pushes §6's fixed 292px rail off the window. Eight defects on this
    // migration, none of them visible in jsdom — hence the class assertions in
    // the suite. The mark carries its own `shrink-0`.
    <div
      data-testid={`cluster-cell-${context.stableId}`}
      className="flex min-w-0 items-center gap-2"
    >
      <Mark
        name={mark.name}
        short={mark.short}
        color={mark.color}
        size="sm"
        // The row already says which cluster this is, twice — the name beside
        // it and the `Open` control's own row context.
        decorative
        withBadge={mark.withText}
        icon={mark.mark === "icon" && mark.icon ? glyph(mark.icon) : undefined}
        imageSrc={mark.mark === "image" ? mark.imageSrc : undefined}
      />
      <div className="flex min-w-0 flex-col">
        {/* `block` is what makes `truncate`'s `overflow: hidden` apply at all
            — it does nothing on an inline box — and `max-w-` is what caps the
            cell's intrinsic contribution, which `white-space: nowrap` would
            otherwise set to the whole string. Helm's chart column carries the
            same three classes for the same reason. */}
        <span
          data-testid={`cluster-name-${context.stableId}`}
          className="block max-w-[220px] truncate font-medium"
        >
          {name}
        </span>
        {detail !== "" && (
          <span
            data-testid={`cluster-detail-${context.stableId}`}
            className="path block max-w-[220px] truncate"
          >
            {detail}
          </span>
        )}
      </div>
    </div>
  );
}

/**
 * §6's cluster table — where each cluster comes from, and which credential it
 * uses.
 *
 * **Driven entirely by props.** It fetches nothing, probes nothing and holds no
 * state: the screen owns the contexts, the probes and the facts, so the same
 * rows can be drawn from a fixture. That is also what keeps this out of the
 * cluster rail's business — the rail selects a cluster, this says where the
 * cluster came from.
 *
 * **Four things it will not say**, each from the spec rather than from taste:
 * no `Expires` column (decision 2 — srelens does not know an exec plugin's
 * version requirement, and a wrong expiry claim on a credentials screen
 * actively misleads); no `healthy`/`degraded` (decision 3, see {@link STATUS});
 * no `0 ms` for a cluster that did not answer (decision 4, see
 * {@link latencyLabel}); and no `Team server` (decision 5 — that section of §6
 * is out of scope and no capability backs it, so `Source` has exactly two
 * values).
 *
 * **`Auth` is a mechanism, never a credential.** `authKind` is rendered
 * verbatim and nothing here reformats it: the Rust side decides what may
 * appear in it — `exec plugin · <basename>`, `token`, `client certificate`, a
 * legacy provider's own name, optionally an account — and a second opinion
 * about that string in the UI is how a field whose whole purpose is carrying
 * no secret starts carrying one.
 */
export function ClusterTable({ rows, onOpen, className }: ClusterTableProps) {
  /**
   * The rows in the order the table draws them, grouped.
   *
   * Taken once and handed to the table, rather than called again for the
   * headings: the boundary and the rows have to be describing one list.
   */
  const ordered = bySource(rows, (row) => row.context.isLocal);

  /**
   * How many clusters each group holds, for the count in its heading.
   *
   * Counted from the rows that are on the screen, which is the rule every count
   * in this migration now follows — `Releases · 383 in this cluster` over six
   * rows was the fault, and a heading saying `14 clusters` over a filtered list
   * would be the same one.
   */
  const sizes = new Map<string, number>();
  for (const row of ordered) {
    const word = sourceOf(row.context);
    sizes.set(word, (sizes.get(word) ?? 0) + 1);
  }

  const columns: Column<ClusterRow>[] = [
    {
      key: "cluster",
      header: "Cluster",
      render: (row) => <ClusterCell row={row} />,
      // Sorted and searched on the name the reader can see, not on the object.
      getValue: (row) => contextDisplayName(row.context.name),
    },
    {
      key: "source",
      header: "Source",
      // Two values, and there is no third. See the note above about
      // `Team server`. Rendered rather than left to the table's
      // `String(row[key])` fallback, which would print `undefined`: the value
      // is derived from `isLocal`, not a field of the row.
      render: (row) => <span>{sourceOf(row.context)}</span>,
      getValue: (row) => sourceOf(row.context),
    },
    {
      key: "via",
      header: "Via",
      /**
       * Mono and capped. This holds a full filesystem path —
       * `/Users/dana/Library/Application Support/srelens/kubeconfigs/acme-prod.yaml`
       * is an ordinary one — and an uncapped cell's intrinsic width is that
       * whole string, which is what pushes the 292px rail off the window.
       * jsdom sees none of it; the suite asserts the classes.
       */
      render: (row) => (
        <span
          data-testid={`cluster-via-${row.context.stableId}`}
          className="path block max-w-[260px] truncate font-mono"
          // The full path for the row whose cell is clipped. The text is the
          // same string, so nothing is hidden behind the hover.
          title={viaOf(row.context)}
        >
          {viaOf(row.context)}
        </span>
      ),
      getValue: (row) => viaOf(row.context),
    },
    {
      key: "auth",
      header: "Auth",
      /**
       * Plain, as §6 asks. Capped like `Via` for the same reason: an
       * `oidc · dana@some-quite-long-domain.example` is as unbounded as a path,
       * and this column sits between two that are already capped.
       */
      render: (row) => (
        <span
          data-testid={`cluster-auth-${row.context.stableId}`}
          className="block max-w-[200px] truncate"
          title={row.context.authKind}
        >
          {row.context.authKind}
        </span>
      ),
      getValue: (row) => row.context.authKind,
    },
    {
      key: "latency",
      header: "Latency",
      align: "end",
      /**
       * The round trip to the API server — a network duration, and labelled as
       * one. It is not a health signal and no colour is put on it.
       *
       * A cluster with no reading draws an em dash: the same mark every other
       * screen in this migration uses for a figure it does not have, and
       * `Status` beside it is where the reader learns why.
       */
      render: (row) => {
        const label = latencyLabel(row.probe);
        return label === null ? (
          // The dash plainly, as Helm draws an age it does not have — no
          // `aria-label` dressing it up as a word, because `Status` in the
          // next cell is where the reading state is actually said, and two
          // announcements of one fact is how they start disagreeing.
          <span className="text-faint" title="no reading">
            —
          </span>
        ) : (
          <span className="tabular-nums">{label}</span>
        );
      },
      getValue: (row) => latencyLabel(row.probe) ?? "",
      getSortValue: (row) => latencySort(row.probe),
    },
    {
      key: "status",
      header: "Status",
      render: (row) => {
        const status = STATUS[row.probe.state];
        return <Badge tone={status.tone}>{status.word}</Badge>;
      },
      getValue: (row) => STATUS[row.probe.state].word,
    },
    {
      // §6's unnamed trailing column.
      key: "actions",
      header: "",
      sortable: false,
      filterable: false,
      align: "end",
      /**
       * Pinned to the end of the table, and this is the whole of §6's primary
       * row action being reachable.
       *
       * MEASURED in Chrome, seventeen clusters on a real kubeconfig: the seven
       * columns sum to 1082px, the pane is 1014px wide at the default 1600px
       * window, and this button's rect was x=1311…1355 against a visible right
       * edge of 1308 — off screen on every row, with the window having to reach
       * 1668px before it appeared. At the 960px minimum the pane is 374px and
       * the overflow 711px.
       *
       * **Pinned rather than capped**, deliberately: shaving the two capped text
       * columns can fit 1600px and cannot fit 960px, where 711px of overflow
       * puts the button past the scroll however narrow the text gets. A pinned
       * column works at every width. Enter and double-click already opened the
       * row, so what was missing was the CONTROL, not the action.
       */
      sticky: "end",
      minWidth: 80,
      render: (row) => (
        // `shrink-0` and `whitespace-nowrap`: flex items shrink by default, and
        // the capped cells to the left are what absorb a narrow window instead
        // — a truncated path is recoverable by widening a column, a clipped
        // control is not.
        <div className="flex shrink-0 items-center justify-end whitespace-nowrap">
          <Button
            size="xs"
            variant="secondary"
            // The word is §6's, and it is deliberately NOT
            // `aria-label="Open prod-eu"`: the accessible name stays `Open`
            // while the cell that names the cluster is the row's own first
            // column, which is what a row-reading screen reader announces
            // around it. `title` carries the cluster for a pointer.
            title={`Open ${contextDisplayName(row.context.name)}`}
            onClick={() => onOpen(row.context.stableId)}
          >
            Open
          </Button>
        </div>
      ),
    },
  ];

  return (
    // `min-w-0` on the frame, not only inside the cells: this component is
    // dropped into the screen's flex row beside the 292px rail, and the frame
    // is the flex item whose `min-width: auto` would refuse to shrink.
    //
    // The SCROLL container is the caller's, through `className` — the screen
    // owns how tall its pane is and whether the table scrolls inside it, the
    // way `Helm` hands its own table a `scroll min-h-0 flex-1` wrapper. A
    // second one here would be a box inside a box with its own scrollbar.
    <div data-testid="cluster-table" className={cx("min-w-0", className)}>
      <Table
        columns={columns}
        data={ordered}
        /**
         * §6's grouping, drawn as a boundary rather than left to the row order.
         *
         * The rows were already ordered — kubeconfig contexts, then local
         * clusters, never interleaved — and on a real kubeconfig the only cue a
         * reader had was the `Source` cell's text changing at row 15 of 17.
         * Ordering is not grouping; this is the line between the two groups,
         * in the same two words the `Source` cells under it use, so the heading
         * cannot come to say something the column does not.
         *
         * **It stops when the reader sorts, and the kit is what stops it** (see
         * `rowGroup`): a sort by `Latency` scatters the sources through one
         * another, and a heading left over that list would be labelling rows
         * that had moved out from under it. The `Source` column is what carries
         * the fact from then on, and clearing the sort brings the boundary back.
         */
        rowGroup={(row) => {
          const word = sourceOf(row.context);
          // Through `joined`, the one separator on this screen — the same
          // ` · ` the rail's own count lines and the second row of every
          // cluster cell are assembled with.
          return { key: word, label: joined([word, plural(sizes.get(word) ?? 0, "cluster")]) };
        }}
        getRowKey={(row) => row.context.stableId}
        // The keyboard's half of `Open`: Enter on the focused row, or a
        // double-click. The kit's own rule — a pointer-only route to opening a
        // row is a fault — and it opens exactly what the button opens.
        onRowActivate={(row) => onOpen(row.context.stableId)}
        // A screen with no clusters at all is `/connect`, not this table with
        // an empty panel (spec: States), so this is the narrow case of a list
        // that came back short rather than the first-run one.
        emptyText="No clusters"
      />
    </div>
  );
}
