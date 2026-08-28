import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { Drawer } from "./Drawer";

/**
 * Carried over from the classic component in full. Every one of these encodes a
 * decision — the Escape exclusions, the focus hand-back, the drag clamps — and
 * none of it is re-derivable from the mock's Inspector, which has the drag and
 * nothing else. (#318)
 */
describe("Drawer", () => {
  it("renders nothing when closed", () => {
    render(
      <Drawer open={false} onClose={() => {}}>
        body
      </Drawer>,
    );
    expect(screen.queryByText("body")).toBeNull();
    expect(screen.queryByRole("complementary")).toBeNull();
  });

  it("renders title and body when open, and closes via the button", () => {
    const onClose = vi.fn();
    render(
      <Drawer open title="Pod · web-1" onClose={onClose}>
        body content
      </Drawer>,
    );
    expect(screen.getByText("Pod · web-1")).toBeDefined();
    expect(screen.getByText("body content")).toBeDefined();
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("closes on Escape when open", () => {
    const onClose = vi.fn();
    render(
      <Drawer open onClose={onClose}>
        body
      </Drawer>,
    );
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("does not close on Escape when closed", () => {
    const onClose = vi.fn();
    render(
      <Drawer open={false} onClose={onClose}>
        body
      </Drawer>,
    );
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).not.toHaveBeenCalled();
  });

  it("lets a layered modal dialog consume Escape first (does not close the drawer)", () => {
    const onClose = vi.fn();
    render(
      <Drawer open onClose={onClose}>
        body
      </Drawer>,
    );
    // Simulate an open radix dialog layered over the drawer.
    const dialog = document.createElement("div");
    dialog.setAttribute("role", "dialog");
    dialog.setAttribute("data-state", "open");
    document.body.appendChild(dialog);
    try {
      fireEvent.keyDown(document, { key: "Escape" });
      expect(onClose).not.toHaveBeenCalled();
    } finally {
      dialog.remove();
    }
  });

  it("does not hijack Escape while focus is in an editable field", () => {
    const onClose = vi.fn();
    render(
      <Drawer open onClose={onClose}>
        <input aria-label="search" />
      </Drawer>,
    );
    const input = screen.getByLabelText("search");
    input.focus();
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).not.toHaveBeenCalled();
  });

  it("takes focus when it opens and gives it back when it closes (#160)", () => {
    const opener = document.createElement("button");
    document.body.appendChild(opener);
    opener.focus();

    const { rerender } = render(
      <Drawer open={false} onClose={() => {}}>
        body
      </Drawer>,
    );
    rerender(
      <Drawer open onClose={() => {}}>
        body
      </Drawer>,
    );
    // Without this a keyboard user opening a detail panel is left at the row
    // they came from, tabbing forward through the whole list to reach it.
    expect(document.activeElement).toBe(screen.getByRole("complementary", { name: "Details" }));

    rerender(
      <Drawer open={false} onClose={() => {}}>
        body
      </Drawer>,
    );
    expect(document.activeElement).toBe(opener);
    opener.remove();
  });

  it("leaves focus alone if the user moved it elsewhere before closing", () => {
    const opener = document.createElement("button");
    const other = document.createElement("button");
    document.body.append(opener, other);
    opener.focus();

    const { rerender } = render(
      <Drawer open onClose={() => {}}>
        body
      </Drawer>,
    );
    other.focus();
    rerender(
      <Drawer open={false} onClose={() => {}}>
        body
      </Drawer>,
    );
    // Restoring here would be the panel arguing with the user on its way out.
    expect(document.activeElement).toBe(other);
    opener.remove();
    other.remove();
  });

  it("does not grow, so its width is the one the drag set", () => {
    // The drawer is documented as a flex sibling of the list. A layout class
    // that also sets `flex: 1` makes flex-basis 0 and grow 1, so the inline
    // width is ignored during sizing: the panel takes a share of the row and
    // dragging updates state without moving anything on screen. `shrink-0`
    // does not save it — that fixes only flex-shrink. (#323 review)
    //
    // Asserted against the real rule from the design's stylesheet, so this
    // fails if `.pane` is reintroduced rather than if a class name changes.
    const css = readFileSync(join(__dirname, "styles/kit.css"), "utf8");
    const pane = css.match(/\.pane\s*\{[^}]*\}/)?.[0];
    expect(pane, "the .pane rule should exist to test against").toBeTruthy();
    const style = document.createElement("style");
    style.textContent = `${pane}\n.shrink-0{flex-shrink:0}\n.flex{display:flex}`;
    document.head.appendChild(style);
    try {
      render(
        <Drawer open onClose={() => {}}>
          body
        </Drawer>,
      );
      const aside = screen.getByRole("complementary", { name: "Details" });
      // `|| "0"`: an unset flex-grow reads as "" in jsdom, and unset is the
      // initial value, which is 0. What must never hold is 1.
      expect(getComputedStyle(aside).flexGrow || "0").toBe("0");
    } finally {
      style.remove();
    }
  });

  it("is not a tab stop of its own", () => {
    render(
      <Drawer open onClose={() => {}}>
        body
      </Drawer>,
    );
    expect(
      screen.getByRole("complementary", { name: "Details" }).getAttribute("tabindex"),
    ).toBe("-1");
  });

  it("resizes by dragging the left edge, clamped to the allowed range", () => {
    const { container } = render(
      <Drawer open onClose={() => {}}>
        body
      </Drawer>,
    );
    const aside = screen.getByRole("complementary", { name: "Details" });
    expect(aside.style.width).toBe("480px");

    const handle = container.querySelector(".resize-handle");
    expect(handle).not.toBeNull();

    // Dragging left grows the panel; text selection is suppressed meanwhile.
    fireEvent.mouseDown(handle as Element, { clientX: 800 });
    expect(document.body.style.userSelect).toBe("none");
    fireEvent.mouseMove(window, { clientX: 700 });
    expect(aside.style.width).toBe("580px");

    // Clamped: far left pins at the max, far right at the min.
    fireEvent.mouseMove(window, { clientX: -2000 });
    expect(aside.style.width).toBe("960px");
    fireEvent.mouseMove(window, { clientX: 5000 });
    expect(aside.style.width).toBe("320px");

    // Releasing detaches the listeners: further movement changes nothing.
    fireEvent.mouseUp(window);
    expect(document.body.style.userSelect).toBe("");
    fireEvent.mouseMove(window, { clientX: 700 });
    expect(aside.style.width).toBe("320px");
  });
});

/**
 * Stacked drawers. `ResourceBrowser` opens the assistant from a selected
 * resource, so the detail drawer and the assistant drawer are open together in
 * the normal flow. (#323 review)
 */
describe("Drawer stacking", () => {
  function two() {
    const lower = vi.fn();
    const upper = vi.fn();
    const first = render(
      <Drawer open title="detail" onClose={lower}>
        detail
      </Drawer>,
    );
    const second = render(
      <Drawer open title="assistant" onClose={upper}>
        assistant
      </Drawer>,
    );
    return { lower, upper, first, second };
  }

  it("backs out of the innermost drawer only", () => {
    // One Escape used to dismiss both, losing the detail the user was reading
    // instead of backing out of the assistant over it.
    const { lower, upper } = two();
    fireEvent.keyDown(document, { key: "Escape" });
    expect(upper).toHaveBeenCalledTimes(1);
    expect(lower).not.toHaveBeenCalled();
  });

  it("hands Escape back to the drawer underneath", () => {
    const { lower, second } = two();
    second.unmount();
    fireEvent.keyDown(document, { key: "Escape" });
    expect(lower).toHaveBeenCalledTimes(1);
  });

  it("still lets a layered dialog take Escape ahead of every drawer", () => {
    const { lower, upper } = two();
    const dialog = document.createElement("div");
    dialog.setAttribute("role", "dialog");
    dialog.setAttribute("data-state", "open");
    document.body.appendChild(dialog);
    try {
      fireEvent.keyDown(document, { key: "Escape" });
      expect(upper).not.toHaveBeenCalled();
      expect(lower).not.toHaveBeenCalled();
    } finally {
      dialog.remove();
    }
  });

  it("does not reorder when a caller passes a new onClose each render", () => {
    // Membership is keyed on `open` alone. Keyed on the handler too, a parent
    // re-rendering the lower drawer would re-push it and steal Escape from the
    // drawer opened over it.
    const lower = vi.fn();
    const upper = vi.fn();
    const first = render(
      <Drawer open title="detail" onClose={() => lower()}>
        detail
      </Drawer>,
    );
    render(
      <Drawer open title="assistant" onClose={() => upper()}>
        assistant
      </Drawer>,
    );
    first.rerender(
      <Drawer open title="detail" onClose={() => lower()}>
        detail
      </Drawer>,
    );
    fireEvent.keyDown(document, { key: "Escape" });
    expect(upper).toHaveBeenCalledTimes(1);
    expect(lower).not.toHaveBeenCalled();
  });
});
