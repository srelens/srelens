import { describe, it, expect, vi, afterEach } from "vitest";
import { registerSelectAllTarget, type SelectAllTarget } from "./selectAll";

/**
 * The chord, as the browser delivers it: a keydown on whatever has focus —
 * the body, when the reader has only scrolled and read.
 */
function pressSelectAll(
  on: EventTarget = document.body,
  init: KeyboardEventInit = {},
): KeyboardEvent {
  const event = new KeyboardEvent("keydown", {
    key: "a",
    metaKey: true,
    bubbles: true,
    cancelable: true,
    ...init,
  });
  on.dispatchEvent(event);
  return event;
}

const undo: Array<() => void> = [];

/** A pane on screen, with a spy for the selection it would make. */
function pane(dom: HTMLElement | null = document.createElement("div")): {
  target: SelectAllTarget;
  selectAll: ReturnType<typeof vi.fn>;
} {
  if (dom && !dom.isConnected) document.body.append(dom);
  const selectAll = vi.fn();
  const target: SelectAllTarget = { dom: () => dom, selectAll };
  undo.push(registerSelectAllTarget(target));
  return { target, selectAll };
}

afterEach(() => {
  for (const off of undo.splice(0)) off();
  document.body.replaceChildren();
});

describe("select-all over a code pane", () => {
  it("selects the document in the one pane on screen", () => {
    // The whole point. A CodeMirror document lives in a `contenteditable`, and
    // the browser's own select-all takes the page AROUND one — so the manifest
    // view put every label and table row beside the YAML on the clipboard, and
    // no YAML. (#656)
    const { selectAll } = pane();
    const event = pressSelectAll();
    expect(selectAll).toHaveBeenCalledOnce();
    expect(event.defaultPrevented).toBe(true);
  });

  it("answers Ctrl-A as well as ⌘A", () => {
    const { selectAll } = pane();
    pressSelectAll(document.body, { metaKey: false, ctrlKey: true });
    expect(selectAll).toHaveBeenCalledOnce();
  });

  it("leaves a bare 'a' alone", () => {
    const { selectAll } = pane();
    const event = pressSelectAll(document.body, { metaKey: false });
    expect(selectAll).not.toHaveBeenCalled();
    expect(event.defaultPrevented).toBe(false);
  });

  it("leaves ⌘⇧A and ⌥⌘A alone", () => {
    // Neighbouring chords an app or the OS may own. Answering a chord because
    // two of its three keys matched is worse than not answering it.
    const { selectAll } = pane();
    pressSelectAll(document.body, { shiftKey: true });
    pressSelectAll(document.body, { altKey: true });
    expect(selectAll).not.toHaveBeenCalled();
  });

  it("leaves the chord to whoever is being typed in", () => {
    // A filter box, a name field: ⌘A there means "select this field", and
    // taking it away would be a regression the reader feels on every screen.
    const { selectAll } = pane();
    const input = document.createElement("input");
    document.body.append(input);
    const event = pressSelectAll(input);
    expect(selectAll).not.toHaveBeenCalled();
    expect(event.defaultPrevented).toBe(false);
  });

  it("leaves the chord to an editor that already has focus", () => {
    // CodeMirror's own keymap answers it there, and its content is a
    // `contenteditable` — the one case the browser gets right on its own.
    const { selectAll } = pane();
    const editable = document.createElement("div");
    editable.setAttribute("contenteditable", "true");
    document.body.append(editable);
    pressSelectAll(editable);
    expect(selectAll).not.toHaveBeenCalled();
  });

  it("does not answer a chord something else already answered", () => {
    const { selectAll } = pane();
    // On capture, so it lands ahead of the module's own bubble-phase listener
    // — the order a real handler that owns the chord would have.
    document.addEventListener("keydown", (e) => e.preventDefault(), { once: true, capture: true });
    pressSelectAll();
    expect(selectAll).not.toHaveBeenCalled();
  });

  it("does nothing when two panes are on screen", () => {
    // Nothing here can tell which one the reader meant, and guessing copies
    // the wrong document without saying so. The chord falls through to the
    // browser instead, exactly as it did before.
    const first = pane();
    const second = pane();
    const event = pressSelectAll();
    expect(first.selectAll).not.toHaveBeenCalled();
    expect(second.selectAll).not.toHaveBeenCalled();
    expect(event.defaultPrevented).toBe(false);
  });

  it("counts only the pane the reader can see", () => {
    // A drawer's three tabs each hold a pane; two of them are behind a
    // `hidden` or an `inert` and answering from one of those would select a
    // document that is not on screen — and leave the visible one out.
    const hidden = document.createElement("div");
    hidden.hidden = true;
    const buried = document.createElement("div");
    hidden.append(buried);
    document.body.append(hidden);
    const covered = pane(buried);
    const shown = pane();
    pressSelectAll();
    expect(covered.selectAll).not.toHaveBeenCalled();
    expect(shown.selectAll).toHaveBeenCalledOnce();
  });

  it("counts only a pane that is still in the document", () => {
    const gone = document.createElement("div");
    document.body.append(gone);
    const detached = pane(gone);
    gone.remove();
    const shown = pane();
    pressSelectAll();
    expect(detached.selectAll).not.toHaveBeenCalled();
    expect(shown.selectAll).toHaveBeenCalledOnce();
  });

  it("counts nothing for a pane that has not mounted yet", () => {
    const { selectAll } = pane(null);
    pressSelectAll();
    expect(selectAll).not.toHaveBeenCalled();
  });

  it("stops listening once the last pane unregisters", () => {
    // The listener is refcounted rather than installed on import: a module
    // that adds a document listener on load adds it in every suite that
    // touches the kit, and in a host that never renders an editor at all.
    const { selectAll } = pane();
    undo.pop()?.();
    const event = pressSelectAll();
    expect(selectAll).not.toHaveBeenCalled();
    expect(event.defaultPrevented).toBe(false);
  });
});
