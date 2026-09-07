import { useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { cx } from "./cx";
import { EmptyState } from "./EmptyState";
import { ErrorState } from "./ErrorState";
import type { IconComponent } from "./IconButton";
import { NavIcon } from "./NavIcon";
import { filled } from "./slot";

export interface ResourceNode {
  id: string;
  label: string;
  /**
   * The glyph for this row. Which icon stands for a pod is the app's
   * vocabulary, not the kit's — the mock resolved it through a local map keyed
   * by name, and that map does not come across; see {@link NavIcon}.
   */
  icon?: IconComponent;
  /** The figure at the end of the row. A node with no count shows none; 0 shows. */
  count?: ReactNode;
  /** Present — even empty — makes this node a group rather than a leaf. */
  children?: ResourceNode[];
  /** Groups only, and only the starting state. Defaults to open. */
  defaultExpanded?: boolean;
}

export interface ResourceTreeProps {
  /** Names the tree for assistive technology (e.g. "Cluster resources"). */
  label: string;
  nodes: ResourceNode[];
  /** The id of the node the surrounding view is currently showing. */
  active?: string;
  onActivate: (id: string) => void;
  /** Give this to own the fold state; leave it out and the tree keeps its own. */
  expanded?: string[];
  onExpandedChange?: (id: string, expanded: boolean) => void;
  /** Filters the rows by label, keeping the groups above every match. */
  query?: string;
  emptyTitle?: ReactNode;
  emptyHint?: ReactNode;
  /** Replaces the tree with an announced failure — a stale tree is worse. */
  error?: { title: ReactNode; detail?: ReactNode; onRetry?: () => void };
  className?: string;
}

/**
 * Whether a label, or anything below it, answers to a search.
 *
 * A group survives on its descendants' behalf: hiding "Workloads" because the
 * word "pods" is not in it would hide the match as well.
 */
function matches(node: ResourceNode, query: string): boolean {
  if (node.label.toLowerCase().includes(query)) return true;
  return (node.children ?? []).some((child) => matches(child, query));
}

/**
 * The nodes a query leaves standing, as a new tree.
 *
 * Exported because the sidebar's filter box and the tree are usually two
 * different components in the app, and both need to agree on what "matches"
 * means — the same reason `Table` exports `filterTableData`. Pure: the input
 * tree is never touched, and an empty query hands back the very array it was
 * given so a caller can skip the work.
 *
 * A group whose own label matches keeps all of its children. The mock filtered
 * them by the same query, so searching for a section's name showed that section
 * with nothing underneath it — the one result you asked for, emptied out.
 */
export function filterResourceNodes(nodes: ResourceNode[], query: string): ResourceNode[] {
  const needle = query.trim().toLowerCase();
  if (!needle) return nodes;
  const keep = (node: ResourceNode): ResourceNode | null => {
    if (!matches(node, needle)) return null;
    if (!node.children) return node;
    if (node.label.toLowerCase().includes(needle)) return node;
    return { ...node, children: node.children.map(keep).filter((n): n is ResourceNode => n !== null) };
  };
  return nodes.map(keep).filter((n): n is ResourceNode => n !== null);
}

/**
 * The fold protocol both trees in this pair speak.
 *
 * Controlled when `expanded` is given — the workspace tree's folds live in app
 * state, because which clusters are open outlives the component — and self-kept
 * otherwise, which is what a resource tree wants. Either way the caller hears
 * about every toggle, so a self-kept tree can still be persisted. Written once
 * because two half-implementations of the same protocol is how they drift.
 */
export function useFolds(
  expanded: string[] | undefined,
  onExpandedChange?: (id: string, next: boolean) => void,
) {
  const [overrides, setOverrides] = useState<Record<string, boolean>>({});
  const controlled = expanded ? new Set(expanded) : null;
  return {
    isOpen: (id: string, fallback: boolean) =>
      controlled ? controlled.has(id) : overrides[id] ?? fallback,
    toggle: (id: string, open: boolean) => {
      if (!controlled) setOverrides((o) => ({ ...o, [id]: !open }));
      onExpandedChange?.(id, !open);
    },
  };
}

interface Row {
  node: ResourceNode;
  level: number;
  parentId?: string;
  group: boolean;
  open: boolean;
}

/** The rows actually on screen, top to bottom — a closed group contributes only itself. */
function flatten(
  nodes: ResourceNode[],
  isOpen: (node: ResourceNode) => boolean,
  level = 1,
  parentId?: string,
): Row[] {
  return nodes.flatMap((node) => {
    const group = node.children !== undefined;
    const open = group && isOpen(node);
    const row: Row = { node, level, parentId, group, open };
    return open ? [row, ...flatten(node.children ?? [], isOpen, level + 1, node.id)] : [row];
  });
}

/**
 * The navigation tree down the side of the app: sections that fold, resources
 * that open, a count against each.
 *
 * It is a real `role="tree"` rather than the column of bare `<button>`s the
 * mock drew, and that is the substance of the change. The mock's rows carried
 * `aria-expanded` with no tree above them to give it meaning, every row was its
 * own Tab stop, and no arrow key did anything — so a keyboard user tabbed
 * through sixty rows to reach the last one, and a screen reader was told about
 * a structure it could not move through. Here the whole tree is one Tab stop
 * landing on the active row, Up and Down walk the rows that are showing, Right
 * opens a section and then steps into it, Left closes it and then climbs to its
 * parent, and Home and End reach the ends. The rows stay `<button>`s underneath
 * the role so Enter and Space keep working without any code of ours. The tree
 * pattern earns itself here because the thing genuinely nests and genuinely
 * folds; its sibling `WorkspaceTree` is a list for the opposite reason.
 *
 * The larger change is that it no longer knows where anything comes from. The
 * mock read `navTree` from a module, resolved each icon through a name-keyed
 * map, and called `openTab` itself; this takes its nodes, its active id and its
 * callbacks, and every glyph arrives on the node that wants it — the line
 * `NavIcon` and `MultiSelect` already drew. (#320)
 */
export function ResourceTree({
  label,
  nodes,
  active,
  onActivate,
  expanded,
  onExpandedChange,
  query = "",
  emptyTitle = "Nothing here",
  emptyHint,
  error,
  className,
}: ResourceTreeProps) {
  const folds = useFolds(expanded, onExpandedChange);
  const refs = useRef(new Map<string, HTMLButtonElement>());
  const [focusId, setFocusId] = useState<string | null>(null);

  const needle = query.trim();
  const visible = useMemo(() => filterResourceNodes(nodes, needle), [nodes, needle]);
  // While a search is running the folds step aside: a filter that finds a row
  // and then leaves it hidden behind a closed section has found nothing.
  const rows = flatten(visible, (node) =>
    needle ? true : folds.isOpen(node.id, node.defaultExpanded ?? true),
  );

  if (error) {
    return (
      <ErrorState
        className={className}
        title={error.title}
        detail={error.detail}
        onRetry={error.onRetry}
      />
    );
  }

  if (rows.length === 0) {
    return (
      <EmptyState
        className={className}
        title={needle ? `Nothing matches “${needle}”` : emptyTitle}
        hint={needle ? undefined : emptyHint}
      />
    );
  }

  // Derived rather than corrected in an effect: the row holding the Tab stop
  // can vanish under a fold or a filter, and a tree whose only stop points at a
  // row that is gone has dropped out of the tab order altogether.
  const stop =
    rows.find((r) => r.node.id === focusId)?.node.id ??
    rows.find((r) => r.node.id === active)?.node.id ??
    rows[0].node.id;

  function move(id: string) {
    setFocusId(id);
    refs.current.get(id)?.focus();
  }

  function onKeyDown(event: KeyboardEvent<HTMLUListElement>) {
    // From the focused row, not from `stop`: a controlled parent may not have
    // committed a fold yet, and computing from stale state sends the next arrow
    // key off from a row the user has already left — the lesson `Tabs` records.
    const at = rows.findIndex((r) => refs.current.get(r.node.id) === document.activeElement);
    const index = at >= 0 ? at : rows.findIndex((r) => r.node.id === stop);
    if (index < 0) return;
    const row = rows[index];

    let next: string | null = null;
    if (event.key === "ArrowDown") next = rows[Math.min(index + 1, rows.length - 1)].node.id;
    else if (event.key === "ArrowUp") next = rows[Math.max(index - 1, 0)].node.id;
    else if (event.key === "Home") next = rows[0].node.id;
    else if (event.key === "End") next = rows[rows.length - 1].node.id;
    else if (event.key === "ArrowRight") {
      if (row.group && !row.open) folds.toggle(row.node.id, row.open);
      else if (row.group && index + 1 < rows.length) next = rows[index + 1].node.id;
    } else if (event.key === "ArrowLeft") {
      if (row.group && row.open) folds.toggle(row.node.id, row.open);
      else if (row.parentId) next = row.parentId;
    } else return;

    // Otherwise Up/Down scroll the sidebar and Home/End jump the page.
    event.preventDefault();
    if (next) move(next);
  }

  return (
    <ul role="tree" aria-label={label} className={cx("pb-2", className)} onKeyDown={onKeyDown}>
      {rows.map((row) => (
        // `none`, so the wrapper does not put a list between the tree and its
        // items: the nesting is carried by aria-level.
        <li role="none" key={row.node.id}>
          <button
            // Explicit, because a bare button inside a form is a submit button
            // and this tree lives beside filter boxes and inline forms.
            type="button"
            role="treeitem"
            ref={(node) => {
              if (node) refs.current.set(row.node.id, node);
              else refs.current.delete(row.node.id);
            }}
            className={row.group ? "tree-group" : "tree-row"}
            style={{ paddingLeft: (row.group ? 6 : 10) + (row.level - 1) * 11 }}
            aria-level={row.level}
            // Only where it means something: a leaf that says aria-expanded=false
            // promises a subtree it does not have.
            aria-expanded={row.group ? row.open : undefined}
            aria-selected={row.group ? undefined : row.node.id === active}
            data-active={row.node.id === active}
            tabIndex={row.node.id === stop ? 0 : -1}
            title={row.node.label}
            onFocus={() => setFocusId(row.node.id)}
            onClick={() =>
              row.group ? folds.toggle(row.node.id, row.open) : onActivate(row.node.id)
            }
          >
            {row.group && (
              // Inline rather than an icon-set import: the kit takes no
              // dependency on lucide.
              <svg
                width="11"
                height="11"
                viewBox="0 0 24 24"
                fill="none"
                aria-hidden="true"
                className="tree-caret shrink-0"
                data-open={row.open}
              >
                <path
                  d="m9 18 6-6-6-6"
                  stroke="currentColor"
                  strokeWidth="2.5"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                />
              </svg>
            )}
            {row.node.icon && <NavIcon icon={row.node.icon} />}
            <span className="truncate">{row.node.label}</span>
            <span className="flex-1" />
            {filled(row.node.count) && <span className="tree-count">{row.node.count}</span>}
          </button>
        </li>
      ))}
    </ul>
  );
}
