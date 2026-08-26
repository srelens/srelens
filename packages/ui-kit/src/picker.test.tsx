import { describe, it, expect } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { Picker, PickerRow } from "./picker";
import { PortalScopeProvider, usePortalHost } from "./portal";

/**
 * jsdom does not implement the two browser APIs a floating, scrolling list is
 * built on: Radix positions the popover with a ResizeObserver watching the
 * trigger, and cmdk scrolls the highlighted row into view. Stubbed here rather
 * than in `test-setup.ts` for the reason `Combobox.test.tsx` gives — a global
 * stub hides from the next component that it needs one.
 */
if (!("ResizeObserver" in window)) {
  (window as unknown as { ResizeObserver: unknown }).ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}
const proto = window.HTMLElement.prototype as unknown as Record<string, unknown>;
proto.scrollIntoView ??= () => {};
proto.hasPointerCapture ??= () => false;
proto.setPointerCapture ??= () => {};
proto.releasePointerCapture ??= () => {};

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

function subject() {
  return (
    <Picker summary="Everything" ariaLabel="Namespace">
      {(close) => <PickerRow value="alpha" label="alpha" checked={false} onSelect={close} />}
    </Picker>
  );
}

const trigger = () => screen.getByRole("combobox", { name: "Namespace" });
const panel = () => screen.getByRole("dialog");

async function open() {
  const view = render(subject());
  await userEvent.click(trigger());
  await screen.findByRole("dialog");
  return view;
}

async function openInSurface() {
  const view = render(<Surface>{subject()}</Surface>);
  await userEvent.click(trigger());
  await screen.findByRole("dialog");
  return view;
}

/**
 * The shell `Combobox` and `MultiSelect` are both built out of. What those two
 * suites cover is what a row click does; what this one covers is the seam they
 * share with the window around them.
 *
 * A searchable list opened in one tab belongs to that tab. The window is a
 * strip of tabs over one screen each, all of them mounted at once with the
 * inactive ones hidden by the `hidden` attribute — which a portal to
 * `document.body` escapes. So the list stayed on screen over whatever tab the
 * reader moved to, anchored to a trigger that had gone with the tab. (#357)
 *
 * The container is the whole change. Radix's Popover is already non-modal and
 * already dismisses on an outside interaction, and the search box's contents
 * are a filter rather than something the reader would mind losing, so none of
 * the dialog's other treatment applies.
 */
describe("Picker inside a surface", () => {
  it("mounts into the surface's own node, so hiding the tab hides it too", async () => {
    await openInSurface();
    expect(mountedIn(panel())).toBe(screen.getByTestId("host"));
  });

  it("mounts into the document body when there is no surface", async () => {
    // The fallback the gallery, the frozen classic app and most of this kit's
    // own tests rely on, and it must stay exactly as it was.
    await open();
    expect(mountedIn(panel())).toBe(document.body);
  });

  it("still filters and picks from inside the surface", async () => {
    await openInSurface();
    await userEvent.type(screen.getByPlaceholderText("Search…"), "alp");
    await userEvent.click(screen.getByRole("option", { name: "alpha" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("still closes on Escape", async () => {
    await openInSurface();
    fireEvent.keyDown(document.activeElement ?? document.body, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });
});
