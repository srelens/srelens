import type { ReactNode } from "react";
import { ContextMenu as Menu } from "radix-ui";
import type { IconComponent } from "./IconButton";
import { usePortalContainer, usePortalScoped } from "./portal";

export type ContextMenuItem =
  | { kind: "sep" }
  | {
      kind?: "item";
      label: string;
      /**
       * Structural rather than lucide's `LucideIcon`, the way
       * {@link IconComponent} is everywhere else in the kit: the design system
       * does not take a dependency on an icon set to describe a hole an icon
       * goes in.
       */
      icon?: IconComponent;
      /** The keystroke that does the same thing, shown pinned to the row's end. */
      hint?: string;
      /** Tints the row with the danger colour, over a label that already says so. */
      danger?: boolean;
      /** Keep the menu open while transient feedback replaces this row's label. */
      closeOnPick?: boolean;
      onPick: () => void;
    };

export interface ContextMenuProps {
  items: ContextMenuItem[];
  /**
   * The region that answers a right-click — a table row, a tab, a tree node.
   *
   * A single element that takes a ref, because the handling is attached to that
   * element rather than to a wrapper around it: a `<span>` wrapped around a
   * `<tr>` is not markup a table will keep.
   */
  children: ReactNode;
  /** Names the menu for assistive technology, e.g. "Tab actions". */
  label?: string;
  /**
   * Fires as the menu opens and closes. The mock's call sites used the close
   * half of this to drop their own coordinate state; what is left for it is
   * everything else they did on the way in, such as selecting the row that was
   * right-clicked.
   */
  onOpenChange?: (open: boolean) => void;
}

/**
 * The right-click menu a desktop table, tab strip and tree are each expected to
 * have: a short list of actions on the thing under the pointer, with the
 * keystroke that does the same thing shown alongside.
 *
 * Built on Radix's ContextMenu rather than by hand, for the reason
 * {@link ConfirmDialog} sets out at length — a menu is roving focus, typeahead,
 * Escape, outside-click dismissal, pointer-anchored and collision-aware
 * placement, and a portal that escapes whatever clipped container the row lives
 * in. The mock hand-wrote two of those and got both wrong: placement was a pair
 * of `Math.min` clamps against a menu size guessed at 240×300 that never
 * flipped and could go negative in a small window, and dismissal was three
 * window listeners attached a task late to dodge the click that opened it. It
 * is library-sized work, and the library is already a dependency.
 *
 * Radix's ContextMenu and not its DropdownMenu: every call site opens this from
 * `onContextMenu` on a row rather than from a button, and the two differ
 * exactly there — ContextMenu anchors to the pointer, answers long-press on
 * touch and the keyboard's own menu key, where DropdownMenu anchors to a
 * trigger the design does not draw.
 *
 * That is also why the API lost the mock's `x`/`y` and gained `children`: the
 * caller no longer tracks a coordinate and an open flag, it wraps the region
 * and Radix does the rest. What stays ours is the item vocabulary, the icon
 * column that holds its width so the labels line up, and keeping the shortcut
 * hint out of each item's accessible name. (#320)
 *
 * Inside a portal scope — one tab of a window that holds several — the menu
 * belongs to that tab twice over. It mounts into the tab's own node, so it is
 * hidden with the tab instead of following the reader to the next one, and it
 * stops being modal, because Radix's menu is modal by default and that took the
 * whole document out of the accessibility tree and switched the document's
 * pointer events off — the tab strip and the cluster rail with them. Both
 * matter, and the second matters more once the first lands: a menu left open on
 * a tab the reader has switched away from is invisible and would otherwise
 * still be holding the window. Outside a scope nothing changes. (#357)
 */
export function ContextMenu({ items, children, label, onOpenChange }: ContextMenuProps) {
  const container = usePortalContainer();
  // Not `container === undefined`, which is the same answer for all but one
  // render and the wrong question in every one of them: undefined means
  // "document.body" outside a surface and "the node has not arrived yet"
  // inside one, and the render between a surface mounting and its ref firing
  // is the second. Read as the first, that render is a window-wide modal
  // inside a tab that has one. (#357 review)
  const scoped = usePortalScoped();
  return (
    <Menu.Root onOpenChange={onOpenChange} modal={!scoped}>
      <Menu.Trigger asChild>{children}</Menu.Trigger>
      <Menu.Portal container={container}>
        <Menu.Content
          aria-label={label}
          className="ctx-menu"
          // `.ctx-menu` is written for a menu that places itself: `position:
          // fixed`. Radix already fixes and translates a wrapper around this
          // content, and a fixed child leaves that wrapper zero-sized — which
          // is the box the collision logic measures, so the menu would flip and
          // shift against nothing. Relative rather than static keeps the
          // stylesheet's z-index doing its job. (#320)
          style={{ position: "relative" }}
        >
          {items.map((item, index) =>
            item.kind === "sep" ? (
              <Menu.Separator key={index} className="ctx-sep" />
            ) : (
              <Menu.Item
                key={index}
                className="ctx-item"
                data-danger={item.danger ? "true" : undefined}
                // Named explicitly, because the name is otherwise computed from
                // everything inside the row — so the shortcut is read out as
                // part of the action ("Close tab command W"), which is neither
                // what the item does nor what a speech-input user would say to
                // reach it. (#320)
                aria-label={item.label}
                onSelect={(event) => {
                  if (item.closeOnPick === false) event.preventDefault();
                  item.onPick();
                }}
              >
                {/* Always taken, filled or not: the design lines the labels up
                    in a column, and an icon present on only some items shoves
                    the rest of them sideways.

                    Hidden here rather than on the icon: the icon is the
                    caller's component, and one that drops the `aria-hidden` it
                    is handed — a plain `<svg>{...props}` forgets it easily —
                    would put a nameless graphic inside the item. The slot is
                    ours, so the guarantee is ours. (#320) */}
                <span
                  data-icon-slot
                  aria-hidden="true"
                  className="flex w-[13px] shrink-0 items-center justify-center"
                >
                  {item.icon && <item.icon size={13} aria-hidden="true" />}
                </span>
                <span className="flex-1 truncate text-left">{item.label}</span>
                {item.hint && (
                  <span className="ctx-hint" aria-hidden="true">
                    {item.hint}
                  </span>
                )}
              </Menu.Item>
            ),
          )}
        </Menu.Content>
      </Menu.Portal>
    </Menu.Root>
  );
}
