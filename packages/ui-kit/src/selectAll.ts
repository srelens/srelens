/**
 * ⌘A / Ctrl-A over a code pane the reader never clicked into.
 *
 * A CodeMirror document lives in a `contenteditable`, and the browser's own
 * select-all refuses to reach inside one it is not already editing: pressed
 * with focus on the body, WebKit and Chromium select the whole page *except*
 * the editor. On the manifest view that is the exact inverse of what was
 * asked for — every label, tab and table row around the YAML lands on the
 * clipboard and the YAML does not. (#656)
 *
 * So the chord is answered here instead: find the one code pane actually on
 * screen, put focus in it, and select its whole document — after which ⌘C is
 * CodeMirror's own copy, which serialises from the editor STATE rather than
 * from the handful of lines a virtualised view has rendered.
 *
 * Nothing here knows about CodeMirror. A caller registers two closures — where
 * it is, and how to select it — so this module stays a plain listener the
 * classic editor can share with the kit's without dragging the kit's
 * components into a classic boot's chunk.
 */

/**
 * A code pane that can answer the chord.
 *
 * `dom` is read at keypress rather than captured once: the element is created
 * imperatively after mount, and replaced outright whenever a structural option
 * changes the editor is rebuilt for.
 */
export interface SelectAllTarget {
  /** The pane's element, or null before it is mounted. */
  dom: () => HTMLElement | null;
  /** Focus the pane and select every character in it. */
  selectAll: () => void;
}

/**
 * Where a keystroke means "select this field", not "select the view".
 *
 * A reader typing in a filter box, a name field, or an editor they have
 * already clicked into is served correctly by the browser (or by CodeMirror's
 * own keymap), and taking the chord off them would be a regression. The
 * read-only panes this module exists for are the case that falls through:
 * their content carries `contenteditable="false"`, which this selector
 * deliberately does not match.
 */
const TYPING_TARGET = 'input, textarea, select, [contenteditable]:not([contenteditable="false"])';

/** Covered by a layer, or in a tab that is not the one on top. */
const OUT_OF_VIEW = '[hidden], [inert], [aria-hidden="true"]';

const targets = new Set<SelectAllTarget>();

/**
 * Whether this pane is one the reader can see right now.
 *
 * `getClientRects()` is empty for anything inside a `display: none` subtree,
 * which is how an inactive tab's pane hides — the case that has to be excluded
 * or a drawer with three tabs would answer the chord from whichever pane
 * mounted first. jsdom lays nothing out and reports no rects for *anything*,
 * so an empty list is read as "hidden" only when the page around it has a
 * layout at all; otherwise there is nothing to conclude and the pane counts.
 */
function onScreen(el: HTMLElement | null): el is HTMLElement {
  if (!el?.isConnected) return false;
  if (el.closest(OUT_OF_VIEW)) return false;
  return el.getClientRects().length > 0 || document.body.getClientRects().length === 0;
}

/**
 * Answer the chord, or leave it alone.
 *
 * **Two panes on screen is not an answer.** Nothing here can tell which one
 * the reader meant, and guessing would copy the wrong document without saying
 * so — so the chord falls through to the browser exactly as it does today.
 * One pane is the shape every surface in this app actually has: the manifest
 * view, the editor, the Helm values pane, each alone in its region.
 */
function onKeyDown(event: KeyboardEvent): void {
  // Already answered — by CodeMirror's own keymap in a pane that has focus, or
  // by anything else that got there first.
  if (event.defaultPrevented) return;
  if (event.key !== "a" && event.key !== "A") return;
  if (!event.metaKey && !event.ctrlKey) return;
  if (event.altKey || event.shiftKey) return;
  const target = event.target;
  if (target instanceof Element && target.closest(TYPING_TARGET)) return;
  const visible = [...targets].filter((t) => onScreen(t.dom()));
  if (visible.length !== 1) return;
  event.preventDefault();
  visible[0].selectAll();
}

/**
 * Register a code pane for the lifetime of a component; returns the undo.
 *
 * The listener is refcounted rather than installed once at import: a module
 * that adds a document listener on load adds it in every test file that
 * touches the kit, and in a host that never renders an editor at all.
 */
export function registerSelectAllTarget(target: SelectAllTarget): () => void {
  if (targets.size === 0) document.addEventListener("keydown", onKeyDown);
  targets.add(target);
  return () => {
    targets.delete(target);
    if (targets.size === 0) document.removeEventListener("keydown", onKeyDown);
  };
}
