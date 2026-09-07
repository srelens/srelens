import { useRef, type KeyboardEvent, type ReactNode } from "react";
import { ContextMenu, type ContextMenuItem } from "./ContextMenu";
import { cx } from "./cx";
import type { IconComponent } from "./IconButton";
import { filled } from "./slot";
import { Tooltip } from "./Tooltip";
import { toneColor, type Tone } from "./tone";

export interface ClusterRailMarker {
  /**
   * What the dot means, in words — "Team connection", "Degraded". It joins the
   * mark's accessible name, so the dot is a second channel rather than the only
   * one.
   */
  label: string;
  tone?: Tone;
}

export interface ClusterRailItem {
  id: string;
  /** The spoken name, and the caption under the mark when captions are on. */
  name: string;
  /** The mark itself: a monogram, a symbol, an uploaded logo — the app's call. */
  mark: ReactNode;
  /** The second half of the hint: provider, version, how it is reached. */
  detail?: string;
  /** Small dots on the corner of the mark, each one naming itself. */
  markers?: ClusterRailMarker[];
  /**
   * The colour of the active indicator, so it can match the mark the app drew.
   * Any CSS colour; a token is the expected form. Left out, it is the accent.
   */
  color?: string;
  /**
   * Why this cluster is out of reach — "Disconnected". Given, the mark dims and
   * the word joins its name; a string rather than a flag so the state cannot be
   * told in opacity alone.
   */
  unavailable?: string;
  /** Items sharing one sit together; a change draws a rule between them. */
  group?: string;
}

export interface ClusterRailProps {
  /** Already filtered and ordered: the rail shows what it is given. */
  items: ClusterRailItem[];
  activeId?: string;
  onSelect: (id: string) => void;
  /** The double-click gesture — usually "focus this cluster and nothing else". */
  onOpen?: (id: string) => void;
  /**
   * The right-click menu for one cluster. An empty list means this cluster
   * answers no right-click, and the browser's own menu is left alone.
   *
   * A list of items rather than the event this used to hand back: a menu is
   * anchored, portalled, arrowed through and dismissed, and none of that can be
   * done by a caller holding a coordinate — see {@link ContextMenu}. Same seam
   * as `TabStrip.menuFor`, for the same reason: the rail knows a mark can be
   * right-clicked, not what the product offers when it is.
   */
  menuFor?: (item: ClusterRailItem) => ContextMenuItem[];
  /** Print the name under each mark. Off by default: the mark carries it. */
  showNames?: boolean;
  /** How wide the marks are, in px. The rail sizes itself and its add tile to match. */
  markSize?: number;
  /** Offered as a dashed tile after the marks. Absent, so is the tile. */
  onAdd?: () => void;
  addLabel?: string;
  /** The glyph in the add tile, if the app would rather use its own icon set. */
  addIcon?: IconComponent;
  emptyLabel?: string;
  /** Something went wrong assembling the list; shown as a warning at the top. */
  error?: string;
  /** Anything the app wants at the foot — a preferences popover, usually. */
  footer?: ReactNode;
  /** Names the landmark. */
  label?: string;
  className?: string;
}

/** Below this the marks are unreadable; above it the rail is a sidebar. */
const MIN_MARK = 16;
const MAX_MARK = 64;
const DEFAULT_MARK = 30;

function clampMark(size: number | undefined): number {
  if (size === undefined || !Number.isFinite(size)) return DEFAULT_MARK;
  return Math.min(Math.max(size, MIN_MARK), MAX_MARK);
}

/**
 * Inline rather than an icon-set import: the kit takes no dependency on lucide,
 * and a plus is a structural affordance rather than product vocabulary. An app
 * that wants its own passes `addIcon`.
 */
function PlusGlyph({ size = 13, className }: { size?: number; className?: string }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" className={className} aria-hidden="true">
      <path d="M12 5v14M5 12h14" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
    </svg>
  );
}

function WarningGlyph() {
  return (
    <svg width="13" height="13" viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <path
        d="M10.3 3.9 1.8 18a2 2 0 0 0 1.7 3h17a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0zM12 9v4M12 17h.01"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

/**
 * The vertical strip of cluster marks down the edge of the window: which
 * clusters this workspace holds, which one is live, and the way into any of the
 * others.
 *
 * The mock read all of that for itself. It pulled the cluster list out of a data
 * module, the marks and the rail's own preferences out of a hotbar store, the
 * active id and the connection states out of a workspace store, and the
 * workspace's membership out of a tabs store — then wrote back through
 * `setActiveCluster`, `openTab`, `focusCluster` and `setRailPrefs`. None of that
 * can come along: the kit may not import `@srelens/core`, and a design system
 * that owns application state is a design system with one call site. So `items`
 * arrives already filtered, ordered and resolved, every gesture leaves as a
 * callback, and the two things the mock opened for itself — a context menu and a
 * preferences popover — become `menuFor` and the `footer` slot, because both are
 * built from the app's own vocabulary of routes and actions. This is the line
 * `NavIcon`, `MultiSelect` and `CodeEditor` each drew before it. (#320)
 *
 * `mark` is a node rather than an {@link IconComponent} because a cluster's mark
 * is not always an icon: the design lets it be initials, a preset symbol, or an
 * image the user uploaded, and only the app knows which. What the rail keeps is
 * the box — every mark is laid out at `markSize` square, so the active
 * indicator, the marker dots and the add tile all line up whatever is inside.
 *
 * Three states the mock told in colour alone now have to be told in words.
 * A team connection and a degraded cluster were unlabelled dots, and a
 * disconnected one was 42% opacity; a dot is a {@link ClusterRailMarker} whose
 * `label` joins the mark's accessible name, and the dimming is bought by naming
 * the reason in `unavailable`. A reader who cannot separate the accent from the
 * severity colour, or who hears the rail rather than sees it, gets the same
 * three facts.
 *
 * The width was `chipSizes[prefs.size] + (showLabels ? 30 : 16)` over a number
 * restored from `localStorage`, which is a stranger's arithmetic: a stored `0`
 * left an invisible rail and a stored four-digit number left one half the window
 * wide, neither recoverable from inside the app. `markSize` is clamped, and a
 * value that is not a number falls back rather than propagating `NaN` into a
 * style.
 *
 * Arrow keys move focus along the rail, and Home and End reach its ends. Every
 * mark stays a Tab stop — this is a `nav`, and a nav's items are all Tab stops —
 * so the arrows are a shortcut through a dozen marks rather than a roving
 * tabindex, which would be the wrong contract for a landmark. The right-click
 * gesture needs no keyboard twin: browsers raise `contextmenu` for Shift+F10 and
 * the Menu key, so `menuFor`'s menu is already reachable without a pointer,
 * which is what makes it safe for `onOpen` — a double-click, and nothing else —
 * to be the shortcut it is rather than the only route to what it does.
 *
 * The hint is a {@link Tooltip} rather than the mock's `title`, which never
 * appeared on focus and was a two-line string with an escaped newline in it. It
 * is worth having on every mark rather than only the detailed ones: with
 * captions off, a monogram is all there is to read.
 */
export function ClusterRail({
  items,
  activeId,
  onSelect,
  onOpen,
  menuFor,
  showNames = false,
  markSize,
  onAdd,
  addLabel = "Connect a cluster",
  addIcon: AddIcon = PlusGlyph,
  emptyLabel = "No clusters",
  error,
  footer,
  label = "Clusters",
  className,
}: ClusterRailProps) {
  const refs = useRef(new Map<string, HTMLButtonElement>());
  const mark = clampMark(markSize);
  // The caption is 30px of room rather than 16, which is the gap the marks need
  // either side of them.
  const width = mark + (showNames ? 30 : 16);

  function onKeyDown(event: KeyboardEvent<HTMLElement>) {
    // From the focused mark, not from `activeId`: the arrows move focus without
    // selecting, so the two are routinely on different clusters.
    const index = items.findIndex((item) => refs.current.get(item.id) === document.activeElement);
    if (index < 0) return;
    let next: number | null = null;
    if (event.key === "ArrowDown") next = (index + 1) % items.length;
    else if (event.key === "ArrowUp") next = (index - 1 + items.length) % items.length;
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = items.length - 1;
    if (next === null) return;
    // Otherwise the rail scrolls under the focus it just moved.
    event.preventDefault();
    refs.current.get(items[next].id)?.focus();
  }

  // `filled` rather than `error != null`: `error={failed && message}` is how a
  // caller makes it conditional, and `false` would buy a warning glyph with no
  // words in it. The second comparison is TypeScript's rather than ours —
  // `filled` answers with a boolean, not a type guard.
  const failure = filled(error) ? error : undefined;

  return (
    <nav
      aria-label={label}
      onKeyDown={onKeyDown}
      className={cx("rule-r flex shrink-0 flex-col items-center gap-1.5 bg-canvas-deep py-2", className)}
      style={{ width }}
    >
      {failure !== undefined && (
        <Tooltip label={failure} side="right">
          {/* An image with a name rather than a bare glyph: the failure is the
              whole message, and there is no room in a 46px rail to write it out.
              Focusable, so the hint is reachable without a pointer. */}
          <span
            role="img"
            aria-label={failure}
            tabIndex={0}
            data-slot="error"
            className="inline-flex"
            style={{ color: toneColor("sev") }}
          >
            <WarningGlyph />
          </span>
        </Tooltip>
      )}

      {items.length === 0 ? (
        // Wrapping rather than truncating: "No clusters" has to survive a rail
        // this narrow, and it is the one line here that is a sentence.
        <p className="px-1 text-center text-[0.5625rem] leading-tight text-faint">{emptyLabel}</p>
      ) : (
        <ul className="flex w-full flex-col items-center gap-1.5">
          {items.map((item, index) => {
            const active = item.id === activeId;
            const markers = item.markers ?? [];
            const away = filled(item.unavailable);
            const spoken = [item.name, item.unavailable, ...markers.map((m) => m.label)]
              .filter((part) => filled(part))
              .join(", ");
            const hint = [item.name, item.detail].filter((part) => filled(part)).join(" — ");
            const menu = menuFor?.(item) ?? [];

            const node = (
              <li key={item.id} className="flex w-full flex-col items-center gap-1.5">
                {/* The rule belongs to the item that starts a new run, so the
                    list stays one list — a reader hears how many clusters there
                    are before arrowing through them. */}
                {index > 0 && items[index - 1].group !== item.group && (
                  <span data-slot="group-rule" aria-hidden="true" className="my-0.5 h-px w-5 bg-rule-strong" />
                )}
                <Tooltip label={hint} side="right">
                  <button
                    type="button"
                    ref={(node) => {
                      if (node) refs.current.set(item.id, node);
                      else refs.current.delete(item.id);
                    }}
                    aria-label={spoken}
                    aria-current={active || undefined}
                    data-unavailable={away || undefined}
                    onClick={() => onSelect(item.id)}
                    onDoubleClick={onOpen ? () => onOpen(item.id) : undefined}
                    className="relative flex w-full flex-col items-center gap-0.5"
                    style={away ? { opacity: 0.42 } : undefined}
                  >
                    <span
                      className="relative inline-flex shrink-0 items-center justify-center"
                      style={{ width: mark, height: mark }}
                    >
                      {active && (
                        <span
                          aria-hidden="true"
                          className="absolute top-1/2 h-4 w-[3px] -translate-y-1/2 rounded-full"
                          style={{ left: -9, background: item.color ?? "var(--accent)" }}
                        />
                      )}
                      {item.mark}
                      {markers.length > 0 && (
                        <span aria-hidden="true" className="absolute -bottom-0.5 -right-0.5 flex gap-px">
                          {markers.map((marker) => (
                            <span
                              key={marker.label}
                              data-slot="marker"
                              className="h-[7px] w-[7px] rounded-full ring-2 ring-canvas-deep"
                              style={{ background: toneColor(marker.tone ?? "accent") }}
                            />
                          ))}
                        </span>
                      )}
                    </span>
                    {showNames && (
                      <span
                        data-slot="caption"
                        className={cx(
                          "w-full truncate text-center text-[0.5625rem] leading-tight",
                          active ? "font-semibold text-ink" : "text-faint",
                        )}
                      >
                        {item.name}
                      </span>
                    )}
                  </button>
                </Tooltip>
              </li>
            );

            // The `li` is the trigger, rather than a wrapper around it: Radix
            // hands the ref and the gesture to whatever single element it is
            // given, and a `span` wrapped around an `li` is not markup a list
            // will keep. Nothing measures it — a context menu anchors to the
            // pointer, not to its trigger — so the rail's layout is untouched.
            // An empty list is left unwrapped so the browser's own menu still
            // opens; wrapping would take it away and give back an empty box.
            return menu.length > 0 ? (
              <ContextMenu key={item.id} items={menu} label={`${item.name} actions`}>
                {node}
              </ContextMenu>
            ) : (
              node
            );
          })}
        </ul>
      )}

      {onAdd && (
        <Tooltip label={addLabel} side="right">
          <button
            type="button"
            aria-label={addLabel}
            onClick={onAdd}
            className="mt-1 flex shrink-0 items-center justify-center rounded-[7px] border border-dashed border-rule-strong text-faint transition-colors hover:text-ink"
            style={{ width: mark, height: mark }}
          >
            <AddIcon size={13} aria-hidden="true" />
          </button>
        </Tooltip>
      )}

      <span className="flex-1" aria-hidden="true" />

      {filled(footer) && <div data-slot="footer">{footer}</div>}
    </nav>
  );
}
