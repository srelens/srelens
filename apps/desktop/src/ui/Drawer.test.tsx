import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import React from "react";
import { Drawer } from "./Drawer";

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

    const handle = container.querySelector(".cursor-col-resize");
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
