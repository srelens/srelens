import { describe, it, expect, vi } from "vitest";
import { useState } from "react";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { createEvent, render, screen, fireEvent } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ResizeHandle } from "./ResizeHandle";

/**
 * A pane that actually resizes. The handle is controlled — it owns the
 * gesture and the announcement, the host owns the width — so every assertion
 * below is on a real host reacting, not on a callback in isolation.
 */
function Pane({
  edge,
  onCommit,
  initial = 240,
  min = 180,
  max = 420,
}: {
  edge?: "left" | "right";
  onCommit?: (width: number) => void;
  initial?: number;
  min?: number;
  max?: number;
}) {
  const [width, setWidth] = useState(initial);
  return (
    <div data-testid="pane" style={{ width }}>
      <ResizeHandle
        label="the pane"
        width={width}
        minWidth={min}
        maxWidth={max}
        edge={edge}
        onResize={setWidth}
        onCommit={onCommit}
      />
    </div>
  );
}

const handle = () => screen.getByRole("separator", { name: "Resize the pane" });
const paneWidth = () => screen.getByTestId("pane").style.width;

describe("ResizeHandle", () => {
  it("is a named separator carrying its current width between its bounds", () => {
    render(<Pane />);
    expect(handle().getAttribute("aria-orientation")).toBe("vertical");
    // Named after what it resizes: a separator called "separator" tells a
    // screen reader nothing about which of two panes it is about to move.
    expect(handle().getAttribute("aria-valuenow")).toBe("240");
    expect(handle().getAttribute("aria-valuemin")).toBe("180");
    expect(handle().getAttribute("aria-valuemax")).toBe("420");
    expect(handle().tabIndex).toBe(0);
  });

  it("drags from where the pointer went down, not from the window's edge", () => {
    const onCommit = vi.fn();
    render(<Pane onCommit={onCommit} />);
    fireEvent.mouseDown(handle(), { clientX: 600 });
    fireEvent.mouseMove(window, { clientX: 640 });
    expect(paneWidth()).toBe("280px");
    // Once, on release: the caller persists this.
    expect(onCommit).not.toHaveBeenCalled();
    fireEvent.mouseUp(window);
    expect(onCommit).toHaveBeenCalledTimes(1);
    expect(onCommit).toHaveBeenCalledWith(280);
  });

  it("stops listening once the drag is over", () => {
    render(<Pane />);
    fireEvent.mouseDown(handle(), { clientX: 600 });
    fireEvent.mouseUp(window);
    fireEvent.mouseMove(window, { clientX: 900 });
    expect(paneWidth()).toBe("240px");
  });

  it("leaves nothing behind when it is unmounted mid-drag", () => {
    const { unmount } = render(<Pane />);
    fireEvent.mouseDown(handle(), { clientX: 600 });
    expect(document.body.style.userSelect).toBe("none");
    unmount();
    // A stranded listener on a dead component, and a page nobody can select
    // text on any more, are the two ways this goes wrong.
    expect(document.body.style.userSelect).toBe("");
    expect(() => fireEvent.mouseMove(window, { clientX: 900 })).not.toThrow();
  });

  it("refuses to go past either end", async () => {
    render(<Pane initial={188} min={180} max={200} />);
    handle().focus();
    await userEvent.keyboard("{ArrowLeft}{ArrowLeft}");
    expect(paneWidth()).toBe("180px");
    await userEvent.keyboard("{ArrowRight}{ArrowRight}{ArrowRight}");
    expect(paneWidth()).toBe("200px");
  });

  it("keeps its keys off the page, which nothing else would notice", () => {
    // Deleted, every test here and in `Sidebar.test.tsx` stays green: Home and
    // End would also jump the page to its ends and the arrows would scroll the
    // pane, and the width would still be right. Pinned because this is now one
    // primitive two call sites depend on, and a silent drift in exactly this
    // class of detail is why it was shared rather than copied.
    render(<Pane />);
    for (const key of ["ArrowLeft", "ArrowRight", "Home", "End"]) {
      const event = createEvent.keyDown(handle(), { key });
      fireEvent(handle(), event);
      expect(event.defaultPrevented, key).toBe(true);
    }
    // A key it does not act on is left alone: swallowing Tab or Escape here
    // would cost the page its focus ring and its dialogs.
    const passed = createEvent.keyDown(handle(), { key: "Escape" });
    fireEvent(handle(), passed);
    expect(passed.defaultPrevented).toBe(false);
  });

  it("goes to the extremes with Home and End, whichever edge it is on", async () => {
    // Home and End are about the VALUE, not the direction: narrowest and
    // widest, the same two answers on either edge.
    for (const edge of ["right", "left"] as const) {
      const { unmount } = render(<Pane edge={edge} min={180} max={420} />);
      handle().focus();
      await userEvent.keyboard("{End}");
      expect(paneWidth()).toBe("420px");
      await userEvent.keyboard("{Home}");
      expect(paneWidth()).toBe("180px");
      unmount();
    }
  });
});

/**
 * The one thing that differs between the two edges.
 *
 * A pane docked on the LEFT carries its grip on its right edge and widens as
 * the pointer goes right. A pane docked on the RIGHT carries its grip on its
 * left edge and widens as the pointer goes LEFT — the edge moves left, so the
 * pane gets wider. Consistency here is with the pointer, not with the other
 * handle, and the arrow keys follow the edge for the same reason.
 */
describe("ResizeHandle on either edge", () => {
  it("widens to the right on a right-edge handle", () => {
    render(<Pane edge="right" />);
    fireEvent.mouseDown(handle(), { clientX: 600 });
    fireEvent.mouseMove(window, { clientX: 660 });
    expect(paneWidth()).toBe("300px");
    fireEvent.mouseMove(window, { clientX: 560 });
    expect(paneWidth()).toBe("200px");
  });

  it("widens to the left on a left-edge handle", () => {
    render(<Pane edge="left" />);
    fireEvent.mouseDown(handle(), { clientX: 600 });
    fireEvent.mouseMove(window, { clientX: 540 });
    expect(paneWidth()).toBe("300px");
    fireEvent.mouseMove(window, { clientX: 640 });
    expect(paneWidth()).toBe("200px");
  });

  it("maps ArrowRight to wider on a right-edge handle", async () => {
    render(<Pane edge="right" />);
    handle().focus();
    await userEvent.keyboard("{ArrowRight}");
    expect(paneWidth()).toBe("256px");
    await userEvent.keyboard("{ArrowLeft}{ArrowLeft}");
    expect(paneWidth()).toBe("224px");
  });

  it("maps ArrowLeft to wider on a left-edge handle", async () => {
    const onCommit = vi.fn();
    render(<Pane edge="left" onCommit={onCommit} />);
    handle().focus();
    await userEvent.keyboard("{ArrowLeft}");
    expect(paneWidth()).toBe("256px");
    expect(onCommit).toHaveBeenLastCalledWith(256);
    await userEvent.keyboard("{ArrowRight}{ArrowRight}");
    expect(paneWidth()).toBe("224px");
    expect(onCommit).toHaveBeenLastCalledWith(224);
  });

  it("says which edge it is on, so the stylesheet can put the rule there", () => {
    const { rerender } = render(<Pane />);
    expect(handle().dataset.edge).toBe("right");
    rerender(<Pane edge="left" />);
    expect(handle().dataset.edge).toBe("left");
  });
});

/**
 * The CSS is one-sided too: the base rule pins the handle to `right: -2px`
 * with a `border-right`, so a left-edge handle needs the mirrored rule — and
 * it has to land in the same cascade layer, or it wins and loses against the
 * wrong things.
 */
describe("the resize handle's stylesheet", () => {
  const css = readFileSync(join(__dirname, "styles", "kit.css"), "utf8");
  const components = css.slice(css.indexOf("@layer components {"), css.indexOf("@layer utilities {"));

  it("mirrors the handle onto the left edge, in the components layer", () => {
    const rule = components.slice(components.indexOf('.resize-handle[data-edge="left"] {'));
    const body = rule.slice(0, rule.indexOf("}"));
    expect(body).toContain("left: -2px");
    expect(body).toContain("border-left:");
    // Or the base rule's own border draws down the far side of the grip.
    expect(body).toMatch(/border-right:\s*none/);
  });
});
