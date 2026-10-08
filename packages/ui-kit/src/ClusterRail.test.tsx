import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent, createEvent, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ClusterRail, type ClusterRailItem } from "./ClusterRail";

// jsdom has no ResizeObserver, and Radix's popper — which Tooltip sits on —
// watches the trigger and the content with one. The kit's shared setup does not
// stub it and is not this file's to edit, so the stub lives here, as it does in
// ColumnPicker.test.tsx. Inert: jsdom does no layout, so there is never a
// resize to report.
if (!("ResizeObserver" in globalThis)) {
  (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}

const ITEMS: ClusterRailItem[] = [
  { id: "prod-eu", name: "prod-eu", mark: <span>PE</span>, group: "team" },
  { id: "prod-us", name: "prod-us", mark: <span>PU</span>, group: "team" },
  { id: "staging", name: "staging", mark: <span>ST</span>, group: "file" },
];

function setup(props: Partial<Parameters<typeof ClusterRail>[0]> = {}) {
  const onSelect = vi.fn();
  const view = render(<ClusterRail items={ITEMS} activeId="prod-eu" onSelect={onSelect} {...props} />);
  return { onSelect, ...view };
}

const chip = (name: string | RegExp) => screen.getByRole("button", { name });

describe("ClusterRail", () => {
  it("renders one button per cluster, named by the cluster", () => {
    setup();
    expect(chip("prod-eu")).toBeDefined();
    expect(chip("prod-us")).toBeDefined();
    expect(chip("staging")).toBeDefined();
  });

  it("renders each mark the caller gave it", () => {
    setup();
    expect(screen.getByText("PE")).toBeDefined();
    expect(screen.getByText("ST")).toBeDefined();
  });

  it("emits the id on click", async () => {
    const { onSelect } = setup();
    await userEvent.click(chip("prod-us"));
    expect(onSelect).toHaveBeenCalledWith("prod-us");
  });

  it("marks the active cluster as current", () => {
    setup();
    expect(chip("prod-eu").getAttribute("aria-current")).toBe("true");
    expect(chip("prod-us").getAttribute("aria-current")).toBeNull();
  });

  it("names the landmark", () => {
    setup();
    expect(screen.getByRole("navigation", { name: "Clusters" })).toBeDefined();
  });

  it("takes a different name for the landmark", () => {
    setup({ label: "Connected clusters" });
    expect(screen.getByRole("navigation", { name: "Connected clusters" })).toBeDefined();
  });

  it("puts the clusters in a list", () => {
    // A rail of a dozen marks is a list, and a screen reader that says so tells
    // the reader how many there are before they start arrowing through them.
    setup();
    expect(screen.getAllByRole("listitem")).toHaveLength(3);
  });

  it("only prints the names under the marks when asked", () => {
    // The name is always the accessible name; `showNames` is about the caption.
    const { container, rerender } = setup();
    expect(container.querySelectorAll("[data-slot='caption']")).toHaveLength(0);
    rerender(<ClusterRail items={ITEMS} activeId="prod-eu" onSelect={() => {}} showNames />);
    expect(container.querySelectorAll("[data-slot='caption']")).toHaveLength(3);
  });

  it("gives every button it owns an explicit type", () => {
    // A bare <button> inside a form submits it.
    setup({ onAdd: () => {} });
    for (const button of screen.getAllByRole("button")) {
      expect(button.getAttribute("type")).toBe("button");
    }
  });
});

/**
 * State that is drawn in colour has to be said in words as well, or it reaches
 * neither a colour-blind reader nor a screen reader.
 */
describe("ClusterRail state", () => {
  it("says what each marker dot means, in the button's name", () => {
    setup({
      items: [
        {
          id: "prod-eu",
          name: "prod-eu",
          mark: <span>PE</span>,
          markers: [
            { label: "Team connection", tone: "accent" },
            { label: "Degraded", tone: "sev" },
          ],
        },
      ],
    });
    const button = chip(/prod-eu/);
    expect(button.getAttribute("aria-label")).toContain("Team connection");
    expect(button.getAttribute("aria-label")).toContain("Degraded");
  });

  it("draws a dot per marker", () => {
    const { container } = setup({
      items: [
        {
          id: "prod-eu",
          name: "prod-eu",
          mark: <span>PE</span>,
          markers: [{ label: "Team connection" }, { label: "Degraded", tone: "sev" }],
        },
      ],
    });
    expect(container.querySelectorAll("[data-slot='marker']")).toHaveLength(2);
  });

  it("names the reason a cluster is out of reach rather than only dimming it", () => {
    setup({
      items: [{ id: "prod-eu", name: "prod-eu", mark: <span>PE</span>, unavailable: "Disconnected" }],
    });
    expect(chip(/prod-eu/).getAttribute("aria-label")).toContain("Disconnected");
  });

  it("dims the cluster that is out of reach", () => {
    const { container } = setup({
      items: [{ id: "prod-eu", name: "prod-eu", mark: <span>PE</span>, unavailable: "Disconnected" }],
    });
    expect(container.querySelector("[data-unavailable='true']")).not.toBeNull();
  });

  it("rules off between groups, but not before the first", () => {
    const { container } = setup();
    // prod-eu/prod-us share a group; staging starts a new one.
    expect(container.querySelectorAll("[data-slot='group-rule']")).toHaveLength(1);
  });
});

describe("ClusterRail gestures", () => {
  it("emits the id on double click", async () => {
    const onOpen = vi.fn();
    setup({ onOpen });
    await userEvent.dblClick(chip("prod-us"));
    expect(onOpen).toHaveBeenCalledWith("prod-us");
  });

  it("opens the caller's menu on the context-menu gesture, and takes over the browser's", async () => {
    // Shift+F10 and the Menu key both raise `contextmenu`, so a keyboard user
    // reaches the same menu without a pointer.
    const onPick = vi.fn();
    const menuFor = vi.fn((item: ClusterRailItem) => [{ label: `Customise ${item.name}`, onPick }]);
    setup({ menuFor });
    const event = createEvent.contextMenu(chip("prod-us"));
    fireEvent(chip("prod-us"), event);

    // Named after the cluster the gesture landed on, so a reader arriving in
    // the menu is told which of a dozen marks it is about.
    const menu = await screen.findByRole("menu", { name: "prod-us actions" });
    fireEvent.click(within(menu).getByRole("menuitem", { name: "Customise prod-us" }));
    expect(onPick).toHaveBeenCalledTimes(1);
    expect(event.defaultPrevented).toBe(true);
  });

  it("leaves the browser's own menu alone when no menu was given", () => {
    setup();
    const event = createEvent.contextMenu(chip("prod-us"));
    fireEvent(chip("prod-us"), event);
    expect(event.defaultPrevented).toBe(false);
  });

  it("leaves it alone for a cluster the caller offers nothing for", () => {
    // An empty list is "no menu here", not "a menu with nothing in it": the
    // second takes the browser's own menu away and gives back an empty box.
    setup({ menuFor: (item) => (item.id === "prod-eu" ? [{ label: "Customise", onPick: () => {} }] : []) });
    const event = createEvent.contextMenu(chip("prod-us"));
    fireEvent(chip("prod-us"), event);
    expect(event.defaultPrevented).toBe(false);
    expect(screen.queryByRole("menu")).toBeNull();
  });
});

/** A vertical rail of a dozen marks is arrowed through, not tabbed through. */
describe("ClusterRail keyboard behaviour", () => {
  it("moves down and up the rail with the arrow keys", async () => {
    setup();
    chip("prod-eu").focus();
    await userEvent.keyboard("{ArrowDown}");
    expect(document.activeElement).toBe(chip("prod-us"));
    await userEvent.keyboard("{ArrowUp}");
    expect(document.activeElement).toBe(chip("prod-eu"));
  });

  it("wraps at both ends", async () => {
    setup();
    chip("staging").focus();
    await userEvent.keyboard("{ArrowDown}");
    expect(document.activeElement).toBe(chip("prod-eu"));
    await userEvent.keyboard("{ArrowUp}");
    expect(document.activeElement).toBe(chip("staging"));
  });

  it("jumps to the first and last with Home and End", async () => {
    setup();
    chip("prod-us").focus();
    await userEvent.keyboard("{End}");
    expect(document.activeElement).toBe(chip("staging"));
    await userEvent.keyboard("{Home}");
    expect(document.activeElement).toBe(chip("prod-eu"));
  });

  it("moves focus without selecting", async () => {
    // Selecting a cluster switches the whole workspace; arrowing past one must
    // not do that on the way.
    const { onSelect } = setup();
    chip("prod-eu").focus();
    await userEvent.keyboard("{ArrowDown}");
    expect(onSelect).not.toHaveBeenCalled();
  });

  it("leaves other keys alone", async () => {
    const { onSelect } = setup();
    chip("prod-eu").focus();
    await userEvent.keyboard("{ArrowRight}");
    await userEvent.keyboard("a");
    expect(document.activeElement).toBe(chip("prod-eu"));
    expect(onSelect).not.toHaveBeenCalled();
  });
});

describe("ClusterRail width", () => {
  const widthOf = (container: HTMLElement) =>
    (container.querySelector("nav") as HTMLElement).style.width;

  it("sizes itself to the marks", () => {
    const { container } = setup({ markSize: 30 });
    expect(widthOf(container)).toBe("46px");
  });

  it("makes room for the captions", () => {
    const { container } = setup({ markSize: 30, showNames: true });
    expect(widthOf(container)).toBe("60px");
  });

  it("clamps a mark size that would collapse or swallow the rail", () => {
    // The mock fed this straight into a width. A stored preference that comes
    // back as 0, or as a number someone typed, must not leave the rail invisible
    // or half the window wide.
    expect(widthOf(setup({ markSize: 0 }).container)).toBe("32px");
    expect(widthOf(setup({ markSize: -40 }).container)).toBe("32px");
    expect(widthOf(setup({ markSize: 9999 }).container)).toBe("80px");
  });

  it("falls back to the default for a size that is not a number", () => {
    expect(widthOf(setup({ markSize: Number.NaN }).container)).toBe("46px");
  });
});

describe("ClusterRail with nothing in it", () => {
  it("says so", () => {
    setup({ items: [] });
    expect(screen.getByText("No clusters")).toBeDefined();
  });

  it("takes its own wording", () => {
    setup({ items: [], emptyLabel: "Nothing connected" });
    expect(screen.getByText("Nothing connected")).toBeDefined();
  });

  it("renders no list at all", () => {
    setup({ items: [] });
    expect(screen.queryByRole("list")).toBeNull();
  });

  it("still offers the way out of the emptiness", () => {
    const onAdd = vi.fn();
    setup({ items: [], onAdd });
    expect(chip("Connect a cluster")).toBeDefined();
  });
});

describe("ClusterRail add tile", () => {
  it("is absent unless there is somewhere to add a cluster", () => {
    setup();
    expect(screen.queryByRole("button", { name: "Connect a cluster" })).toBeNull();
  });

  it("is named, and calls back", async () => {
    const onAdd = vi.fn();
    setup({ onAdd });
    await userEvent.click(chip("Connect a cluster"));
    expect(onAdd).toHaveBeenCalled();
  });

  it("takes its own wording", () => {
    setup({ onAdd: () => {}, addLabel: "Add a context" });
    expect(chip("Add a context")).toBeDefined();
  });

  it("takes the app's own glyph", () => {
    const Glyph = () => <svg data-testid="app-glyph" />;
    setup({ onAdd: () => {}, addIcon: Glyph });
    expect(screen.getByTestId("app-glyph")).toBeDefined();
  });
});

describe("ClusterRail when the list could not be loaded", () => {
  it("shows the failure, in words", () => {
    setup({ error: "kubeconfig unreadable" });
    expect(screen.getByRole("img", { name: "kubeconfig unreadable" })).toBeDefined();
  });

  it("keeps the clusters it did have", () => {
    setup({ error: "kubeconfig unreadable" });
    expect(chip("prod-eu")).toBeDefined();
  });

  it("shows nothing when there is nothing wrong", () => {
    setup();
    expect(screen.queryByRole("img")).toBeNull();
  });
});

describe("ClusterRail footer", () => {
  it("renders what the app puts there", () => {
    setup({ footer: <button type="button">Options</button> });
    expect(chip("Options")).toBeDefined();
  });

  it("buys no space for a slot that resolved to nothing", () => {
    // `footer={showOptions && <Options/>}` is how a caller makes it conditional.
    const { container } = setup({ footer: false });
    expect(container.querySelector("[data-slot='footer']")).toBeNull();
  });
});

/**
 * Reordering (#829). The rail knows a mark can be moved; the caller owns the
 * order, so every case here is about what `onMove` is asked for and what the
 * rail shows while it is being asked.
 */
describe("reordering", () => {
  const dataTransfer = () => ({ effectAllowed: "", dropEffect: "", setData: vi.fn() });

  /** jsdom has no DragEvent, so the pointer's position is put on by hand. */
  function fireAt(type: "dragOver" | "drop", node: Element, clientY: number) {
    const event = createEvent[type](node);
    Object.defineProperty(event, "clientY", { value: clientY });
    Object.defineProperty(event, "dataTransfer", { value: dataTransfer() });
    fireEvent(node, event);
  }

  /**
   * jsdom lays nothing out, and the rail reads the pointer against every
   * mark's box: stack the three, 30px tall with the rail's 6px gap. So
   * prod-eu is 0–30, prod-us 36–66, staging 72–102.
   */
  const TOPS: Record<string, number> = { "prod-eu": 0, "prod-us": 36, staging: 72 };
  function layOut() {
    for (const [name, top] of Object.entries(TOPS)) {
      chip(name).getBoundingClientRect = () =>
        ({ top, bottom: top + 30, height: 30, left: 0, right: 30, width: 30, x: 0, y: top, toJSON: () => ({}) }) as DOMRect;
    }
  }

  /** Start dragging `source` and hold it at height `y` over `node`; drop there unless told not to. */
  function dragTo(source: string, node: Element, y: number, drop = true) {
    layOut();
    fireEvent.pointerDown(chip(source));
    fireEvent.dragStart(chip(source), { dataTransfer: dataTransfer() });
    fireAt("dragOver", node, y);
    if (drop) fireAt("drop", node, y);
  }

  function dragOnto(source: string, target: string, half: "upper" | "lower", drop = true) {
    dragTo(source, chip(target), TOPS[target] + (half === "upper" ? 5 : 25), drop);
  }

  it("is not draggable, and refuses a drag, unless the caller can reorder", () => {
    setup();
    for (const name of ["prod-eu", "prod-us", "staging"]) expect(chip(name).getAttribute("draggable")).toBe("false");
    // A drag that starts anyway (an image inside the mark can be dragged by
    // the browser) is cancelled rather than left to carry the mark off.
    expect(fireEvent.dragStart(chip("prod-us"), { dataTransfer: dataTransfer() })).toBe(false);
    expect(chip("prod-us").getAttribute("data-dragging")).toBeNull();
  });

  it("marks every mark draggable when it can", () => {
    setup({ onMove: vi.fn() });
    for (const name of ["prod-eu", "prod-us", "staging"]) expect(chip(name).getAttribute("draggable")).toBe("true");
  });

  it("asks for the index the item ends up at when dropped on a mark's upper half", () => {
    const onMove = vi.fn();
    setup({ onMove });
    // staging (2) onto the upper half of prod-eu (0): before it, so index 0.
    dragOnto("staging", "prod-eu", "upper");
    expect(onMove).toHaveBeenCalledTimes(1);
    expect(onMove).toHaveBeenCalledWith("staging", 0);
  });

  it("counts the index with the item already taken out when it moves down", () => {
    const onMove = vi.fn();
    setup({ onMove });
    // prod-eu (0) onto the lower half of prod-us (1): after it. Insertion point
    // 2, less the slot prod-eu itself leaves, is index 1.
    dragOnto("prod-eu", "prod-us", "lower");
    expect(onMove).toHaveBeenCalledWith("prod-eu", 1);
  });

  it("moves to the end when dropped on the last mark's lower half", () => {
    const onMove = vi.fn();
    setup({ onMove });
    dragOnto("prod-eu", "staging", "lower");
    expect(onMove).toHaveBeenCalledWith("prod-eu", 2);
  });

  it("asks for nothing when the drop would leave the item where it is", () => {
    const onMove = vi.fn();
    setup({ onMove });
    // Onto its own lower half, and onto the upper half of the mark below it.
    dragOnto("prod-us", "prod-us", "lower");
    dragOnto("prod-us", "staging", "upper");
    expect(onMove).not.toHaveBeenCalled();
  });

  describe("over the gap between two marks (PR #838 review)", () => {
    // The gaps are inside the drop area and are not marks. Read off the
    // event's target, a release over one fell back to wherever the pointer
    // had last been over a mark, or to the end.
    it("drops between the two marks either side of the gap, wherever the pointer came from", () => {
      const onMove = vi.fn();
      const { container } = setup({ onMove });
      const list = container.querySelector("ul")!;
      // Straight into the gap between prod-eu (0–30) and prod-us (36–66),
      // never having been over a mark: staging lands between them.
      dragTo("staging", list, 33);
      expect(onMove).toHaveBeenCalledWith("staging", 1);
    });

    it("does not carry over the place of the last mark the pointer crossed", () => {
      const onMove = vi.fn();
      const { container } = setup({ onMove });
      const list = container.querySelector("ul")!;
      layOut();
      fireEvent.pointerDown(chip("prod-eu"));
      fireEvent.dragStart(chip("prod-eu"), { dataTransfer: dataTransfer() });
      // Over the upper half of prod-us first (before it: no move for prod-eu)...
      fireAt("dragOver", chip("prod-us"), 40);
      // ...then down into the gap below prod-us, and released there.
      fireAt("dragOver", list, 69);
      fireAt("drop", list, 69);
      expect(onMove).toHaveBeenCalledWith("prod-eu", 1);
    });

    it("drops at the end below the last mark, and at the start above the first", () => {
      const onMove = vi.fn();
      const { container } = setup({ onMove });
      const list = container.querySelector("ul")!;
      dragTo("prod-eu", list, 140);
      expect(onMove).toHaveBeenLastCalledWith("prod-eu", 2);
      dragTo("staging", list, -4);
      expect(onMove).toHaveBeenLastCalledWith("staging", 0);
    });

    it("draws the rule for the gap the pointer is in", () => {
      const { container } = setup({ onMove: vi.fn() });
      dragTo("staging", container.querySelector("ul")!, 33, false);
      expect(Array.from(container.querySelectorAll("li")).map((li) => li.getAttribute("data-drop"))).toEqual([
        null,
        "before",
        null,
      ]);
    });
  });

  describe("moves offered to the caller's menu (PR #838 review)", () => {
    function movesFor(onMove?: (id: string, to: number) => void) {
      const seen = new Map<string, { up: boolean; down: boolean; moves: { moveUp?: () => void; moveDown?: () => void } }>();
      const view = render(
        <ClusterRail
          items={ITEMS}
          activeId="prod-eu"
          onSelect={vi.fn()}
          onMove={onMove}
          menuFor={(item, moves) => {
            seen.set(item.id, { up: !!moves.moveUp, down: !!moves.moveDown, moves });
            return [{ label: "Open", onPick: () => {} }];
          }}
        />,
      );
      return { seen, view };
    }

    it("offers each direction only where the mark has somewhere to go", () => {
      const { seen } = movesFor(vi.fn());
      expect(seen.get("prod-eu")).toMatchObject({ up: false, down: true });
      expect(seen.get("prod-us")).toMatchObject({ up: true, down: true });
      expect(seen.get("staging")).toMatchObject({ up: true, down: false });
    });

    it("offers neither on a rail that cannot be reordered", () => {
      const { seen } = movesFor(undefined);
      for (const id of ["prod-eu", "prod-us", "staging"]) expect(seen.get(id)).toMatchObject({ up: false, down: false });
    });

    it("makes the move through the rail, so it is announced like any other", () => {
      const onMove = vi.fn();
      const { seen, view } = movesFor(onMove);

      seen.get("staging")!.moves.moveUp!();
      expect(onMove).toHaveBeenCalledWith("staging", 1);

      view.rerender(
        <ClusterRail items={[ITEMS[0], ITEMS[2], ITEMS[1]]} activeId="prod-eu" onSelect={vi.fn()} onMove={onMove} menuFor={() => []} />,
      );
      expect(screen.getByRole("status").textContent).toBe("staging moved to position 2 of 3");
    });
  });

  it("draws a rule where the item would land, and none where dropping changes nothing", () => {
    const { container } = setup({ onMove: vi.fn() });
    const rule = () => Array.from(container.querySelectorAll("li")).map((li) => li.getAttribute("data-drop"));

    dragOnto("staging", "prod-eu", "upper", false);
    expect(rule()).toEqual(["before", null, null]);

    dragOnto("prod-eu", "staging", "lower", false);
    expect(rule()).toEqual([null, null, "after"]);

    // Either side of the mark being dragged: no rule at all.
    dragOnto("prod-us", "prod-us", "upper", false);
    expect(rule()).toEqual([null, null, null]);
    dragOnto("prod-us", "staging", "upper", false);
    expect(rule()).toEqual([null, null, null]);
  });

  it("dims the mark being dragged, and clears everything when the drag ends", () => {
    const { container } = setup({ onMove: vi.fn() });
    dragOnto("staging", "prod-eu", "upper", false);
    expect(chip("staging").getAttribute("data-dragging")).toBe("true");

    fireEvent.dragEnd(chip("staging"), { dataTransfer: dataTransfer() });
    expect(chip("staging").getAttribute("data-dragging")).toBeNull();
    expect(container.querySelector("[data-drop]")).toBeNull();
  });

  it("does not select the cluster a drag started on, and selects again on the next plain click", () => {
    const { onSelect } = setup({ onMove: vi.fn() });
    dragOnto("staging", "prod-eu", "upper");
    // The click a host fires at the end of a drag is a pointer's: detail 1.
    fireEvent.click(chip("staging"), { detail: 1 });
    expect(onSelect).not.toHaveBeenCalled();

    fireEvent.pointerDown(chip("staging"));
    fireEvent.click(chip("staging"), { detail: 1 });
    expect(onSelect).toHaveBeenCalledWith("staging");
  });

  it("still selects from the keyboard after a drag, with no pointer press in between (PR #838 review)", async () => {
    // Enter and Space click a button without a pointerdown, which is the only
    // thing that clears the drag's flag: after one drag every mark was dead
    // to the keyboard until the pointer was used again.
    const { onSelect } = setup({ onMove: vi.fn() });
    dragOnto("staging", "prod-eu", "upper");
    fireEvent.dragEnd(chip("staging"), { dataTransfer: dataTransfer() });

    chip("prod-us").focus();
    await userEvent.keyboard("{Enter}");
    expect(onSelect).toHaveBeenLastCalledWith("prod-us");

    chip("staging").focus();
    await userEvent.keyboard(" ");
    expect(onSelect).toHaveBeenLastCalledWith("staging");
    expect(onSelect).toHaveBeenCalledTimes(2);
  });

  it("moves the focused mark one place with Ctrl/Cmd+Shift+Arrow, and does not wrap", () => {
    const onMove = vi.fn();
    setup({ onMove });
    chip("prod-us").focus();
    fireEvent.keyDown(chip("prod-us"), { key: "ArrowUp", ctrlKey: true, shiftKey: true });
    expect(onMove).toHaveBeenLastCalledWith("prod-us", 0);
    fireEvent.keyDown(chip("prod-us"), { key: "ArrowDown", metaKey: true, shiftKey: true });
    expect(onMove).toHaveBeenLastCalledWith("prod-us", 2);

    onMove.mockClear();
    chip("prod-eu").focus();
    fireEvent.keyDown(chip("prod-eu"), { key: "ArrowUp", ctrlKey: true, shiftKey: true });
    chip("staging").focus();
    fireEvent.keyDown(chip("staging"), { key: "ArrowDown", ctrlKey: true, shiftKey: true });
    expect(onMove).not.toHaveBeenCalled();
  });

  it("leaves the plain arrows moving focus, not marks", () => {
    const onMove = vi.fn();
    setup({ onMove });
    chip("prod-eu").focus();
    fireEvent.keyDown(chip("prod-eu"), { key: "ArrowDown" });
    expect(document.activeElement).toBe(chip("prod-us"));
    expect(onMove).not.toHaveBeenCalled();
  });

  it("says where the mark went, and keeps focus on it, once the caller's new order arrives", () => {
    const onMove = vi.fn();
    const view = setup({ onMove });
    expect(screen.getByRole("status").textContent).toBe("");
    chip("staging").focus();
    fireEvent.keyDown(chip("staging"), { key: "ArrowUp", ctrlKey: true, shiftKey: true });
    // Nothing yet: the order is the caller's, and it has not changed.
    expect(screen.getByRole("status").textContent).toBe("");

    const reordered = [ITEMS[0], ITEMS[2], ITEMS[1]];
    view.rerender(<ClusterRail items={reordered} activeId="prod-eu" onSelect={vi.fn()} onMove={onMove} />);
    expect(screen.getByRole("status").textContent).toBe("staging moved to position 2 of 3");
    expect(document.activeElement).toBe(chip("staging"));
  });

  it("announces nothing when the caller declines the move", () => {
    const onMove = vi.fn();
    const view = setup({ onMove });
    chip("staging").focus();
    fireEvent.keyDown(chip("staging"), { key: "ArrowUp", ctrlKey: true, shiftKey: true });
    // Same order back, as a new array.
    view.rerender(<ClusterRail items={[...ITEMS]} activeId="prod-eu" onSelect={vi.fn()} onMove={onMove} />);
    expect(screen.getByRole("status").textContent).toBe("");
  });
});
