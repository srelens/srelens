import { contextDisplayName, type ClusterContext, type ClusterFacts } from "@srelens/core";
import {
  Badge,
  Button,
  Mark,
  Table,
  cx,
  type BadgeTone,
  type Column,
} from "@srelens/ui-kit";
import { useMark } from "../../lib/marks";
import { glyph } from "../../lib/tree";
import type { Probe, ProbeState } from "../../lib/probe";

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
 * What each probe state is called, and how it is toned.
 *
 * **No cluster is ever `healthy` or `degraded`.** §6's mock tones
 * `healthy`→ok and `degraded`→sev, and the spec's decision 3 refuses both:
 * `connectCluster` reports whether the API server answered, and calling that
 * answer a health verdict claims a check that never ran. What is drawn is the
 * reading itself.
 *
 * `unread` is the absence, NAMED as an absence. Not a third status word
 * ("pending", "idle") — those read as things the cluster is, and this is a
 * thing srelens has not done yet.
 */
const STATUS: Record<ProbeState, { word: string; tone: BadgeTone }> = {
  reachable: { word: "reachable", tone: "ok" },
  unreachable: { word: "unreachable", tone: "sev" },
  unread: { word: "no reading", tone: "muted" },
};

/**
 * The one separator on this screen, and the only place parts are joined.
 *
 * Every caller has parts that may be missing — a cluster with no region, a
 * local cluster with no detected provider — so the filter is here rather than
 * at each site. `· ·` and a line that begins or ends with a separator tell a
 * reader nothing and look broken, and both are what an unconditional
 * `parts.join(" · ")` produces.
 *
 * An empty string means there was nothing to say; callers render NOTHING for
 * it rather than an empty line.
 */
function joined(parts: readonly (string | undefined)[]): string {
  return parts
    .map((part) => part?.trim() ?? "")
    .filter((part) => part.length > 0)
    .join(" · ");
}

/**
 * The host a cluster answers on, for a local cluster's `Via`.
 *
 * `new URL` rather than a regexp, and the raw string when it will not parse: a
 * `server` srelens cannot read is still what the kubeconfig says, and printing
 * it verbatim is more use to a reader than an invented "unknown". Unix-socket
 * and non-URL servers arrive here too.
 */
function hostOf(server: string): string {
  try {
    return new URL(server).host || server;
  } catch {
    return server;
  }
}

/**
 * §6's `Source`, and the whole of its vocabulary.
 *
 * Two values, from `isLocal`. **`Team server` is never one of them** — §6's
 * third source signs in to a team server and lists its members with presence,
 * and no capability in core reports either (spec decision 5). A column that
 * could say it would be a column asserting a backend srelens does not have.
 */
function sourceOf(context: ClusterContext): string {
  return context.isLocal ? "Local" : "Kubeconfig";
}

/**
 * §6's `Via`: what the cluster is actually reached THROUGH.
 *
 * For a kubeconfig context that is the file it was declared in — the column's
 * whole reason for existing, and the field decision 1 added. For a local
 * cluster the file is beside the point (kind writes one for you); what
 * identifies it is the tool that made it and the endpoint it listens on.
 */
function viaOf(context: ClusterContext): string {
  return context.isLocal ? joined([context.provider, hostOf(context.server)]) : context.sourceFile;
}

/**
 * The round trip, or nothing.
 *
 * **Gated on the STATE as well as on the number.** `probe.ts` documents
 * `latencyMs` as absent unless the state is `reachable`, so the two gates
 * agree today — but the table is what a reader trusts, and `0 ms` on a cluster
 * that never answered reads as "instant", which is the exact opposite of the
 * truth (spec decision 4). One gate would leave that one drift away; two make
 * it structural.
 *
 * A reading under half a millisecond is a real reading of a cluster on this
 * laptop, and it is NOT discarded — it is drawn as `<1 ms`, because rounding it
 * to `0 ms` would put on screen the one string this column may never show.
 */
function latencyLabel(probe: Probe): string | null {
  if (probe.state !== "reachable") return null;
  const ms = probe.latencyMs;
  if (typeof ms !== "number" || !Number.isFinite(ms) || ms < 0) return null;
  return ms < 0.5 ? "<1 ms" : `${Math.round(ms)} ms`;
}

/** The number the `Latency` column SORTS on — never the text beside it. */
function latencySort(probe: Probe): number {
  // A cluster with no reading sorts last however the column is turned, rather
  // than ordering as the text "—" happens to collate.
  return latencyLabel(probe) === null ? Number.POSITIVE_INFINITY : (probe.latencyMs ?? 0);
}

/**
 * Rows grouped by where the cluster comes from, as §6 groups them.
 *
 * **Kubeconfig contexts first, then local clusters** — §6's own order is
 * `team` → `file` → `local`, and with the team server out of scope (spec
 * decision 5) that leaves file before local. It is also the order Pane 2 lists
 * its sections in (`Kubeconfig · on this machine`, then
 * `Local · runs on this laptop`), so a reader's eye maps a group in the table
 * onto the section beside it.
 *
 * (The brief for this task glossed §6 as "local clusters together, then
 * kubeconfig contexts", which is the reverse of what §6 and Pane 2 both say.
 * The GROUPING is the requirement — rows must not interleave — and this
 * follows the design it cites for the order.)
 *
 * Stable within each group: whatever order the caller listed its clusters in
 * survives, because `listContexts` returns them in the kubeconfig's own order
 * and re-sorting them here would be this file inventing a second opinion about
 * a list the rail already draws one way.
 */
function bySource(rows: readonly ClusterRow[]): ClusterRow[] {
  return [...rows.filter((r) => !r.context.isLocal), ...rows.filter((r) => r.context.isLocal)];
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
        data={bySource(rows)}
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
