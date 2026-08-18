import React, { useEffect, useRef, useState } from "react";
import { X } from "lucide-react";

export interface DrawerProps {
  open: boolean;
  title?: React.ReactNode;
  /** Action controls shown on the right of the header, before Close. */
  headerActions?: React.ReactNode;
  onClose: () => void;
  children: React.ReactNode;
  defaultWidth?: number;
}

/**
 * Docked right-side detail panel. Sits inline beside the list (a flex sibling),
 * so the content area shrinks rather than being covered. Drag its left edge to
 * resize. Renders nothing when closed; width persists across open/close.
 */
export function Drawer({ open, title, headerActions, onClose, children, defaultWidth = 480 }: DrawerProps) {
  const [width, setWidth] = useState(defaultWidth);
  const handleRef = useRef<HTMLDivElement>(null);
  const panelRef = useRef<HTMLElement>(null);
  const returnFocusTo = useRef<HTMLElement | null>(null);
  const startX = useRef(0);
  const startW = useRef(0);
  const widthRef = useRef(width);
  widthRef.current = width;

  useEffect(() => setWidth(defaultWidth), [defaultWidth]);

  useEffect(() => {
    const handle = handleRef.current;
    if (!handle) return;
    function move(e: MouseEvent) {
      // Dragging the left edge: the panel grows as the pointer moves left.
      const next = startW.current - (e.clientX - startX.current);
      setWidth(Math.max(320, Math.min(960, next)));
    }
    function up() {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
      document.body.style.userSelect = "";
    }
    function down(e: MouseEvent) {
      e.preventDefault();
      startX.current = e.clientX;
      startW.current = widthRef.current;
      document.body.style.userSelect = "none";
      window.addEventListener("mousemove", move);
      window.addEventListener("mouseup", up);
    }
    handle.addEventListener("mousedown", down);
    return () => {
      handle.removeEventListener("mousedown", down);
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
      document.body.style.userSelect = "";
    };
  }, [open]);

  // Move focus into the panel when it opens, and hand it back to whatever
  // opened it when it closes (#160). The drawer is deliberately NOT modal — it
  // sits beside the list rather than over it, so focus is placed, never
  // trapped: a keyboard user can tab straight back out to the rows behind.
  useEffect(() => {
    if (!open) return;
    const opener = document.activeElement;
    returnFocusTo.current = opener instanceof HTMLElement ? opener : null;
    panelRef.current?.focus();
    return () => {
      // Only if focus is still inside the closing panel: the user may have
      // clicked elsewhere, and yanking focus back would be the panel arguing
      // with them on the way out.
      const active = document.activeElement;
      const inside = panelRef.current?.contains(active as Node) || active === document.body;
      if (inside && returnFocusTo.current?.isConnected) returnFocusTo.current.focus();
    };
  }, [open]);

  // Close on Escape while open. Bail when a modal dialog is layered on top — it
  // owns Esc, so the first Esc closes the dialog and a second closes the drawer —
  // and when focus is in an editable field / the manifest editor, where Esc has
  // its own meaning.
  useEffect(() => {
    if (!open) return;
    function onKeyDown(e: KeyboardEvent) {
      if (e.key !== "Escape" || e.defaultPrevented) return;
      if (document.querySelector('[role="dialog"][data-state="open"]')) return;
      const el = document.activeElement as HTMLElement | null;
      if (el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.isContentEditable)) {
        return;
      }
      onClose();
    }
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [open, onClose]);

  if (!open) return null;
  return (
    <aside
      ref={panelRef}
      aria-label="Details"
      // -1: focusable so opening can land here, but never a Tab stop of its
      // own — tabbing from the list should reach the panel's controls, not the
      // panel itself.
      tabIndex={-1}
      style={{ width }}
      className="relative flex shrink-0 flex-col border-l border-border bg-card outline-none"
    >
      <div
        ref={handleRef}
        aria-hidden="true"
        className="absolute inset-y-0 left-0 z-10 w-1.5 -translate-x-1/2 cursor-col-resize hover:bg-primary/40"
      />
      <header className="flex shrink-0 items-center gap-2 border-b border-border px-4 py-2">
        <div className="min-w-0 flex-1 truncate text-sm font-medium">{title}</div>
        <div className="flex items-center gap-1">{headerActions}</div>
        <button
          type="button"
          aria-label="Close"
          onClick={onClose}
          className="fl-details-close inline-flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground"
        >
          <X aria-hidden="true" />
        </button>
      </header>
      <div className="min-h-0 flex-1 overflow-y-auto p-4">{children}</div>
    </aside>
  );
}
