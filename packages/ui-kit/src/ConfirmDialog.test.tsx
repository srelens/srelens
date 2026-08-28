import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ConfirmDialog } from "./ConfirmDialog";

function open(props: Partial<Parameters<typeof ConfirmDialog>[0]> = {}) {
  return render(
    <ConfirmDialog
      title="Delete pod?"
      message="This cannot be undone."
      onConfirm={() => {}}
      onCancel={() => {}}
      {...props}
    />,
  );
}

const overlay = () => document.querySelector('[data-slot="dialog-overlay"]') as HTMLElement;

/**
 * These cover what this component owns: its API, its wiring to Radix, and the
 * two behaviours that are ours rather than the library's — `busy` blocking
 * every dismissal path, and a long message scrolling instead of pushing the
 * actions out of a clipped card.
 *
 * Deliberately absent: focus trapping, layering between stacked dialogs, and
 * the scroll lock. Those were hand-written here and drew twenty-two review
 * findings, sixteen in tab-stop detection alone; they are now Radix's, and
 * asserting a dependency's internals through our component is the same mistake
 * in a new place. What is verified below is that Radix is wired up and doing
 * its job — the background really is hidden, focus really does land inside —
 * not how it achieves it. The exception is the last block: two focus cases
 * Radix's defaults do not cover for a dialog with no trigger. (#324)
 */
describe("ConfirmDialog", () => {
  it("renders title/message and fires confirm and cancel", () => {
    const onConfirm = vi.fn();
    const onCancel = vi.fn();
    render(
      <ConfirmDialog
        title="Delete pod?"
        message="This cannot be undone."
        confirmLabel="Delete"
        danger
        onConfirm={onConfirm}
        onCancel={onCancel}
      />,
    );
    expect(screen.getByRole("dialog")).toBeDefined();
    expect(screen.getByText("Delete pod?")).toBeDefined();
    fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onConfirm).toHaveBeenCalled();
    expect(onCancel).toHaveBeenCalled();
  });

  it("disables buttons while busy", () => {
    open({ busy: true });
    expect((screen.getByRole("button", { name: "Cancel" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("shows a working spinner in place of the confirm label while busy", () => {
    open({ busy: true, confirmLabel: "Apply" });
    expect(screen.queryByText("Apply")).toBeNull();
    expect(screen.getByRole("status", { name: "Working" })).toBeDefined();
  });

  it("uses the danger variant only when asked", () => {
    const { rerender } = render(
      <ConfirmDialog title="t" message="m" confirmLabel="Go" onConfirm={() => {}} onCancel={() => {}} />,
    );
    expect(screen.getByRole("button", { name: "Go" }).className).toContain("btn-accent");
    rerender(
      <ConfirmDialog title="t" message="m" confirmLabel="Go" danger onConfirm={() => {}} onCancel={() => {}} />,
    );
    expect(screen.getByRole("button", { name: "Go" }).className).toContain("btn-danger");
  });
});

describe("ConfirmDialog is wired to Radix correctly", () => {
  it("announces itself as a modal, named and described", () => {
    open();
    const dialog = screen.getByRole("dialog");
    expect(dialog.getAttribute("aria-modal")).toBe("true");
    expect(screen.getByRole("dialog", { name: "Delete pod?" })).toBeDefined();
    const describedBy = dialog.getAttribute("aria-describedby");
    expect(document.getElementById(describedBy ?? "")?.textContent).toBe("This cannot be undone.");
  });

  it("hides the page behind it from assistive technology", () => {
    // The isolation that matters more than aria-modal: background content is
    // removed from the accessibility tree, not merely flagged.
    const behind = document.createElement("div");
    behind.innerHTML = "<button>background</button>";
    document.body.appendChild(behind);
    try {
      open();
      expect(behind.getAttribute("aria-hidden")).toBe("true");
    } finally {
      behind.remove();
    }
  });

  it("keeps the marker Drawer looks for", () => {
    // Drawer defers the first Escape to a layered modal by querying exactly
    // this selector. Radix sets it, but that is worth pinning: losing it would
    // close a drawer and a dialog on one keypress, silently.
    open();
    expect(document.querySelector('[role="dialog"][data-state="open"]')).not.toBeNull();
  });

  it("moves focus into the dialog, onto Cancel", () => {
    open({ confirmLabel: "Delete", danger: true });
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Cancel" }));
  });

  it("renders in a portal, outside the tree it was declared in", () => {
    const { container } = open();
    expect(container.contains(screen.getByRole("dialog"))).toBe(false);
  });
});

describe("ConfirmDialog dismissal", () => {
  it("cancels on Escape", () => {
    const onCancel = vi.fn();
    open({ onCancel });
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it("cancels on a click on the overlay", async () => {
    const onCancel = vi.fn();
    open({ onCancel });
    await userEvent.click(overlay());
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it("does not cancel on a click inside the dialog", async () => {
    const onCancel = vi.fn();
    open({ onCancel });
    await userEvent.click(screen.getByRole("dialog"));
    expect(onCancel).not.toHaveBeenCalled();
  });
});

/**
 * `busy` is this component's own contract, not Radix's: the action is already
 * in flight, so every way out is closed until it finishes. Dismissing would
 * strand a request the user can no longer see.
 */
describe("ConfirmDialog while an action is in flight", () => {
  it("ignores Escape", () => {
    const onCancel = vi.fn();
    open({ busy: true, onCancel });
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onCancel).not.toHaveBeenCalled();
  });

  it("ignores the overlay", async () => {
    const onCancel = vi.fn();
    open({ busy: true, onCancel });
    await userEvent.click(overlay());
    expect(onCancel).not.toHaveBeenCalled();
  });

  it("leaves no enabled control to dismiss with", () => {
    open({ busy: true, confirmLabel: "Apply" });
    const buttons = screen.getAllByRole("button") as HTMLButtonElement[];
    expect(buttons.length).toBeGreaterThan(0);
    expect(buttons.every((b) => b.disabled)).toBe(true);
  });
});

describe("ConfirmDialog with tall content", () => {
  it("scrolls the message and keeps the actions in place", () => {
    // The card is capped and clips. Without an internal scroll region a long
    // message — a manifest preview, a stack of validation errors — pushes
    // Confirm and Cancel outside the clipped area with no way to reach them.
    // Structural, because jsdom does no layout. (#324 review)
    open({ message: "a very long explanation. ".repeat(200) });
    const dialog = screen.getByRole("dialog");
    const message = document.getElementById(dialog.getAttribute("aria-describedby") ?? "");
    expect(message?.className).toContain("overflow-y-auto");
    expect(message?.className).toContain("min-h-0");

    const actions = screen.getByRole("button", { name: "Cancel" }).parentElement;
    expect(actions?.className, "the action row must not shrink away").toContain("shrink-0");
  });

  it("keeps the design's own card styling", () => {
    // The visuals are unchanged by the move to Radix: the appearance still
    // comes from the kit's classes, and Radix contributes only behaviour.
    open();
    const dialog = screen.getByRole("dialog");
    expect(dialog.className).toContain("card");
    expect(dialog.querySelector(".card-head")).not.toBeNull();
    expect(dialog.querySelector(".card-title")).not.toBeNull();
  });
});

/**
 * The two focus seams this component closes itself, both downstream of it being
 * mounted only while open, so it never renders a `Dialog.Trigger`. Radix is
 * still the one trapping focus; these cover the cases where its defaults assume
 * a trigger that is not there. (#324 review)
 */
describe("ConfirmDialog focus", () => {
  it("returns focus to the opener on close", async () => {
    render(<button>Delete pod</button>);
    const opener = screen.getByRole("button", { name: "Delete pod" });
    opener.focus();

    const { unmount } = open();
    expect(document.activeElement).not.toBe(opener);

    // Radix hands focus back a tick after unmount, from its focus scope's
    // cleanup — not synchronously.
    unmount();
    await waitFor(() => expect(document.activeElement).toBe(opener));
  });

  it("keeps focus inside when confirming disables both buttons", () => {
    // Confirming flips busy, disabling the button that was just pressed; the
    // browser blurs it and, with no enabled tab stop left, focus falls to the
    // document. Radix's focus scope does not catch this — a disabled control
    // blurs with a null relatedTarget, which it ignores.
    const { rerender } = render(
      <ConfirmDialog title="t" message="m" confirmLabel="Apply" onConfirm={() => {}} onCancel={() => {}} />,
    );
    const confirm = screen.getByRole("button", { name: "Apply" });
    confirm.focus();
    expect(document.activeElement).toBe(confirm);

    // jsdom does not blur a control when it becomes disabled, so the literal
    // browser sequence cannot be reproduced here — this stands in for it by
    // putting focus where the browser would leave it, outside the dialog.
    confirm.blur();

    rerender(
      <ConfirmDialog title="t" message="m" busy confirmLabel="Apply" onConfirm={() => {}} onCancel={() => {}} />,
    );
    const dialog = screen.getByRole("dialog");
    expect(dialog.contains(document.activeElement), "focus must stay in the modal").toBe(true);
  });
});
