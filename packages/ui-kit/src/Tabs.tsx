import { useRef, type KeyboardEvent } from "react";

export interface TabItem {
  id: string;
  label: string;
}

export interface TabsProps {
  tabs: TabItem[];
  active: string;
  onChange: (id: string) => void;
  /** Names the strip for assistive technology (e.g. "Resource views"). */
  label?: string;
}

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
export function Tabs({ tabs, active, onChange, label }: TabsProps) {
  const refs = useRef(new Map<string, HTMLButtonElement>());

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
    <div className="tabstrip" role="tablist" aria-label={label} onKeyDown={onKeyDown}>
      {tabs.map((t) => (
        <button
          key={t.id}
          type="button"
          role="tab"
          className="tab"
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
