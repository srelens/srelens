import { describe, it, expect } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { Tooltip } from "./Tooltip";
import { PortalScopeProvider, usePortalHost } from "./portal";

// jsdom has no ResizeObserver, and Radix's popper watches the trigger and the
// content with one. The kit's shared setup does not stub it, and that setup is
// not this file's to edit, so the stub lives here. Inert: jsdom does no layout,
// so there is never a resize to report. (ColumnPicker.test.tsx carries the
// same stub.)
if (!("ResizeObserver" in globalThis)) {
  (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}

function setup(props: Partial<Parameters<typeof Tooltip>[0]> = {}) {
  return render(
    <Tooltip label="Delete this pod" {...props}>
      <button type="button">Delete</button>
    </Tooltip>,
  );
}

const target = () => screen.getByRole("button", { name: "Delete" });

/**
 * What this component owns: that the hint reaches assistive technology at all.
 *
 * The mock was `<span class="tip" data-tip={label}>` and nothing else — the
 * text lived in `content: attr(data-tip)` on a `::after`. A pseudo-element
 * cannot be the target of `aria-describedby` and its content is not reliably
 * exposed to a screen reader, so the hint was visual-only: whatever the button
 * needed explaining, a screen-reader user was never told. Everything below
 * exists to pin the fix. (#320)
 *
 * Deliberately absent: placement, collision handling, the pointer grace area,
 * and the open delay's exact timing. Those are Radix's, and jsdom does no
 * layout. Escape is here because WCAG 1.4.13 requires a hint to be dismissible
 * without moving the pointer or focus, and the CSS-only version could not be.
 */
describe("Tooltip", () => {
  it("shows what it is wrapped around", () => {
    setup();
    expect(target()).toBeDefined();
  });

  it("says nothing until the target is hovered or focused", () => {
    setup();
    expect(screen.queryByRole("tooltip")).toBeNull();
  });

  it("reaches assistive technology, described by a real element", async () => {
    // The whole point. `aria-describedby` needs an id, an id needs an element,
    // and `::after` is not one.
    setup();
    await userEvent.tab();
    const tip = await screen.findByRole("tooltip");
    expect(tip.textContent).toBe("Delete this pod");
    expect(target().getAttribute("aria-describedby")).toBe(tip.id);
  });

  it("appears for a keyboard user", async () => {
    setup();
    await userEvent.tab();
    expect(document.activeElement).toBe(target());
    expect(await screen.findByRole("tooltip")).toBeDefined();
  });

  it("appears for a pointer user", async () => {
    setup();
    await userEvent.hover(target());
    expect(await screen.findByRole("tooltip")).toBeDefined();
  });

  it("goes away again when focus leaves", async () => {
    setup();
    await userEvent.tab();
    await screen.findByRole("tooltip");
    fireEvent.blur(target());
    await waitFor(() => expect(screen.queryByRole("tooltip")).toBeNull());
  });

  it("dismisses on Escape", async () => {
    // WCAG 1.4.13: content shown on hover or focus must be dismissible without
    // moving either. The CSS version had no way to do this — the hint sat over
    // whatever was behind it until the pointer moved off.
    setup();
    await userEvent.tab();
    await screen.findByRole("tooltip");
    fireEvent.keyDown(document, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("tooltip")).toBeNull());
  });

  it("leaves the target's own accessible name alone", async () => {
    // A description, not a name. Naming the target with the hint would replace
    // what the button says with an explanation of it, which a speech-input user
    // then cannot say.
    setup();
    expect(screen.getByRole("button", { name: "Delete" })).toBeDefined();
    await userEvent.tab();
    await screen.findByRole("tooltip");
    expect(screen.getByRole("button", { name: "Delete" })).toBeDefined();
  });

  it("renders in a portal, out of the tree it was declared in", async () => {
    // The reason a real element beats the `::after`: `.popover`, `.card` and
    // the panes all clip, and an absolutely positioned pseudo-element inside
    // one of them is cut off. A portalled node is not.
    const { container } = setup();
    await userEvent.tab();
    expect(container.contains(await screen.findByRole("tooltip"))).toBe(false);
  });

  it("needs no setup from the caller to work", async () => {
    // Radix's Tooltip throws unless a TooltipProvider is somewhere above it.
    // That ceremony is this component's to carry, not every call site's: a kit
    // primitive that throws because an app forgot a root wrapper is worse than
    // one that gives up a shared skip-delay window.
    render(
      <Tooltip label="Restart the deployment">
        <button type="button">Restart</button>
      </Tooltip>,
    );
    await userEvent.tab();
    expect(await screen.findByRole("tooltip")).toBeDefined();
  });

  it("makes a keyboard-reachable target out of content that is not one", async () => {
    // `.tip` was a plain span, so its `:focus-within` only ever fired when the
    // caller happened to wrap something focusable. Wrapped around a badge or a
    // truncated string — which is most of them — the hint was pointer-only.
    render(<Tooltip label="Restarted 4 times">4</Tooltip>);
    await userEvent.tab();
    expect(await screen.findByRole("tooltip")).toBeDefined();
  });

  it("stays out of the way when there is nothing to say", async () => {
    // `content: attr(data-tip)` with an empty attribute is still a box: the
    // mock drew a bare padded rectangle in the ink colour for any caller whose
    // label was computed and came back empty.
    render(
      <Tooltip label="">
        <button type="button">Delete</button>
      </Tooltip>,
    );
    await userEvent.tab();
    await userEvent.hover(target());
    expect(screen.queryByRole("tooltip")).toBeNull();
    expect(target()).toBeDefined();
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

async function showInSurface() {
  const view = render(
    <Surface>
      <Tooltip label="Delete this pod">
        <button type="button">Delete</button>
      </Tooltip>
    </Surface>,
  );
  await userEvent.tab();
  await screen.findByRole("tooltip");
  return view;
}

/**
 * A hint belongs to the tab the thing it is about is in.
 *
 * The window is a strip of tabs over one screen each, all of them mounted at
 * once with the inactive ones hidden by the `hidden` attribute — which a portal
 * to `document.body` escapes. A tooltip is the weakest case for this of the
 * five layers: it is tied to hover and focus, both of which the browser
 * normally takes away when the tab it is in is hidden, so most of the time it
 * closes itself. But "most of the time" is the whole gap — a hint drawn beside
 * a button that is no longer on screen is a caption on the wrong picture, and
 * the container costs one line and removes the question. (#357)
 */
describe("Tooltip inside a surface", () => {
  it("mounts into the surface's own node, so hiding the tab hides it too", async () => {
    await showInSurface();
    expect(mountedIn(screen.getByRole("tooltip"))).toBe(screen.getByTestId("host"));
  });

  it("mounts into the document body when there is no surface", async () => {
    // The fallback the gallery, the frozen classic app and most of this kit's
    // own tests rely on, and it must stay exactly as it was.
    setup();
    await userEvent.tab();
    await screen.findByRole("tooltip");
    expect(mountedIn(screen.getByRole("tooltip"))).toBe(document.body);
  });

  it("still describes its target from inside the surface", async () => {
    // The portal moved; the wiring that makes the hint reachable at all did
    // not. `aria-describedby` crosses the portal either way, and this is the
    // one property of this component that a wrong container could quietly
    // break.
    await showInSurface();
    const tip = screen.getByRole("tooltip");
    expect(screen.getByRole("button", { name: "Delete" }).getAttribute("aria-describedby")).toBe(tip.id);
  });
});
