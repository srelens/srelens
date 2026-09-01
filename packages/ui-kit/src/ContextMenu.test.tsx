import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { ContextMenu, type ContextMenuItem } from "./ContextMenu";
import { PortalScopeProvider, usePortalHost } from "./portal";

// jsdom has no ResizeObserver, and Radix's popper watches the trigger and the
// content with one. The kit's shared setup does not stub it, and that setup is
// not this file's to edit, so the stub lives here. Inert: jsdom does no layout,
// so there is never a resize to report. (ColumnPicker.test.tsx carries the same
// stub, as does apps/desktop's setup.)
if (!("ResizeObserver" in globalThis)) {
  (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}

function Trash({ className }: { className?: string }) {
  return (
    <svg data-testid="trash" className={className} viewBox="0 0 24 24">
      <path d="M4 7h16" />
    </svg>
  );
}

const ITEMS: ContextMenuItem[] = [
  { label: "Duplicate tab", onPick: () => {} },
  { label: "Pin tab", onPick: () => {} },
  { kind: "sep" },
  { label: "Close tab", hint: "⌘W", onPick: () => {} },
  { label: "Close all tabs", icon: Trash, danger: true, onPick: () => {} },
];

function setup(props: Partial<Parameters<typeof ContextMenu>[0]> = {}) {
  return render(
    <ContextMenu items={ITEMS} label="Tab actions" {...props}>
      <button type="button">checkout-api</button>
    </ContextMenu>,
  );
}

const region = () => screen.getByText("checkout-api");

async function open(props: Partial<Parameters<typeof ContextMenu>[0]> = {}) {
  const view = setup(props);
  fireEvent.contextMenu(region());
  await screen.findByRole("menu");
  return view;
}

/**
 * What this component owns: the item vocabulary it accepts, how each kind of
 * item is drawn, what a pick reports back, and its wiring to Radix's
 * ContextMenu.
 *
 * Deliberately absent: roving focus, typeahead, outside-click dismissal and
 * collision-aware placement. Those are the library's — the mock hand-wrote a
 * fraction of them and got the fraction wrong, which is the whole reason this
 * wraps Radix. Asserting a dependency's internals through our component pins
 * the version we happen to have rather than the behaviour we promise. Escape is
 * here because losing it would be a real regression for the one user who has no
 * other way out. (#320)
 */
describe("ContextMenu", () => {
  it("shows the region it guards and nothing else until asked", () => {
    setup();
    expect(region()).toBeDefined();
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("opens on a right-click on that region", async () => {
    await open();
    expect(screen.getByRole("menu")).toBeDefined();
  });

  it("lists the items in the order given", async () => {
    await open();
    const labels = screen.getAllByRole("menuitem").map((item) => item.textContent);
    expect(labels).toEqual(["Duplicate tab", "Pin tab", "Close tab⌘W", "Close all tabs"]);
  });

  it("reports the pick and closes", async () => {
    const onPick = vi.fn();
    await open({ items: [{ label: "Duplicate tab", onPick }] });
    fireEvent.click(screen.getByRole("menuitem", { name: "Duplicate tab" }));
    expect(onPick).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(screen.queryByRole("menu")).toBeNull());
  });

  it("can keep a feedback-bearing item visible after it is picked", async () => {
    const onPick = vi.fn();
    await open({ items: [{ label: "Copy", closeOnPick: false, onPick }] });
    fireEvent.click(screen.getByRole("menuitem", { name: "Copy" }));
    expect(onPick).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("menu")).toBeDefined();
  });

  it("draws a separator that is not itself an item", async () => {
    await open();
    const menu = screen.getByRole("menu");
    expect(within(menu).getAllByRole("separator")).toHaveLength(1);
    expect(within(menu).getAllByRole("menuitem")).toHaveLength(4);
  });

  it("tells the caller when it opens and closes", async () => {
    const onOpenChange = vi.fn();
    await open({ onOpenChange });
    expect(onOpenChange).toHaveBeenCalledWith(true);
    fireEvent.keyDown(document.activeElement ?? document.body, { key: "Escape" });
    await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false));
  });

  it("closes on Escape", async () => {
    await open();
    fireEvent.keyDown(document.activeElement ?? document.body, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("menu")).toBeNull());
  });
});

/**
 * The mock drew a bare `<div>` of `<button>`s: no role, no name, no portal, and
 * a shortcut hint swallowed into every item's accessible name. Each of those is
 * corrected here.
 */
describe("ContextMenu announces itself", () => {
  it("carries the name it was given", async () => {
    await open();
    expect(screen.getByRole("menu", { name: "Tab actions" })).toBeDefined();
  });

  it("keeps the shortcut visible but out of the item's name", async () => {
    // The hint is a reminder of the keystroke, not part of what the item is
    // called; folded into the name it becomes "Close tab command W", which is
    // neither what the item does nor what a speech-input user would say.
    await open();
    const item = screen.getByRole("menuitem", { name: "Close tab" });
    expect(within(item).getByText("⌘W")).toBeDefined();
  });

  it("treats icons as decoration", async () => {
    // Hidden by the slot around it rather than by asking the icon to hide
    // itself: the icon is the caller's component, and one that quietly drops
    // the `aria-hidden` it is handed would put a nameless graphic inside the
    // item's name. The slot is ours, so the guarantee is ours.
    await open();
    const item = screen.getByRole("menuitem", { name: "Close all tabs" });
    expect(within(item).getByTestId("trash").closest("[aria-hidden='true']")).not.toBeNull();
  });

  it("holds the icon column open for items without one", async () => {
    // The design lines the labels up in a column. An icon that appears only on
    // some items shoves the rest sideways, so the space is always taken.
    await open();
    const withIcon = screen.getByRole("menuitem", { name: "Close all tabs" });
    const without = screen.getByRole("menuitem", { name: "Duplicate tab" });
    expect(within(withIcon).getByTestId("trash")).toBeDefined();
    expect(without.querySelector("[data-icon-slot]")).not.toBeNull();
  });

  it("marks a destructive item for the stylesheet without leaning on the tint", async () => {
    // `.ctx-item[data-danger]` turns the row red. The label is what says it is
    // destructive; the colour is a second channel over words that already
    // carry the meaning.
    await open();
    const item = screen.getByRole("menuitem", { name: "Close all tabs" });
    expect(item.getAttribute("data-danger")).toBe("true");
    expect(screen.getByRole("menuitem", { name: "Duplicate tab" }).getAttribute("data-danger")).toBeNull();
  });

  it("holds no control that could submit a surrounding form", async () => {
    // The mock's items were bare `<button>`s with no type, so a menu opened
    // over a row inside a form submitted it on the first pick. Radix's items
    // are not buttons at all; this pins that, and catches a future item that
    // reaches for one.
    await open();
    const menu = screen.getByRole("menu");
    expect(menu.querySelectorAll("button:not([type='button'])")).toHaveLength(0);
  });
});

describe("ContextMenu is wired to Radix correctly", () => {
  it("renders in a portal, outside the region it guards", async () => {
    const { container } = await open();
    expect(container.contains(screen.getByRole("menu"))).toBe(false);
  });

  it("moves focus into the menu", async () => {
    await open();
    expect(screen.getByRole("menu").contains(document.activeElement)).toBe(true);
  });

  it("keeps the design's own menu styling", async () => {
    await open();
    const menu = screen.getByRole("menu");
    expect(menu.className).toContain("ctx-menu");
    expect(screen.getByRole("menuitem", { name: "Duplicate tab" }).className).toContain("ctx-item");
  });

  it("overrides the stylesheet's fixed position so the collision box has a size", async () => {
    // `.ctx-menu` is written for a menu that places itself: `position: fixed`.
    // Radix already fixes and translates a wrapper around this content, and a
    // fixed child leaves that wrapper zero-sized — which is the box the
    // collision logic measures, so the menu would flip and shift against
    // nothing. Relative rather than static, so the stylesheet's z-index still
    // applies.
    await open();
    expect(screen.getByRole("menu").style.position).toBe("relative");
  });
});

/** A tab-sized surface that owns the layers opened inside it, as `TabSurface` does. */
function Surface({ children }: { children: ReactNode }) {
  const { ref, scope } = usePortalHost();
  return (
    <div data-testid="surface">
      <PortalScopeProvider scope={scope}>
        <div data-testid="content">{children}</div>
        <div data-testid="host" ref={ref} />
      </PortalScopeProvider>
    </div>
  );
}

/**
 * The node the layer was portalled into.
 *
 * Radix's `Portal` renders a div of its own as a direct child of the container
 * and the popper adds another inside that, so the container is never the
 * content's parent. Matching the outermost portalled element and taking *its*
 * parent names the container exactly, where "somewhere in the document" would
 * also pass for a container that was wrong but still attached.
 */
function mountedIn(node: Element): Element | null {
  return node.closest("body > *, [data-testid='host'] > *")?.parentElement ?? null;
}

async function openInSurface() {
  const view = render(
    <Surface>
      <ContextMenu items={ITEMS} label="Tab actions">
        <button type="button">checkout-api</button>
      </ContextMenu>
    </Surface>,
  );
  fireEvent.contextMenu(screen.getByText("checkout-api"));
  await screen.findByRole("menu");
  return view;
}

/**
 * A menu opened in one tab belongs to that tab.
 *
 * The window is a strip of tabs over one screen each, all of them mounted at
 * once with the inactive ones hidden by the `hidden` attribute — which a portal
 * to `document.body` escapes. So a right-click menu opened in one tab stayed on
 * screen over whatever tab the reader moved to. (#357)
 *
 * A menu is not a dialog and does not get a dialog's treatment: it already
 * dismisses on an outside interaction, which is right for it, and it holds
 * nothing the reader typed that leaving the tab would lose. What it does share
 * with the dialog is Radix's modality — `ContextMenu.Root` defaults to `modal`,
 * which takes the whole document out of the accessibility tree and switches the
 * document's pointer events off, so the tab strip and the cluster rail were
 * unreachable behind a right-click menu for exactly the reason they were
 * unreachable behind a dialog. Worse once the menu mounts inside the tab: a
 * menu left open on a tab the reader has switched away from is invisible and
 * still holding the window hostage.
 */
describe("ContextMenu inside a surface", () => {
  it("mounts into the surface's own node, so hiding the tab hides it too", async () => {
    await openInSurface();
    expect(mountedIn(screen.getByRole("menu"))).toBe(screen.getByTestId("host"));
  });

  it("mounts into the document body when there is no surface", async () => {
    // The fallback the gallery, the frozen classic app and most of this kit's
    // own tests rely on, and it must stay exactly as it was.
    await open();
    expect(mountedIn(screen.getByRole("menu"))).toBe(document.body);
  });

  it("leaves the rest of the window in the accessibility tree", async () => {
    const chrome = document.createElement("div");
    chrome.innerHTML = "<button>the tab strip</button>";
    document.body.appendChild(chrome);
    try {
      await openInSurface();
      expect(chrome.getAttribute("aria-hidden")).toBeNull();
    } finally {
      chrome.remove();
    }
  });

  it("still takes the whole document out of it when there is no surface", async () => {
    const chrome = document.createElement("div");
    chrome.innerHTML = "<button>the window chrome</button>";
    document.body.appendChild(chrome);
    try {
      await open();
      expect(chrome.getAttribute("aria-hidden")).toBe("true");
    } finally {
      chrome.remove();
    }
  });

  it("leaves the window outside the tab clickable", async () => {
    // Radix's modal menu switches the document's own pointer events off, so the
    // first click anywhere outside the menu does nothing but dismiss it. Inside
    // a tab that costs the reader a click to reach a strip they can see.
    await openInSurface();
    expect(document.body.style.pointerEvents).toBe("");
  });

  it("still blocks the window outside it when there is no surface", async () => {
    await open();
    expect(document.body.style.pointerEvents).toBe("none");
  });

  it("still closes on Escape", async () => {
    await openInSurface();
    fireEvent.keyDown(document.activeElement ?? document.body, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("menu")).toBeNull());
  });

  it("still closes on an interaction outside it", async () => {
    const elsewhere = document.createElement("button");
    elsewhere.textContent = "the tab strip";
    document.body.appendChild(elsewhere);
    try {
      await openInSurface();
      await userEvent.click(elsewhere);
      await waitFor(() => expect(screen.queryByRole("menu")).toBeNull());
    } finally {
      elsewhere.remove();
    }
  });
});

/**
 * A surface whose node has not arrived yet.
 *
 * Real for exactly one render: `usePortalHost` holds the node in state, filled
 * by a ref callback that fires after the render that declared it, so a layer
 * mounting in the same commit as its surface sees a scope with no container.
 * Standing it up by hand is the only way to hold that render still. (#357
 * review)
 */
const arriving = { container: undefined, visible: true, hold: () => () => {} };

async function openInArrivingSurface() {
  const view = render(
    <PortalScopeProvider scope={arriving}>
      <ContextMenu items={ITEMS} label="Tab actions">
        <button type="button">checkout-api</button>
      </ContextMenu>
    </PortalScopeProvider>,
  );
  fireEvent.contextMenu(screen.getByText("checkout-api"));
  await screen.findByRole("menu");
  return view;
}

describe("ContextMenu before its surface's node arrives", () => {
  it("is still the tab's menu, not the window's", async () => {
    // Where to mount and whether there is a surface to be modal within are two
    // questions, and the container answers only the first: `undefined` means
    // "document.body" outside a tab and "not yet" inside one. Read as the
    // second, this render is a window-wide modal — the tab strip and the
    // cluster rail out of the accessibility tree and the document's pointer
    // events off — inside a tab that has one.
    const chrome = document.createElement("div");
    chrome.innerHTML = "<button>the tab strip</button>";
    document.body.appendChild(chrome);
    try {
      await openInArrivingSurface();
      expect(chrome.getAttribute("aria-hidden")).toBeNull();
      expect(document.body.style.pointerEvents).toBe("");
    } finally {
      chrome.remove();
    }
  });
});
