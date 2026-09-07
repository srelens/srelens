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
