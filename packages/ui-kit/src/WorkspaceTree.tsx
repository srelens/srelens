import { useId, useState, type ReactNode } from "react";
import { cx } from "./cx";
import { EmptyState } from "./EmptyState";
import { IconButton, type IconComponent } from "./IconButton";
import { useFolds } from "./ResourceTree";
import { filled } from "./slot";
import { Spinner } from "./Spinner";
import { toneColor } from "./tone";

/**
 * How a cluster is reachable right now. Generic connection state, deliberately:
 * what a "team server" is, or which latency counts as bad, stays in the app.
 */
export type ClusterLink = "connected" | "connecting" | "disconnected" | "error";

export interface WorkspaceCluster {
  id: string;
  name: string;
  /** A mark before the name — the app's cluster chip, colour and all. */
  chip?: ReactNode;
  /** Extra identity for the row's tooltip, e.g. the kube context name. */
  detail?: string;
  link?: ClusterLink;
  /** Why the last connection attempt failed. Only read when `link` is "error". */
  error?: string;
  /** A mark after the name — the app's connection-kind dot, a latency figure. */
  meta?: ReactNode;
  /** The figure at the end of a connected row. */
  count?: ReactNode;
}

export interface WorkspaceTreeProps {
  /** The workspace's own name: the row everything else hangs under. */
  name: string;
  clusters: WorkspaceCluster[];
  /** The cluster the rest of the app is currently pointed at. */
  active?: string;
  onActivate: (id: string) => void;
  /** Offered on a cluster that is offline or failed. Left out, no such button. */
  onConnect?: (id: string) => void;
  /** Offered on a connected cluster. Left out, no such button. */
  onDrillIn?: (id: string) => void;
  /** Give this to own which clusters are open; leave it out and the tree keeps its own. */
  expanded?: string[];
  onExpandedChange?: (id: string, expanded: boolean) => void;
  /** What appears under an opened cluster — usually a `ResourceTree`. */
  renderExpanded?: (cluster: WorkspaceCluster) => ReactNode;
  emptyTitle?: ReactNode;
  emptyHint?: ReactNode;
  emptyAction?: ReactNode;
  className?: string;
}

/** Inline rather than an icon-set import: the kit takes no dependency on lucide. */
function Caret({ open }: { open: boolean }) {
  return (
    <svg
      width="11"
      height="11"
      viewBox="0 0 24 24"
      fill="none"
      aria-hidden="true"
      className="tree-caret shrink-0"
      data-open={open}
    >
      <path
        d="m9 18 6-6-6-6"
        stroke="currentColor"
        strokeWidth="2.5"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

const PlugIcon: IconComponent = ({ size = 14, className, ...rest }) => (
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" className={className} {...rest}>
    <path
      d="M9 2v6M15 2v6M6 8h12v3a6 6 0 0 1-6 6 6 6 0 0 1-6-6V8ZM12 17v5"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
    />
  </svg>
);

const DrillInIcon: IconComponent = ({ size = 14, className, ...rest }) => (
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" className={className} {...rest}>
    <path
      d="M4 4v7a4 4 0 0 0 4 4h12m0 0-4-4m4 4-4 4"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
    />
  </svg>
);

/**
 * The workspace and the clusters in it: one row per cluster, each able to open
 * onto whatever the caller wants to show inside it.
 *
 * Deliberately not a `role="tree"`, unlike its sibling `ResourceTree`. A
 * treeitem is one focusable node, and these rows are not one control but three
 * — a caret that folds, a row that selects, and a button that either drills in
 * or connects. Squeezing that into the tree pattern means either giving up the
 * roving tabindex the role promises, or leaving two of the three controls
 * unreachable from a keyboard; both are worse than what a plain list of
 * ordinary buttons gives you for free, which is that Tab reaches every one of
 * them in the order they are read. The mock landed in the gap between the two:
 * it put `role="treeitem"` and `aria-selected` on a `div` with no tree above
 * it, which claims a structure that is not there. The nesting is only two deep
 * anyway, and the workspace is one thing rather than a forest.
 *
 * What else changed from the mock. Its caret meant "connect" on an offline
 * cluster and "fold" otherwise, and its row meant "connect" or "select" by the
 * same invisible rule, so the same click did different things depending on
 * state a user could only infer from a dimmed row; connecting is its own named
 * button here and the row always means the row. Its drill-in sat at opacity 0
 * until the row was hovered, which is a control a keyboard can focus and nobody
 * can see, so it is simply visible. And a failed connection had nowhere to put
 * its reason, so the reason is a line under the row, tied to it with
 * `aria-describedby` rather than shouted through a live region — one row
 * failing should not interrupt what is being read elsewhere.
 *
 * As with `ResourceTree`, it knows nothing about where its clusters come from:
 * no store, no actions, no cluster chips of its own. (#320)
 */
export function WorkspaceTree({
  name,
  clusters,
  active,
  onActivate,
  onConnect,
  onDrillIn,
  expanded,
  onExpandedChange,
  renderExpanded,
  emptyTitle = "No clusters",
  emptyHint,
  emptyAction,
  className,
}: WorkspaceTreeProps) {
  const folds = useFolds(expanded, onExpandedChange);
  // The root's own fold is a view preference of this component and nothing
  // else's business; a cluster's is app state, which is why only that one is
  // offered to the caller.
  const [rootOpen, setRootOpen] = useState(true);
  const ids = useId();
  const listId = `${ids}list`;

  return (
    <div className={cx("pb-2", className)}>
      <button
        // A bare button inside a form is a submit button (bd24d1a).
        type="button"
        id={`${ids}root`}
        className="tree-group"
        style={{ paddingLeft: 6 }}
        aria-expanded={rootOpen}
        aria-controls={listId}
        onClick={() => setRootOpen((open) => !open)}
      >
        <Caret open={rootOpen} />
        <span className="truncate">{name}</span>
        <span className="flex-1" />
        <span className="tree-count">{clusters.length}</span>
      </button>

      {rootOpen && clusters.length === 0 && (
        <EmptyState title={emptyTitle} hint={emptyHint} action={emptyAction} />
      )}

      {rootOpen && clusters.length > 0 && (
        <ul id={listId} aria-labelledby={`${ids}root`}>
          {clusters.map((cluster) => {
            const link = cluster.link ?? "connected";
            const offline = link === "disconnected" || link === "error";
            // A cluster with no connection has nothing to unfold, so the caret
            // says so rather than opening onto an empty box.
            const foldable = renderExpanded !== undefined;
            const open = foldable && link === "connected" && folds.isOpen(cluster.id, false);
            const bodyId = `${ids}${cluster.id}-body`;
            const errorId = `${ids}${cluster.id}-error`;
            const showError = link === "error" && filled(cluster.error);

            return (
              <li key={cluster.id}>
                <div
                  className="tree-cluster"
                  data-active={cluster.id === active}
                  data-off={offline || undefined}
                >
                  {foldable && (
                    // Hand-written rather than an IconButton: the glyph turns
                    // with the state, which a plain icon button has no notion of.
                    <button
                      type="button"
                      className="caret-btn"
                      aria-expanded={open}
                      aria-controls={bodyId}
                      aria-label={`${open ? "Collapse" : "Expand"} ${cluster.name}`}
                      title={
                        link === "connected"
                          ? undefined
                          : `Connect ${cluster.name} to see its resources`
                      }
                      disabled={link !== "connected"}
                      onClick={() => folds.toggle(cluster.id, open)}
                    >
                      <Caret open={open} />
                    </button>
                  )}

                  <button
                    type="button"
                    className="flex min-w-0 flex-1 items-center gap-1.5 text-left"
                    // `aria-current`, not `aria-selected`: this is a list of
                    // places, and the row is a link in all but markup.
                    aria-current={cluster.id === active ? "true" : undefined}
                    aria-describedby={showError ? errorId : undefined}
                    title={filled(cluster.detail) ? `${cluster.name} · ${cluster.detail}` : cluster.name}
                    onClick={() => onActivate(cluster.id)}
                  >
                    {cluster.chip}
                    <span className="min-w-0 flex-1 truncate">{cluster.name}</span>
                    {link === "connecting" && (
                      <Spinner className="size-3 shrink-0" label={`Connecting to ${cluster.name}`} />
                    )}
                    {/* The link state goes in the row's name, so it is not
                        carried by a dimmed row alone. */}
                    {link === "disconnected" && <span className="tree-count">Offline</span>}
                    {link === "error" && (
                      <span className="tree-count" style={{ color: toneColor("sev") }}>
                        Failed
                      </span>
                    )}
                    {link === "connected" && filled(cluster.count) && (
                      <span className="tree-count">{cluster.count}</span>
                    )}
                  </button>

                  {cluster.meta}

                  {offline && onConnect && (
                    <IconButton
                      icon={PlugIcon}
                      label={`Connect ${cluster.name}`}
                      onClick={() => onConnect(cluster.id)}
                    />
                  )}
                  {link === "connected" && onDrillIn && (
                    <IconButton
                      icon={DrillInIcon}
                      label={`Drill into ${cluster.name}`}
                      onClick={() => onDrillIn(cluster.id)}
                    />
                  )}
                </div>

                {showError && (
                  <p
                    id={errorId}
                    className="px-3 pb-1 text-[0.6875rem] leading-snug"
                    style={{ color: toneColor("sev") }}
                  >
                    {cluster.error}
                  </p>
                )}

                {open && <div id={bodyId}>{renderExpanded?.(cluster)}</div>}
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
