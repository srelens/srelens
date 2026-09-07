import { useState, type ReactNode } from "react";
import { cx } from "./cx";
import { EmptyState } from "./EmptyState";
import { ResizeHandle } from "./ResizeHandle";
import { filled } from "./slot";
import { TextInput } from "./TextInput";

export interface SidebarProps {
  /**
   * Names the landmark, and by extension the resize handle. Required: a page
   * with two unnamed `nav`s in it gives a screen reader two identical stops.
   */
  label: string;
  /** The way out of a drilled-in view, at the very top. */
  back?: { label: string; count?: ReactNode; onClick: () => void };
  /** The identity band under the back bar — whose cluster this is, how it connects. */
  header?: ReactNode;
  query?: string;
  onQueryChange?: (query: string) => void;
  /** Names the filter box and fills its placeholder. */
  queryLabel?: string;
  /** The scrolling middle: a tree, a list, whatever the caller navigates by. */
  children?: ReactNode;
  emptyTitle?: ReactNode;
  emptyHint?: ReactNode;
  footer?: ReactNode;
  defaultWidth?: number;
  minWidth?: number;
  maxWidth?: number;
  /** Fired when a resize settles, so the app can persist it. */
  onWidthChange?: (width: number) => void;
  className?: string;
}

/**
 * The app's left-hand navigation column: a way back, whose cluster you are
 * looking at, a filter, the tree itself, and whatever the app wants to keep in
 * view at the bottom — in a column the user can widen.
 *
 * Every one of those is a slot. The mock's sidebar reached into four modules to
 * fill them itself — the active tab's route, the workspace's clusters, the
 * hotbar's chips — and chose between two different trees on a flag it read from
 * a store. None of that is a design decision, and the kit cannot hold app state
 * anyway, so what is left is the frame: the bands, the rules between them, the
 * scrolling middle and the drag. What goes in them is the caller's, including
 * which tree; that is why this takes `children` rather than a `focused` flag.
 *
 * The resize is the part worth hardening, and it now lives in
 * {@link ResizeHandle} — this was the only hardened copy of it in the kit, and
 * the detail peek beside the resource list needed the same control on its
 * other edge. What is left here is the state the handle deliberately does not
 * hold: the width itself, and the report to the app when it settles, because
 * `localStorage` is the app's and not the design system's. (#320)
 */
export function Sidebar({
  label,
  back,
  header,
  query,
  onQueryChange,
  queryLabel = "Filter resources",
  children,
  emptyTitle = "Nothing here",
  emptyHint,
  footer,
  defaultWidth = 238,
  minWidth = 180,
  maxWidth = 420,
  onWidthChange,
  className,
}: SidebarProps) {
  const [width, setWidth] = useState(defaultWidth);

  return (
    <nav
      aria-label={label}
      className={cx("relative flex shrink-0 flex-col", className)}
      style={{ width, background: "var(--surface-sunk)" }}
    >
      {back && (
        <button type="button" className="focus-back rule-b" onClick={back.onClick}>
          {/* Inline rather than an icon-set import: the kit takes no dependency
              on lucide. */}
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" aria-hidden="true" className="shrink-0">
            <path
              d="m15 18-6-6 6-6"
              stroke="currentColor"
              strokeWidth="2.5"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          </svg>
          <span className="truncate">{back.label}</span>
          <span className="flex-1" />
          {filled(back.count) && <span className="tree-count">{back.count}</span>}
        </button>
      )}

      {filled(header) && (
        <div data-slot="header" className="rule-b px-2.5 py-2">
          {header}
        </div>
      )}

      {onQueryChange && (
        <div className="rule-b px-2 py-1.5">
          {/* The kit's input rather than the mock's bare one, which had a
              placeholder and no label — and a placeholder disappears the moment
              anything is typed into it. */}
          <TextInput
            type="search"
            value={query ?? ""}
            onValueChange={onQueryChange}
            placeholder={queryLabel}
            aria-label={queryLabel}
          />
        </div>
      )}

      <div className="scroll flex-1 py-1">
        {filled(children) ? children : <EmptyState title={emptyTitle} hint={emptyHint} />}
      </div>

      {filled(footer) && (
        <div data-slot="footer" className="rule-t p-2">
          {footer}
        </div>
      )}

      {/* The sidebar is docked on the left, so its grip is on its right edge
          — `ResizeHandle`'s default. It names itself after the landmark. */}
      <ResizeHandle
        label={label}
        width={width}
        minWidth={minWidth}
        maxWidth={maxWidth}
        onResize={setWidth}
        onCommit={onWidthChange}
      />
    </nav>
  );
}
