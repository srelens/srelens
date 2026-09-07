import { useEffect, useRef, type KeyboardEvent, type MouseEvent } from "react";
import { cx } from "./cx";

/** Which edge of the pane the grip sits on, and so which way widens it. */
export type ResizeEdge = "left" | "right";

export interface ResizeHandleProps {
  /**
   * What this resizes, in the reader's words — "Cluster navigation", "the
   * resource details". The handle announces itself as `Resize {label}`, so a
   * page with two of these gives assistive technology two distinguishable
   * stops rather than two anonymous separators.
   */
  label: string;
  /** The pane's current width, announced as `aria-valuenow`. */
  width: number;
  minWidth: number;
  maxWidth: number;
  /** The edge the handle is on. A pane docked left grips on its right. */
  edge?: ResizeEdge;
  /** Every step of a drag, and every key: the host owns the width. */
  onResize: (width: number) => void;
  /** Once, when the resize settles — the moment worth persisting. */
  onCommit?: (width: number) => void;
  className?: string;
}

/** One arrow key's worth of width. Coarse enough to get somewhere, fine enough to aim. */
const STEP = 16;

/**
 * The grip on a resizable pane's edge: drag it, or focus it and use the keys.
 *
 * Extracted from `Sidebar`, which had the only hardened copy of this and is
 * now one of its two callers. It was worth extracting rather than copying
 * because everything below the pointer handling is an accessibility detail
 * that a second copy drifts on. The mock's version was a `role="separator"`
 * with a mousedown listener and nothing else — a control announced to
 * assistive technology that only a pointer can work, which is the worst of
 * both. So: it is named after what it moves, it carries its width as
 * `aria-valuenow` between its two bounds, it is in the tab order, and it takes
 * the arrow keys and Home/End, `preventDefault`ing them so Home does not also
 * jump the page.
 *
 * The drag measures from where the pointer went DOWN rather than from
 * `clientX` minus a constant — the mock subtracted 46, the width of the rail
 * it happened to sit beside, an assumption about the shell that the kit is in
 * no position to make. Listeners live on `window` for the duration and a
 * `release` ref undoes them (and the `user-select` lock) from an unmount, so a
 * pane torn down mid-drag cannot strand a listener or leave the page
 * unselectable.
 *
 * Width is reported, never stored: `localStorage` is the app's, not the design
 * system's. And it is reported once on release rather than per pixel, because
 * a caller persisting it should not be written to forty times on the way
 * there. (#320)
 */
export function ResizeHandle({
  label,
  width,
  minWidth,
  maxWidth,
  edge = "right",
  onResize,
  onCommit,
  className,
}: ResizeHandleProps) {
  const widthRef = useRef(width);
  widthRef.current = width;
  // Whatever a drag in flight needs undone, so unmounting mid-drag does not
  // leave listeners on the window and the page unselectable.
  const release = useRef<() => void>(() => {});
  useEffect(() => () => release.current(), []);

  const clamp = (next: number) => Math.max(minWidth, Math.min(maxWidth, Math.round(next)));

  function onMouseDown(event: MouseEvent) {
    event.preventDefault();
    const startX = event.clientX;
    const startWidth = widthRef.current;
    // Tracked here rather than read back off `widthRef` at release: the host
    // is free to ignore a step, and what settled is what this handle last
    // asked for.
    let settled = startWidth;
    const move = (e: globalThis.MouseEvent) => {
      const travelled = e.clientX - startX;
      // The whole of the difference between the two edges: on a left-edge
      // grip the pane widens as the pointer goes left, because that is the
      // direction its edge moves.
      settled = clamp(edge === "left" ? startWidth - travelled : startWidth + travelled);
      onResize(settled);
    };
    const detach = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
      document.body.style.userSelect = "";
      release.current = () => {};
    };
    function up() {
      detach();
      onCommit?.(settled);
    }
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
    document.body.style.userSelect = "none";
    release.current = detach;
  }

  function onKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    // Inverted with the edge, for the same reason the drag is: the key that
    // moves the edge outward is the one that widens the pane.
    const wider = edge === "left" ? "ArrowLeft" : "ArrowRight";
    const narrower = edge === "left" ? "ArrowRight" : "ArrowLeft";
    let next: number | null = null;
    if (event.key === wider) next = width + STEP;
    else if (event.key === narrower) next = width - STEP;
    // About the value, not the direction: narrowest and widest are the same
    // two answers on either edge.
    else if (event.key === "Home") next = minWidth;
    else if (event.key === "End") next = maxWidth;
    if (next === null) return;
    // Otherwise Home/End also jump the page and the arrows scroll the pane.
    event.preventDefault();
    const settled = clamp(next);
    onResize(settled);
    onCommit?.(settled);
  }

  return (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label={`Resize ${label}`}
      aria-valuenow={width}
      aria-valuemin={minWidth}
      aria-valuemax={maxWidth}
      // A resize a pointer can do and a keyboard cannot is not a resize.
      tabIndex={0}
      data-edge={edge}
      className={cx("resize-handle", className)}
      onMouseDown={onMouseDown}
      onKeyDown={onKeyDown}
    />
  );
}
