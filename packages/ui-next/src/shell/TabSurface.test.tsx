import { describe, it, expect, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { Dialog } from "@srelens/ui-kit";
import { TabSurface } from "./TabSurface";

/** Holds a count so a remount would be visible: it would reset to 0. */
function Counter() {
  const [n, setN] = useState(0);
  return (
    <button type="button" onClick={() => setN(n + 1)}>
      count {n}
    </button>
  );
}

const surfaces = () => Array.from(document.querySelectorAll("[data-slot='tab-surface']")) as HTMLElement[];
const surface = () => surfaces()[0];
const content = () => document.querySelector("[data-slot='tab-content']") as HTMLElement;

describe("TabSurface", () => {
  it("shows its child when visible", () => {
    render(<TabSurface visible><p>the table</p></TabSurface>);
    expect(screen.getByText("the table")).toBeDefined();
    expect(surface().hidden).toBe(false);
  });

  it("hides rather than unmounts when not visible, so state survives", async () => {
    const { rerender } = render(<TabSurface visible><Counter /></TabSurface>);
    await userEvent.click(screen.getByRole("button"));
    expect(screen.getByRole("button").textContent).toBe("count 1");

    rerender(<TabSurface visible={false}><Counter /></TabSurface>);
    expect(surface().hidden).toBe(true);
    expect(screen.getByRole("button", { hidden: true }).textContent).toBe("count 1");

    rerender(<TabSurface visible><Counter /></TabSurface>);
    expect(screen.getByRole("button").textContent).toBe("count 1");
  });

  it("takes the hidden tab out of the accessibility tree and the tab order", () => {
    // `hidden` does both; `display: none` alone would too, but `hidden` is the
    // attribute that says what is meant.
    render(<TabSurface visible={false}><button type="button">inside</button></TabSurface>);
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("fills its container", () => {
    render(<TabSurface visible><p>x</p></TabSurface>);
    expect(surface().className).toContain("absolute");
    expect(surface().className).toContain("inset-0");
  });

  it("passes the tab's column layout through the content wrapper", () => {
    // The wrapper is new: it exists so the portal target can be a sibling
    // rather than a descendant. A screen inside a tab is written against a
    // full-height flex column, so the wrapper has to be one too, or every
    // ported screen loses its height the day this lands.
    render(<TabSurface visible><p>x</p></TabSurface>);
    expect(content().className).toContain("flex");
    expect(content().className).toContain("flex-col");
    expect(content().className).toContain("flex-1");
    expect(content().className).toContain("min-h-0");
  });
});

/**
 * A dialog opened in a tab belongs to that tab.
 *
 * Two halves of one bug, which have to land together. A dialog portalled to
 * `document.body` covered the tab strip, the cluster rail and the status bar,
 * so no other tab could be reached until it was dismissed — and a portal
 * escapes `hidden`, so the moment that blocking was lifted the first tab's
 * dialog would have sat on top of whatever tab the reader moved to. Mounting it
 * inside the tab's own subtree answers both at once. (#357)
 */
describe("TabSurface as a portal scope", () => {
  it("keeps a dialog opened inside it inside it", () => {
    render(
      <TabSurface visible>
        <Dialog title="Customise" onClose={() => {}}>body</Dialog>
      </TabSurface>,
    );
    expect(surface().contains(screen.getByRole("dialog"))).toBe(true);
    expect(screen.getByRole("dialog").parentElement).not.toBe(document.body);
  });

  it("hides the dialog when the tab is hidden, which a portal to the body would not", () => {
    const { rerender } = render(
      <TabSurface visible>
        <Dialog title="Customise" onClose={() => {}}>body</Dialog>
      </TabSurface>,
    );
    expect(screen.getByRole("dialog")).toBeDefined();

    rerender(
      <TabSurface visible={false}>
        <Dialog title="Customise" onClose={() => {}}>body</Dialog>
      </TabSurface>,
    );
    expect(screen.queryByRole("dialog")).toBeNull();
    // Still mounted, and still this tab's: hidden, not moved and not unmounted.
    expect(surface().contains(screen.getByRole("dialog", { hidden: true }))).toBe(true);
  });

  it("gives two tabs a dialog each, in their own tabs", () => {
    render(
      <>
        <TabSurface visible={false}>
          <Dialog title="Rename workload" onClose={() => {}}>a</Dialog>
        </TabSurface>
        <TabSurface visible>
          <Dialog title="Uninstall release" onClose={() => {}}>b</Dialog>
        </TabSurface>
      </>,
    );
    const [first, second] = surfaces();
    expect(first.contains(screen.getByRole("dialog", { hidden: true, name: "Rename workload" }))).toBe(true);
    expect(second.contains(screen.getByRole("dialog", { name: "Uninstall release" }))).toBe(true);
  });

  it("makes the tab's content unreachable while a dialog covers it", () => {
    // The overlay stops the pointer, and that is all it stops. Without this the
    // covered screen is still a tab stop and still on an assistive
    // technology's own cursor, because a non-modal Radix dialog no longer
    // applies `aria-hidden` to anything.
    const { rerender } = render(
      <TabSurface visible>
        <Dialog title="Customise" onClose={() => {}}>body</Dialog>
      </TabSurface>,
    );
    expect(content().hasAttribute("inert")).toBe(true);

    rerender(<TabSurface visible><p>the table</p></TabSurface>);
    expect(content().hasAttribute("inert")).toBe(false);
  });

  it("leaves the dialog itself reachable", () => {
    // Which is why the portal target is a sibling of the content and not
    // inside it: an inert subtree takes its own descendants with it.
    render(
      <TabSurface visible>
        <Dialog title="Customise" onClose={() => {}}>body</Dialog>
      </TabSurface>,
    );
    expect(content().contains(screen.getByRole("dialog"))).toBe(false);
    expect(surface().contains(screen.getByRole("dialog"))).toBe(true);
  });

  it("keeps a dialog, and what was typed in it, across a switch away and back", () => {
    // The reason the surface is hidden rather than unmounted, now that a dialog
    // lives inside it. A reader who leaves a half-filled dialog to check
    // another tab — the release name in Helm's uninstall gate is the case that
    // reported this — must find it exactly as they left it. (#357)
    const dialog = (
      <Dialog title="Uninstall release" onClose={() => {}}>
        <input aria-label="Release name" defaultValue="" />
      </Dialog>
    );
    const { rerender } = render(<TabSurface visible>{dialog}</TabSurface>);
    const field = screen.getByLabelText("Release name") as HTMLInputElement;
    fireEvent.change(field, { target: { value: "kube-prometheus" } });

    rerender(<TabSurface visible={false}>{dialog}</TabSurface>);
    expect(screen.queryByRole("dialog")).toBeNull();

    rerender(<TabSurface visible>{dialog}</TabSurface>);
    expect(screen.getByRole("dialog", { name: "Uninstall release" })).toBeDefined();
    expect((screen.getByLabelText("Release name") as HTMLInputElement).value).toBe("kube-prometheus");
  });

  it("does not close a dialog when the reader clicks the shell around the tab", async () => {
    // Switching tabs is a click outside the dialog, and the overlay no longer
    // covers the strip that click lands on. Standing in for the strip here,
    // because TabSurface's siblings in the window are exactly that.
    const onClose = vi.fn();
    render(
      <>
        <button type="button">another tab</button>
        <TabSurface visible>
          <Dialog title="Customise" onClose={onClose}>body</Dialog>
        </TabSurface>
      </>,
    );
    await userEvent.click(screen.getByRole("button", { name: "another tab" }));
    expect(onClose).not.toHaveBeenCalled();
    expect(screen.getByRole("dialog")).toBeDefined();
  });

  it("still closes a dialog on a click on its own overlay", async () => {
    // The other half of the same rule, kept apart from it on purpose: one loose
    // assertion about "clicks outside" would pass for both while only one of
    // them was true.
    const onClose = vi.fn();
    render(
      <TabSurface visible>
        <Dialog title="Customise" onClose={onClose}>body</Dialog>
      </TabSurface>,
    );
    await userEvent.click(document.querySelector("[data-slot='dialog-overlay']") as HTMLElement);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("makes a hidden tab's dialog ignore Escape", () => {
    // Radix routes Escape to whichever layer was opened last, which is not
    // necessarily the tab the reader is looking at.
    const onClose = vi.fn();
    render(
      <TabSurface visible={false}>
        <Dialog title="Customise" onClose={onClose}>body</Dialog>
      </TabSurface>,
    );
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).not.toHaveBeenCalled();
  });

  it("marks nothing inert with no dialog open", () => {
    render(<TabSurface visible><p>the table</p></TabSurface>);
    expect(content().hasAttribute("inert")).toBe(false);
  });
});
