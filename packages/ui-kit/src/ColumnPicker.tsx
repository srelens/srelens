import { Popover } from "radix-ui";
import { Button } from "./Button";
import { cx } from "./cx";
import { usePortalContainer } from "./portal";
import { filled } from "./slot";

export interface ColumnOption {
  key: string;
  label: string;
}

export interface ColumnPickerProps {
  columns: ColumnOption[];
  hidden: ReadonlySet<string>;
  onToggle: (key: string) => void;
  /** The row identifier: always shown, and never offered as a toggle. */
  pinnedKey?: string;
  /** Text on the trigger. Empty leaves an icon-only button. */
  label?: string;
}

/**
 * Toolbar control for choosing which table columns are visible. Each column is
 * a checkbox; the `pinnedKey` column is the row identifier, so it is held on —
 * a table whose rows lost their name is not a table any more. Visibility is the
 * caller's state, passed in as the set of hidden keys and changed only through
 * `onToggle`, so the same layout can be persisted and applied to the table
 * itself. (#318)
 *
 * On Radix's Popover rather than shadcn's, which cannot come along: it lives in
 * `apps/desktop` and is written against the classic Tailwind config. Radix is
 * what shadcn's was wrapping, so this is the same behaviour with one layer
 * removed — the anchoring, the portal, the outside-click and Escape dismissal,
 * and the `aria-expanded`/`aria-controls` pair on the trigger. That contract is
 * the same one ConfirmDialog explains at length: it is library-sized, and
 * hand-writing it here would repeat a mistake this kit has already paid for.
 *
 * What is ours is the trigger's count and the pin. Both read the caller's
 * hidden set through the pin, because that set outlives the layout it was
 * written for — a persisted set can still name a column that has since become
 * the identifier, and neither the checkbox nor the count may believe it.
 *
 * Inside a portal scope — one tab of a window that holds several — the panel
 * mounts into the tab's own node rather than the document body, so it is hidden
 * with the tab. A portal escapes the `hidden` attribute an inactive tab wears,
 * so the column list opened over one table used to stay on screen over the next
 * tab, with the toolbar it belonged to already gone. Nothing else changes: this
 * popover is already non-modal and already dismisses on an outside interaction,
 * which is right for a panel. Outside a scope nothing changes at all. (#357)
 */
export function ColumnPicker({
  columns,
  hidden,
  onToggle,
  pinnedKey,
  label = "Columns",
}: ColumnPickerProps) {
  const hiddenCount = columns.filter((c) => c.key !== pinnedKey && hidden.has(c.key)).length;
  // `filled` rather than a truthiness check, so an empty label is the caller
  // asking for an icon-only button rather than a button with no name at all:
  // the visible text is the accessible name whenever there is any, since a name
  // matching what is on screen is what lets a speech-input user say it, and the
  // fallback appears only when nothing is on screen to say. (#318 review)
  const named = filled(label);
  const container = usePortalContainer();
  return (
    <Popover.Root>
      <Popover.Trigger asChild>
        {/* Explicitly type="button": the kit's Button deliberately does not
            default it, and this control sits in toolbars that sit inside
            forms — where a bare button submits on the click that opens it. */}
        <Button
          type="button"
          variant="ghost"
          size="sm"
          aria-label={named ? undefined : "Choose columns"}
        >
          {/* Inline rather than an icon-set import: the kit takes no dependency
              on lucide, and this is the only glyph it needs. */}
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" aria-hidden="true">
            <rect
              x="3"
              y="3"
              width="18"
              height="18"
              rx="2"
              stroke="currentColor"
              strokeWidth="2"
            />
            <path d="M9 3v18M15 3v18" stroke="currentColor" strokeWidth="2" />
          </svg>
          {named && label}
          {/* Left in the accessible name rather than hidden from it: how many
              columns survived the last visit is the reason to open the panel,
              and it is no less useful read aloud than seen. */}
          {hiddenCount > 0 && (
            <span className="tabular-nums opacity-70">({columns.length - hiddenCount})</span>
          )}
        </Button>
      </Popover.Trigger>
      <Popover.Portal container={container}>
        <Popover.Content
          align="end"
          sideOffset={6}
          className="popover p-1"
          // `.popover` is written for a panel that places itself: `position:
          // fixed`. Radix already fixes and translates a wrapper around this
          // content, and a fixed child leaves that wrapper zero-sized — which
          // is the box the collision logic measures, so the panel would flip
          // and shift against nothing. Relative rather than static keeps the
          // stylesheet's z-index doing its job. (#318 review)
          style={{ position: "relative" }}
        >
          <div role="group" aria-label="Toggle columns" className="flex flex-col">
            {columns.map((column) => {
              const pinned = column.key === pinnedKey;
              const checked = pinned || !hidden.has(column.key);
              return (
                <label
                  key={column.key}
                  // `.ns-row` is the design's row inside a popover, and
                  // `data-on` is how it marks the live one — so a visible
                  // column reads stronger than a hidden one without a second
                  // rule being invented for this panel.
                  className={cx("ns-row rounded", pinned && "opacity-60")}
                  data-on={checked}
                  // On the row, not only the input: the row is what a pointer
                  // and a screen reader both meet first, and the disabled
                  // checkbox inside it says nothing about why.
                  aria-disabled={pinned || undefined}
                >
                  <input
                    type="checkbox"
                    checked={checked}
                    disabled={pinned}
                    // Guarded as well as disabled. `disabled` is the browser
                    // enforcing the pin, and it enforces it well — a disabled
                    // control takes no click and no focus. But the invariant
                    // belongs to this component, not to the attribute: swap
                    // `disabled` for `aria-disabled` one day, as an a11y
                    // review may well ask for, and the identifier column
                    // becomes toggleable with nothing failing. (#318 review)
                    onChange={() => {
                      if (!pinned) onToggle(column.key);
                    }}
                    style={{ accentColor: "var(--accent)" }}
                  />
                  <span>{column.label}</span>
                </label>
              );
            })}
          </div>
        </Popover.Content>
      </Popover.Portal>
    </Popover.Root>
  );
}
