import { useRef, type KeyboardEvent } from "react";

export interface TabItem {
  id: string;
  label: string;
}

/**
 * `strip` is the window chrome's flat run of tabs; `segmented` is the compact
 * rounded control the design draws inside a pane; `underline` is the row of
 * words a full page reads across, ruled beneath the active one.
 */
export type TabsVariant = "strip" | "segmented" | "underline";

export interface TabsProps {
  tabs: TabItem[];
  active: string;
  onChange: (id: string) => void;
  /** Names the strip for assistive technology (e.g. "Resource views"). */
  label?: string;
  variant?: TabsVariant;
}

/**
 * Three looks, one control. Two of them wear CSS the design system already
 * had: the strip is `.tabstrip`/`.tab`, which `TabStrip` wears for the window's
 * document tabs, and the segmented control is `.seg`/`.seg-btn`, which
 * `CustomizeMark` wears for its radiogroup. Neither is restyled — a new
 * appearance for a control that already had one is how a design system stops
 * being one. (#331)
 *
 * `underline` is the third, and it is new CSS rather than a borrowed skin
 * because nothing in the design drew it before: the resource full tab shows
 * its panes as plain words on the page with an accent rule under the active
 * one. It was added when the design asked for it and not a moment earlier,
 * which is the only defence a third appearance has. The control underneath is
 * unchanged — same roles, same roving tabindex, same arrow keys.
 */
const SKIN: Record<TabsVariant, { list: string; tab: string }> = {
  strip: { list: "tabstrip", tab: "tab" },
  segmented: { list: "seg", tab: "seg-btn" },
  underline: { list: "utabs", tab: "utab" },
};

/**
 * Horizontal tab strip for switching views. Panels are rendered by callers, so
 * this owns only the list.
 *
 * The classic version wrapped Radix's Tabs, which supplied the keyboard
 * contract for free. Under the kit's no-dependency rule that has to be written
 * out, and it is not optional: a tablist where every tab is a Tab stop and the
 * arrow keys do nothing is a worse control than a row of buttons, because the
 * ARIA roles promise behaviour that is not there. So:
 *
 *   - roving tabindex — the strip is one Tab stop, the active tab is the one it
 *     lands on, and Tab from there moves past the strip rather than through it
 *   - Left/Right move between tabs, wrapping at both ends
 *   - Home/End jump to the first and last
 *
 * Selection follows focus, which is the expected pattern for tabs whose panels
 * are already rendered and cheap to switch. (#318)
 */
export function Tabs({ tabs, active, onChange, label, variant = "strip" }: TabsProps) {
  const refs = useRef(new Map<string, HTMLButtonElement>());
  const skin = SKIN[variant];

  function focus(id: string) {
    onChange(id);
    // The element for the newly active tab may not have its tabIndex updated
    // until React re-renders, but focus() does not care about tabIndex.
    refs.current.get(id)?.focus();
  }

  function onKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    // From the focused tab, not from `active`. A controlled parent may not have
    // committed the change yet — a deferred update, or a parent that validates
    // first — and computing from stale state sent the second arrow key off from
    // a tab the user had already left. Focus is the thing that actually moved.
    // (#323 review)
    const focused = tabs.findIndex((t) => refs.current.get(t.id) === document.activeElement);
    const index = focused >= 0 ? focused : tabs.findIndex((t) => t.id === active);
    if (index < 0) return;
    let next: number | null = null;
    if (event.key === "ArrowRight") next = (index + 1) % tabs.length;
    else if (event.key === "ArrowLeft") next = (index - 1 + tabs.length) % tabs.length;
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = tabs.length - 1;
    if (next === null) return;
    // Otherwise Left/Right also scroll the strip and Home/End jump the page.
    event.preventDefault();
    focus(tabs[next].id);
  }

  return (
    // `data-variant` is not only for the segmented styling: it is also what
    // tells the stylesheet these are panes rather than the window's documents,
    // and so lets `.tab` drop the 108px minimum a five-pane peek cannot afford.
    <div
      className={skin.list}
      data-variant={variant}
      role="tablist"
      aria-label={label}
      onKeyDown={onKeyDown}
    >
      {tabs.map((t) => (
        <button
          key={t.id}
          type="button"
          role="tab"
          className={skin.tab}
          data-active={t.id === active}
          aria-selected={t.id === active}
          tabIndex={t.id === active ? 0 : -1}
          ref={(node) => {
            if (node) refs.current.set(t.id, node);
            else refs.current.delete(t.id);
          }}
          onClick={() => onChange(t.id)}
        >
          {t.label}
        </button>
      ))}
    </div>
  );
}
