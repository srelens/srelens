import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { ContextMenu, type ContextMenuItem } from "./ContextMenu";
import { cx } from "./cx";
import type { IconComponent } from "./IconButton";
import { NavIcon } from "./NavIcon";
import { Popover } from "./Popover";
import { filled } from "./slot";
import { toneColor } from "./tone";

export interface StripTab {
  id: string;
  /** What the tab is called, and the front of its accessible name. */
  title: string;
  /** The quiet tag after the title — the cluster, the namespace, "logs". */
  sub?: string;
  /**
   * The glyph for this tab. Taken per tab rather than mapped from a kind:
   * which icon means "workloads" is the product's vocabulary, not the design
   * system's, the same way {@link NavIcon} renders whatever it is handed.
   */
  icon?: IconComponent;
  /** Pinned tabs show a pin instead of a close, and refuse to be closed. */
  pinned?: boolean;
  /** A peek rather than a commitment — drawn in italic by the stylesheet. */
  preview?: boolean;
}

export interface TabStripProps {
  tabs: StripTab[];
  /** The document on screen. An id matching nothing is tolerated. */
  activeId: string;
  /**
   * Fires for a tab that is already active too, so a caller that promotes a
   * preview tab on a second open has somewhere to do it.
   */
  onSelect: (id: string) => void;
  /** Offered per tab when given, and never on a pinned one. Left out, no close at all. */
  onClose?: (id: string) => void;
  /** Offered at the end of the strip when given. */
  onNew?: () => void;
  /** The right-click menu for one tab. Left out, tabs answer no right-click. */
  menuFor?: (tab: StripTab) => ContextMenuItem[];
  /** Names the strip. */
  label?: string;
  newLabel?: string;
  /** The accelerator for a new tab, as printed. The app binds it; see below. */
  newHint?: string;
  overflowLabel?: string;
  className?: string;
}

/* Inline rather than an icon-set import: the kit takes no dependency on lucide,
   and these are the only three glyphs it needs. */

const CloseGlyph = () => (
  <svg width="11" height="11" viewBox="0 0 24 24" fill="none" aria-hidden="true">
    <path d="M18 6 6 18M6 6l12 12" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
  </svg>
);

const PlusGlyph = () => (
  <svg width="13" height="13" viewBox="0 0 24 24" fill="none" aria-hidden="true">
    <path d="M12 5v14M5 12h14" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
  </svg>
);

const ChevronGlyph = () => (
  <svg width="13" height="13" viewBox="0 0 24 24" fill="none" aria-hidden="true">
    <path d="m6 9 6 6 6-6" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
  </svg>
);

/**
 * The full name, said rather than truncated.
 *
 * The mock hung this off `title=`, which is a tooltip: not reachable from a
 * keyboard, announced inconsistently, and not the tab's accessible name. Set
 * explicitly here for a second reason — the close button lives inside the tab,
 * and a name computed from the contents would read "Pods prod-eu Close Pods".
 * Pinned is in the name because it is why Delete does nothing.
 */
function tabName(tab: StripTab): string {
  return `${tab.title}${filled(tab.sub) ? ` · ${tab.sub}` : ""}${tab.pinned ? ", pinned" : ""}`;
}

/**
 * The app's document tabs: the strip along the top of the window holding every
 * resource, log stream, shell and editor that is open.
 *
 * Not the kit's `Tabs`, despite the shared word and the shared stylesheet.
 * `Tabs` is a view switcher inside one screen — a fixed handful of labels, all
 * of them cheap, none of them dismissable. These are documents. They arrive and
 * leave as the user works, they close, they pin, they carry the cluster they
 * belong to, they spill past the width of the window into a list, and each one
 * answers a right-click. #318 recorded this as a restyle of `Tabs`, which is
 * the mistake #332 exists to correct: the two share no behaviour worth sharing.
 *
 * The keyboard is most of what changed from the mock, where there was none. Its
 * tabs were `<div role="tab">` with a click handler and no `tabIndex`, so the
 * document tab bar of a desktop application could not be reached at all without
 * a mouse, while `role="tablist"` promised assistive technology arrow keys that
 * did nothing. The strip is one tab stop landing on the active tab, Left and
 * Right move the focus within it, Home and End go to the ends, and Enter or
 * Space opens what the focus is on.
 *
 * Activation is manual, unlike `Tabs`, which selects as focus moves. There the
 * panels are already rendered and switching is free; here a tab is a terminal
 * session or a log stream, and arrowing past three of them to reach the fourth
 * must not open three documents on the way.
 *
 * Arrowing does not wrap, and that is a deliberate departure from its sibling.
 * `Tabs` holds three or four labels that are all on screen at once, so wrapping
 * is a shortcut. This strip scrolls: it holds as many documents as the user has
 * opened, most of them out of view, and the browser scrolls the focused tab
 * into view as it goes. Wrapping there means one arrow key at the last tab
 * yanks the whole strip back to the start — the tab bar equivalent of a cursor
 * jumping to the top of the file — and Home and End already reach the ends
 * deliberately. The ARIA practices leave the choice open; a scrolling strip is
 * the case where the ends should feel like ends.
 *
 * Delete and Backspace close the focused tab, which is the practices guide's
 * own recommendation for closable tabs and the only reason the close affordance
 * has a keyboard equivalent at all. The visible close button stays where it is
 * and stays pointer-operable, but out of the tab order at `tabIndex={-1}`. That
 * is the same reckoning `WorkspaceTree` made when it declined `role="tree"` —
 * a tab, like a treeitem, is one focusable node, and three controls do not fit
 * in one — except that here the pattern is worth keeping and the extra control
 * is what gives way. Tab reaches the strip and then leaves it; the close is
 * reached by pointer or by Delete, and closing moves the focus to a neighbour
 * rather than dropping it on the body.
 *
 * No accelerator is installed. The mock bound six of them at the window per
 * instance — ⌘W, ⌘T, ⌘⇧T, ⌘[, ⌘] and ⌘1-9 — which `ConsoleDock` already had
 * stripped out for the reason that applies here twice over: a window-level key
 * belongs to whatever owns the window, a component cannot know what else the
 * app has bound, and two strips on screen both answer the same keystroke. The
 * hints are rendered, through `newHint` and through the menu items the caller
 * supplies; the app does the binding.
 *
 * Everything the mock read out of `useTabs()` and did by calling into the store
 * — activate, close, open, duplicate, pin, close others, close to the right,
 * reopen — is a prop or a caller-supplied menu item now. The kit holds no app
 * state. `menuFor` is the seam for the long tail of it: the strip knows a tab
 * can be right-clicked, not what the product offers when it is. (#332)
 */
export function TabStrip({
  tabs,
  activeId,
  onSelect,
  onClose,
  onNew,
  menuFor,
  label = "Open tabs",
  newLabel = "New tab",
  newHint,
  overflowLabel = "All open tabs",
  className,
}: TabStripProps) {
  const listRef = useRef<HTMLDivElement>(null);
  const refs = useRef(new Map<string, HTMLDivElement>());
  // Where the roving tab stop sits while the strip has the focus. Separate from
  // `activeId` precisely because activation is manual: the two are the same
  // until the user starts arrowing, and the point of arrowing is that they part.
  const [focusedId, setFocusedId] = useState<string | null>(null);
  // The tab to land on once the caller has actually removed the one being
  // closed. Held in a ref rather than state: nothing renders differently for
  // it, and it must survive the render that the close causes.
  const pending = useRef<{ closed: string; next: string } | null>(null);

  const closable = onClose !== undefined;

  // Falls back through active to the first tab, so the strip keeps a tab stop
  // even when the caller hands over an id that matches nothing — mid-transition,
  // or one that has just been closed. A strip with no stop is out of the tab
  // order entirely, which is the fault this component exists to fix.
  const roving =
    tabs.find((t) => t.id === focusedId)?.id ??
    tabs.find((t) => t.id === activeId)?.id ??
    tabs[0]?.id;

  useEffect(() => {
    const move = pending.current;
    if (!move) return;
    pending.current = null;
    // The caller may have declined to close it. Leave the focus alone.
    if (tabs.some((t) => t.id === move.closed)) return;
    const node = refs.current.get(move.next);
    if (!node) return;
    setFocusedId(move.next);
    node.focus();
  }, [tabs]);

  function requestClose(tab: StripTab) {
    // Pinned is the user saying "not this one", and a pinned tab shows no close
    // button — so Delete must not be the way around it.
    if (!onClose || tab.pinned) return;
    const at = tabs.findIndex((t) => t.id === tab.id);
    const neighbour = tabs[at + 1] ?? tabs[at - 1];
    // Only when the strip is where the focus already is: closing a tab from a
    // menu somewhere else should not pull the focus back here.
    const holdsFocus = listRef.current?.contains(document.activeElement) ?? false;
    pending.current = holdsFocus && neighbour ? { closed: tab.id, next: neighbour.id } : null;
    onClose(tab.id);
  }

  function onKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    // From the tab the focus is on, not from `activeId`: with manual activation
    // those differ by design, and the close button inside a tab can be the
    // focus too — its keys are not the strip's.
    const target = (event.target as HTMLElement).closest('[role="tab"]');
    const from = tabs.findIndex((t) => refs.current.get(t.id) === target);
    if (from < 0) return;

    if (event.key === "Enter" || event.key === " ") {
      // A div is not a button, so neither key clicks it, and Space would scroll
      // the page instead.
      event.preventDefault();
      onSelect(tabs[from].id);
      return;
    }
    if (event.key === "Delete" || event.key === "Backspace") {
      // Backspace is browser history in some hosts, and neither should reach
      // whatever is behind the strip.
      event.preventDefault();
      requestClose(tabs[from]);
      return;
    }

    let next = from;
    if (event.key === "ArrowRight") next = Math.min(from + 1, tabs.length - 1);
    else if (event.key === "ArrowLeft") next = Math.max(from - 1, 0);
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = tabs.length - 1;
    else return;
    // At either end the key is left alone rather than swallowed: nothing moved,
    // and no wrapping — see the doc comment.
    if (next === from) return;
    event.preventDefault();
    setFocusedId(tabs[next].id);
    refs.current.get(tabs[next].id)?.focus();
  }

  return (
    <div className={cx("tabstrip", className)}>
      {/*
        The tablist is the scrolling half rather than the whole bar, for two
        reasons: a tablist owns tabs and nothing else, so the new and overflow
        controls cannot be inside it; and those two should stay put while the
        tabs scroll under them.
      */}
      <div
        ref={listRef}
        role="tablist"
        aria-label={label}
        className="flex min-w-0 flex-1 overflow-x-auto"
        onKeyDown={onKeyDown}
        onBlur={(event) => {
          // Focus has left the strip: the tab stop goes back to the active tab,
          // so returning to the strip lands on the document being read rather
          // than wherever the last arrow key stopped.
          if (!event.currentTarget.contains(event.relatedTarget)) setFocusedId(null);
        }}
      >
        {tabs.map((tab) => {
          const current = tab.id === activeId;
          const node = (
            <div
              key={tab.id}
              ref={(el) => {
                if (el) refs.current.set(tab.id, el);
                else refs.current.delete(tab.id);
              }}
              role="tab"
              className="tab"
              data-active={current}
              data-preview={tab.preview ? "true" : undefined}
              data-pinned={tab.pinned ? "true" : undefined}
              aria-selected={current}
              aria-label={tabName(tab)}
              tabIndex={tab.id === roving ? 0 : -1}
              onFocus={() => setFocusedId(tab.id)}
              onClick={() => onSelect(tab.id)}
              // Kept from the mock, where it was the only way to close a tab
              // that did not go through a window accelerator. It is a shortcut
              // now rather than the sole route, which is what made it a fault.
              onAuxClick={(event) => {
                if (event.button === 1) requestClose(tab);
              }}
            >
              {tab.icon && (
                // Tinted by the tab rather than by the icon: `currentColor`
                // carries it down, so a caller's plain SVG needs to know
                // nothing about the active state. The slot guarantees the
                // icon is hidden, the way ContextMenu's does.
                <span
                  className="flex shrink-0 items-center"
                  style={{ color: current ? toneColor("accent") : "var(--ink-faint)" }}
                >
                  <NavIcon icon={tab.icon} />
                </span>
              )}
              <span className="truncate">{tab.title}</span>
              {filled(tab.sub) && <span className="tab-sub truncate">{tab.sub}</span>}
              {tab.pinned ? (
                // A dot in the stylesheet, not a glyph. Hidden: the tab's own
                // name says "pinned", which is where it is any use.
                <span className="tab-pin" aria-hidden="true" />
              ) : (
                closable && (
                  <button
                    type="button"
                    className="tab-close"
                    // Pointer-operable, out of the tab order. See the doc
                    // comment: the tab stays one focusable node, and Delete is
                    // the keyboard's way to this.
                    tabIndex={-1}
                    aria-label={`Close ${tab.title}`}
                    onClick={(event) => {
                      // The button sits inside the tab, so its click reaches
                      // the tab's own handler otherwise — and closing a tab you
                      // were not on should not first switch you to it.
                      event.stopPropagation();
                      requestClose(tab);
                    }}
                  >
                    <CloseGlyph />
                  </button>
                )
              )}
            </div>
          );

          return menuFor ? (
            <ContextMenu key={tab.id} items={menuFor(tab)} label={`${tab.title} actions`}>
              {node}
            </ContextMenu>
          ) : (
            node
          );
        })}
      </div>

      {filled(onNew ? newLabel : null) && (
        <button
          type="button"
          className="tab-new"
          // The keystroke is shown, never folded into the name: "New tab
          // command T" is not what the control does, and not what a
          // speech-input user would say to reach it. Same reasoning as
          // ContextMenu's hints.
          aria-label={newLabel}
          title={filled(newHint) ? `${newLabel}  ${newHint}` : newLabel}
          onClick={onNew}
        >
          <PlusGlyph />
        </button>
      )}

      {tabs.length > 0 && (
        // Always offered rather than only when the strip actually overflows:
        // knowing that needs a measurement on every render and every resize,
        // and a control that appears and disappears under the pointer is worse
        // than one that is always in the same place.
        <Popover
          label={overflowLabel}
          align="end"
          className="w-[268px] py-1"
          trigger={
            <span className="tab-new">
              <ChevronGlyph />
              {/* The mock's trigger was a `<span title="All open tabs">`: not
                  focusable, not a button, named only by a tooltip. Popover
                  gives it the button; the name has to come from what it shows,
                  and it shows a chevron. */}
              <span className="sr-only">{overflowLabel}</span>
            </span>
          }
        >
          {(close) => (
            <>
              {tabs.map((tab) => {
                const current = tab.id === activeId;
                return (
                  <button
                    key={tab.id}
                    type="button"
                    className="ns-row"
                    data-on={current}
                    // Which one you are on, said rather than only shown.
                    aria-current={current || undefined}
                    onClick={() => {
                      onSelect(tab.id);
                      close();
                    }}
                  >
                    <span
                      className="flex w-[12px] shrink-0 items-center justify-center"
                      style={{ color: current ? toneColor("accent") : "var(--ink-faint)" }}
                    >
                      {tab.icon && <NavIcon icon={tab.icon} />}
                    </span>
                    <span className="flex-1 truncate text-left">{tab.title}</span>
                    {filled(tab.sub) && <span className="path shrink-0 text-faint">{tab.sub}</span>}
                  </button>
                );
              })}
            </>
          )}
        </Popover>
      )}
    </div>
  );
}
