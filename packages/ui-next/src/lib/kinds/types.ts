import type { Column } from "@srelens/ui-kit";

/** The least every listed row has: what the table keys and acts on. */
export interface ListRow {
  name: string;
  namespace?: string;
}

/**
 * A row's identity, encoded once — `namespace/name`.
 *
 * Branded rather than a bare `string` on purpose: every map keyed on a row
 * (the table's selection, an `enrich` result) must be keyed by the WHOLE
 * identity, and a name alone typechecks perfectly while being wrong. In an
 * all-namespaces view two namespaces each running `api-0` are two rows, and a
 * name-keyed map silently gives both the reading of whichever arrived last.
 * The brand makes the only way to produce one of these {@link rowKey}, so a
 * second site cannot format the composite key its own way.
 */
export type RowKey = string & { readonly __rowKey: unique symbol };

/**
 * The one encoding of a row's identity: `namespace/name`, with the empty
 * namespace for a cluster-scoped row. Neither a namespace nor a name may
 * contain `/`, so the pair is unambiguous.
 */
export function rowKey(row: ListRow): RowKey {
  return `${row.namespace ?? ""}/${row.name}` as RowKey;
}

/** Which row actions a kind offers. Absent, not disabled — see the spec. */
export interface KindActions {
  logs?: boolean;
  shell?: boolean;
  forward?: boolean;
  scale?: boolean;
  restart?: boolean;
  evict?: boolean;
  /** CronJob only: offers Suspend/Resume, labelled from the row's own state. */
  suspend?: boolean;
  /** CronJob only: offers Run now — not destructive, takes no confirm. */
  trigger?: boolean;
  /**
   * Opposite default from every field above: absent (or `true`) offers
   * Delete, which is what keeps all 34 built-in kinds offering it without
   * every one of their descriptors having to say so. Only `false` withholds
   * it — for a custom resource, whose `k8sKind` is the CRD's own kind and
   * which the backend's kind→GVR resolution has no path for, so Delete would
   * always fail: the confirm (and even the kubectl preview, which falls back
   * to lowercasing) reads as a real operation right up until it isn't.
   * Offering an action that cannot work is worse than not offering it.
   */
  delete?: boolean;
}

/**
 * Everything the list screen needs to know about one kind, as data.
 *
 * The screen names no kind: it looks one of these up and composes. That is
 * what makes the 24 typed column sets a table a reviewer can read rather than
 * 24 branches in a component, and what lets the column and sort behaviour be
 * tested without rendering anything.
 */
export interface KindDescriptor<Row extends ListRow = ListRow> {
  /** The Kubernetes kind, for actions and for the detail route. */
  k8sKind: string;
  /**
   * The API group — set only for a custom kind, whose kind NAME alone does
   * not identify it: a CRD may legally reuse a built-in kind's name in its
   * own group (`Deployment` in `acme.io`), and anything that resolves by
   * kind alone then lands on the built-in. Carried into the edit route.
   */
  group?: string;
  columns: Column<Row>[];
  /** `watch` streams snapshots; `poll` re-lists on an interval. */
  source: "watch" | "poll";
  scope: "namespaced" | "cluster";
  /** Required for `poll`; unused for `watch`. */
  load?: (context: string, namespace: string) => Promise<{ rows?: Row[]; error?: string }>;
  /** Extra per-row data merged onto the row of the same identity — pod
   *  metrics, node metrics. Keyed by {@link rowKey}, never by name: see
   *  {@link RowKey}. */
  enrich?: (context: string, namespace: string) => Promise<Map<RowKey, Partial<Row>>>;
  enrichMs?: number;
  actions: KindActions;
  /**
   * Which rows need a second look — a pod not Running, a workload whose ready
   * count is short of desired. Per-kind, like `actions`: absent for a kind
   * with no sensible notion of "unhealthy" (a Secret, a Service), which shows
   * no dot at all rather than a dot that is always off.
   *
   * A boolean, not a reason string — the screen supplies the words. Same
   * "never colour alone" contract the cluster rail's `unavailable` follows:
   * whatever renders this must say why beside the dot, not just tint it.
   */
  flagged?: (row: Row) => boolean;
  /**
   * Extra panes the resource detail shell offers beyond Details, YAML and
   * Events, which every kind gets. Absent (the default) means neither — most
   * kinds have no containers or metrics of their own. Set by whichever task
   * ports that pane's per-kind body (Pod's containers, a node's or pod's
   * metrics, ...), never inferred from the kind's name.
   */
  panes?: {
    containers?: boolean;
    metrics?: boolean;
  };
}
