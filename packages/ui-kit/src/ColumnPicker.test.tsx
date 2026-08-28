import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { ColumnPicker, type ColumnOption } from "./ColumnPicker";
import { PortalScopeProvider, usePortalHost } from "./portal";

// jsdom has no ResizeObserver, and Radix's popper watches the trigger and the
// content with one. The kit's shared setup does not stub it — nothing in the
// kit was positioned against an anchor before this — and that setup is not this
// file's to edit, so the stub lives here. Inert: jsdom does no layout, so there
// is never a resize to report. (apps/desktop's setup carries the same stub.)
if (!("ResizeObserver" in globalThis)) {
  (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}

const COLUMNS: ColumnOption[] = [
  { key: "name", label: "Name" },
  { key: "namespace", label: "Namespace" },
  { key: "status", label: "Status" },
  { key: "age", label: "Age" },
];

function setup(props: Partial<Parameters<typeof ColumnPicker>[0]> = {}) {
  const onToggle = vi.fn();
  const view = render(
    <ColumnPicker
      columns={COLUMNS}
      hidden={new Set<string>()}
      onToggle={onToggle}
      pinnedKey="name"
      {...props}
    />,
  );
  return { onToggle, ...view };
}

const trigger = () => screen.getByRole("button", { name: /Columns/ });

async function open(props: Partial<Parameters<typeof ColumnPicker>[0]> = {}) {
  const view = setup(props);
  await userEvent.click(trigger());
  await screen.findByRole("group");
  return view;
}

const box = (label: string) => screen.getByRole("checkbox", { name: label }) as HTMLInputElement;

/**
 * What this component owns: the visibility contract it presents to a table —
 * every column offered, the pinned one held on, a key handed back per toggle —
 * and its wiring to Radix's Popover.
 *
 * Deliberately absent: focus trapping inside the panel, outside-click
 * dismissal, collision flipping. Those are the library's, and asserting a
 * dependency's internals through our component only pins the version we happen
 * to have. Escape is here because it is the one dismissal path a keyboard user
 * has no alternative to, so losing it would be a real regression rather than a
 * library upgrade. (#318)
 */
describe("ColumnPicker", () => {
  it("keeps the panel shut until the trigger is used", () => {
    setup();
    expect(screen.queryByRole("group")).toBeNull();
    expect(screen.queryByRole("checkbox")).toBeNull();
  });

  it("opens from the trigger and lists every column, in order", async () => {
    await open();
    expect(screen.getAllByRole("checkbox").map((b) => b.parentElement?.textContent)).toEqual([
      "Name",
      "Namespace",
      "Status",
      "Age",
    ]);
  });

  it("reports its open state on the trigger", async () => {
    setup();
    expect(trigger().getAttribute("aria-expanded")).toBe("false");
    await userEvent.click(trigger());
    await waitFor(() => expect(trigger().getAttribute("aria-expanded")).toBe("true"));
  });

  it("names the group of options", async () => {
    await open();
    // Without a name the panel is a bare pile of checkboxes: a screen-reader
    // user arrives at "Name, checkbox" with nothing saying what turning it off
    // would do.
    expect(screen.getByRole("group", { name: "Toggle columns" })).toBeDefined();
  });

  it("renders the panel in a portal, out of the toolbar it was declared in", async () => {
    const { container } = await open();
    expect(container.contains(screen.getByRole("group"))).toBe(false);
  });
});

describe("ColumnPicker toggling", () => {
  it("hands back the key of the column that was clicked", async () => {
    const { onToggle } = await open();
    await userEvent.click(box("Status"));
    expect(onToggle).toHaveBeenCalledTimes(1);
    expect(onToggle).toHaveBeenCalledWith("status");
  });

  it("shows hidden columns unchecked and visible ones checked", async () => {
    await open({ hidden: new Set(["status", "age"]) });
    expect(box("Namespace").checked).toBe(true);
    expect(box("Status").checked).toBe(false);
    expect(box("Age").checked).toBe(false);
  });

  it("stays open across a toggle, so several can be changed in one visit", async () => {
    const { onToggle } = await open();
    await userEvent.click(box("Status"));
    await userEvent.click(box("Age"));
    expect(onToggle.mock.calls).toEqual([["status"], ["age"]]);
  });
});

/**
 * The pinned column is the row identifier. A table whose rows lost their name
 * is not a table any more, so this is a hard invariant, not a default.
 */
describe("ColumnPicker and the pinned column", () => {
  it("offers it checked and disabled", async () => {
    await open();
    expect(box("Name").checked).toBe(true);
    expect(box("Name").disabled).toBe(true);
  });

  it("refuses to toggle it", async () => {
    // Fired directly rather than through userEvent, which would decline to
    // click a disabled control and so assert nothing about this component. The
    // pin is ours to keep, not the attribute's: this is the change event
    // arriving anyway, and the answer still has to be no.
    const { onToggle } = await open();
    fireEvent.click(box("Name"));
    expect(onToggle).not.toHaveBeenCalled();
  });

  it("holds it on even when the caller's hidden set names it", async () => {
    // The set is the caller's state and may be stale — a persisted layout from
    // before this column was pinned, say. The pin wins.
    await open({ hidden: new Set(["name"]) });
    expect(box("Name").checked).toBe(true);
  });

  it("marks the row disabled to assistive technology, not only the input", async () => {
    await open();
    expect(box("Name").closest("label")?.getAttribute("aria-disabled")).toBe("true");
    expect(box("Status").closest("label")?.hasAttribute("aria-disabled")).toBe(false);
  });
});

describe("ColumnPicker's count", () => {
  it("says how many columns are showing once any are hidden", async () => {
    setup({ hidden: new Set(["status"]) });
    expect(trigger().textContent).toContain("(3)");
  });

  it("does not count the pinned column as hidden", async () => {
    // It is never off, so a stale set naming it must not make the trigger
    // claim a column is missing.
    setup({ hidden: new Set(["name"]) });
    expect(trigger().textContent).not.toContain("(");
  });

  it("stays quiet while everything is visible", () => {
    setup();
    expect(trigger().textContent).not.toContain("(");
  });

  it("announces the count, rather than showing it to sighted users only", () => {
    setup({ hidden: new Set(["status", "age"]) });
    expect(screen.getByRole("button", { name: /Columns\s*\(2\)/ })).toBeDefined();
  });
});

describe("ColumnPicker's trigger", () => {
  it("takes its accessible name from its visible label", () => {
    // Matching what is on screen is what lets speech-input users say it, so
    // the label is the name rather than a separate aria-label saying something
    // else. (#318 review)
    setup({ label: "Fields" });
    expect(screen.getByRole("button", { name: "Fields" })).toBeDefined();
  });

  it("still has a name when the label is empty", () => {
    setup({ label: "" });
    expect(screen.getByRole("button", { name: "Choose columns" }).textContent).toBe("");
  });

  it("does not submit the form it is standing in", async () => {
    // This control lives in toolbars, and a toolbar sits inside a form often
    // enough that a bare <button> submitting on open is a live bug. The kit's
    // Button does not default the type, so this component sets it.
    const onSubmit = vi.fn((e: React.FormEvent) => e.preventDefault());
    render(
      <form onSubmit={onSubmit}>
        <ColumnPicker columns={COLUMNS} hidden={new Set()} onToggle={() => {}} pinnedKey="name" />
      </form>,
    );
    expect(trigger().getAttribute("type")).toBe("button");
    await userEvent.click(trigger());
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("carries a decorative glyph that adds nothing to the name", () => {
    // Inlined rather than imported from lucide: the kit takes no dependency on
    // an icon set.
    setup();
    const glyph = trigger().querySelector("svg");
    expect(glyph).not.toBeNull();
    expect(glyph?.getAttribute("aria-hidden")).toBe("true");
  });
});

describe("ColumnPicker dismissal", () => {
  it("closes on Escape", async () => {
    await open();
    fireEvent.keyDown(document, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("group")).toBeNull());
  });
});

describe("ColumnPicker's appearance", () => {
  it("dresses the panel in the design's own popover surface", async () => {
    await open();
    const panel = screen.getByRole("group").parentElement;
    expect(panel?.className).toContain("popover");
  });

  it("leaves the panel in flow so the popper can measure it", async () => {
    // `.popover` is written for a panel that positions itself: `position:
    // fixed`. Radix already fixes and translates a wrapper around the content,
    // and a fixed child leaves that wrapper zero-sized — which is what the
    // collision logic measures, so the panel would flip and shift against a
    // box of nothing. Structural, because jsdom does no layout. (#318 review)
    await open();
    const panel = screen.getByRole("group").parentElement as HTMLElement;
    expect(panel.style.position).toBe("relative");
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
  const onToggle = vi.fn();
  const view = render(
    <Surface>
      <ColumnPicker columns={COLUMNS} hidden={new Set<string>()} onToggle={onToggle} pinnedKey="name" />
    </Surface>,
  );
  await userEvent.click(trigger());
  await screen.findByRole("group");
  return { onToggle, ...view };
}

const columnPanel = () => screen.getByRole("dialog");

/**
 * A panel anchored to a toolbar button belongs to the tab that toolbar is in.
 *
 * The window is a strip of tabs over one screen each, all of them mounted at
 * once with the inactive ones hidden by the `hidden` attribute — which a portal
 * to `document.body` escapes. So the column list opened over one table stayed
 * on screen over whatever tab the reader moved to, with its trigger and its
 * table already gone. (#357)
 *
 * The container is the whole change. Radix's Popover is already non-modal and
 * already dismisses on an outside interaction, which is the right answer for a
 * panel and is why none of the dialog's other treatment applies here.
 */
describe("ColumnPicker inside a surface", () => {
  it("mounts into the surface's own node, so hiding the tab hides it too", async () => {
    await openInSurface();
    expect(mountedIn(columnPanel())).toBe(screen.getByTestId("host"));
  });

  it("mounts into the document body when there is no surface", async () => {
    // The fallback the gallery, the frozen classic app and most of this kit's
    // own tests rely on, and it must stay exactly as it was.
    await open();
    expect(mountedIn(columnPanel())).toBe(document.body);
  });

  it("still toggles a column from inside the surface", async () => {
    const { onToggle } = await openInSurface();
    await userEvent.click(screen.getByRole("checkbox", { name: "Status" }));
    expect(onToggle).toHaveBeenCalledWith("status");
  });

  it("still closes on Escape", async () => {
    await openInSurface();
    fireEvent.keyDown(document.activeElement ?? document.body, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("group")).toBeNull());
  });
});
