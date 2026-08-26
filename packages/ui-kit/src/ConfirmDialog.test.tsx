import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { ConfirmDialog } from "./ConfirmDialog";
import { PortalScopeProvider, usePortalHost } from "./portal";

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

/** A tab-sized surface that owns the layers opened inside it, as TabSurface does. */
function Surface({ visible = true, children }: { visible?: boolean; children: ReactNode }) {
  const { ref, layered, scope } = usePortalHost(visible);
  return (
    <div data-testid="surface" hidden={!visible}>
      <PortalScopeProvider scope={scope}>
        <div data-testid="content" inert={layered}>
          {children}
        </div>
        <div data-testid="host" ref={ref} />
      </PortalScopeProvider>
    </div>
  );
}

function openInSurface(props: Partial<Parameters<typeof ConfirmDialog>[0]> = {}, visible = true) {
  return render(
    <Surface visible={visible}>
      <ConfirmDialog
        title="Delete pod?"
        message="This cannot be undone."
        onConfirm={() => {}}
        onCancel={() => {}}
        {...props}
      />
    </Surface>,
  );
}

/**
 * The same question asked inside one tab of a window that holds several.
 *
 * A confirmation portalled to `document.body` covered the tab strip, the
 * cluster rail and the status bar, and Radix's focus trap and `aria-hidden`
 * isolation made every other tab unreachable until it was answered — and,
 * because a portal escapes the `hidden` attribute an inactive tab is hidden
 * with, it would have followed the reader to whatever tab they moved to. So it
 * belongs to its tab: mounted in it, covering it and no more, and marking that
 * tab's own content unreachable rather than the document's. (#357)
 *
 * `busy` is untouched by any of it. The action is in flight; every way out of
 * *this* dialog is still closed. What is now open is the way out of the tab,
 * which was never what `busy` was protecting.
 */
describe("ConfirmDialog inside a surface", () => {
  it("mounts into the surface's own node, so hiding the tab hides it too", () => {
    openInSurface();
    expect(screen.getByRole("dialog").parentElement).toBe(screen.getByTestId("host"));
  });

  it("mounts into the document body when there is no surface", () => {
    // The fallback the gallery, the window chrome and most of this kit's tests
    // rely on. Asserted as the literal parent rather than "somewhere in the
    // document": a container that was wrong but still attached would pass the
    // looser check.
    open();
    expect(screen.getByRole("dialog").parentElement).toBe(document.body);
  });

  it("positions its overlay against the tab, not the window", () => {
    openInSurface();
    expect(overlay().className).toContain("absolute");
    expect(overlay().className).not.toContain("fixed");
  });

  it("centres itself in its surface rather than in the window", () => {
    openInSurface();
    expect(screen.getByRole("dialog").className).toContain("absolute");
    expect(screen.getByRole("dialog").className).not.toContain("fixed");
  });

  it("positions its overlay against the window when there is no surface", () => {
    open();
    expect(overlay().className).toContain("fixed");
    expect(overlay().className).not.toContain("absolute");
    expect(screen.getByRole("dialog").className).toContain("fixed");
  });

  it("leaves the page outside its tab in the accessibility tree", () => {
    // The inverse of the unscoped case above. Radix's `aria-hidden` isolation
    // takes the *whole document* out of the accessibility tree, which is what
    // made every other tab unreachable. Scoped, the isolation is the surface's
    // own `inert` instead, and it stops at the tab.
    const behind = document.createElement("div");
    behind.innerHTML = "<button>the tab strip</button>";
    document.body.appendChild(behind);
    try {
      openInSurface();
      expect(behind.getAttribute("aria-hidden")).toBeNull();
      expect(screen.getByRole("dialog").getAttribute("aria-modal")).toBeNull();
    } finally {
      behind.remove();
    }
  });

  it("marks its own tab's content inert while it is open", () => {
    openInSurface();
    expect(screen.getByTestId("content").hasAttribute("inert")).toBe(true);
  });

  it("clears its own tab's inert mark when it is answered", () => {
    // Rerendered rather than remounted: a fresh surface would start clear
    // whatever the dialog had done to the old one, so it would pass even if
    // the hold were never released.
    const { rerender } = render(
      <Surface>
        <ConfirmDialog title="Delete pod?" message="m" onConfirm={() => {}} onCancel={() => {}} />
      </Surface>,
    );
    expect(screen.getByTestId("content").hasAttribute("inert")).toBe(true);
    rerender(<Surface><p>the table</p></Surface>);
    expect(screen.getByTestId("content").hasAttribute("inert")).toBe(false);
  });

  it("keeps the marker Drawer looks for", () => {
    openInSurface();
    expect(document.querySelector('[role="dialog"][data-state="open"]')).not.toBeNull();
  });

  it("cancels on Escape", () => {
    const onCancel = vi.fn();
    openInSurface({ onCancel });
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it("cancels on a click on the overlay", async () => {
    const onCancel = vi.fn();
    openInSurface({ onCancel });
    await userEvent.click(overlay());
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it("does not cancel when the click lands on the shell outside its tab", async () => {
    // With the overlay no longer covering the tab strip, switching tabs became
    // an outside pointer-down, and a non-modal Radix layer dismisses on any of
    // those. Cancelling a destructive confirmation because the reader looked at
    // another tab is the wrong answer to a question they never answered. (#357)
    const strip = document.createElement("button");
    strip.textContent = "another tab";
    document.body.appendChild(strip);
    try {
      const onCancel = vi.fn();
      openInSurface({ onCancel });
      await userEvent.click(strip);
      expect(onCancel).not.toHaveBeenCalled();
    } finally {
      strip.remove();
    }
  });

  it("ignores Escape while its tab is hidden", () => {
    // Radix routes Escape to the layer opened last, not to the one the reader
    // can see. A hidden tab's question must not answer itself.
    const onCancel = vi.fn();
    openInSurface({ onCancel }, false);
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onCancel).not.toHaveBeenCalled();
  });

  it("does not cancel on a click inside the dialog", async () => {
    const onCancel = vi.fn();
    openInSurface({ onCancel });
    await userEvent.click(screen.getByRole("dialog"));
    expect(onCancel).not.toHaveBeenCalled();
  });

  it("still ignores Escape while the action is in flight", () => {
    const onCancel = vi.fn();
    openInSurface({ busy: true, onCancel });
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onCancel).not.toHaveBeenCalled();
  });

  it("still ignores the overlay while the action is in flight", async () => {
    const onCancel = vi.fn();
    openInSurface({ busy: true, onCancel });
    await userEvent.click(overlay());
    expect(onCancel).not.toHaveBeenCalled();
  });

  it("still moves focus into itself, and back to the opener on close", async () => {
    render(<button>Delete pod</button>);
    const opener = screen.getByRole("button", { name: "Delete pod" });
    opener.focus();

    const { unmount } = openInSurface();
    expect(screen.getByRole("dialog").contains(document.activeElement)).toBe(true);

    unmount();
    await waitFor(() => expect(document.activeElement).toBe(opener));
  });
});

function setupTabbing(inSurface: boolean, props: Partial<Parameters<typeof ConfirmDialog>[0]> = {}) {
  const onCancel = vi.fn();
  const dialog = (
    <ConfirmDialog
      title="Delete pod?"
      message="This cannot be undone."
      confirmLabel="Delete"
      danger
      onConfirm={() => {}}
      onCancel={onCancel}
      {...props}
    />
  );
  const view = render(
    <>
      <button type="button">the tab strip</button>
      {inSurface ? <Surface>{dialog}</Surface> : dialog}
      <button type="button">the status bar</button>
    </>,
  );
  return { onCancel, ...view };
}

const named = (name: string) => screen.getByRole("button", { name });

async function pressTabFrom(from: HTMLElement, shift = false) {
  await waitFor(() => expect(screen.getByRole("dialog").contains(document.activeElement)).toBe(true));
  from.focus();
  fireEvent.keyDown(from, { key: "Tab", shiftKey: shift });
}

/**
 * Tab is how a keyboard user leaves, and this is the dialog they most need to
 * leave: every destructive confirmation in the app is this component.
 *
 * Scoping it to its tab gave the tab strip back to the pointer only. Radix
 * hands its focus scope `loop: true` and hardcodes it — the scope loops whether
 * or not it is trapping — so Tab off the last control came back round to Cancel
 * and the reader was still shut inside a dialog that no longer had any right to
 * hold them. The loop cannot be turned off, so it is out-run: the card answers
 * Tab at its own edge first and moves focus out, and Radix's handler then finds
 * focus on neither edge and does nothing. (#357 review)
 *
 * Outside a surface the loop is still right and is left alone.
 */
describe("ConfirmDialog and the Tab key", () => {
  it("lets Tab off the last control reach the window past the tab", async () => {
    setupTabbing(true);
    await pressTabFrom(named("Delete"));
    expect(document.activeElement).toBe(named("the status bar"));
  });

  it("lets Shift+Tab off the first control reach the window before the tab", async () => {
    setupTabbing(true);
    await pressTabFrom(named("Cancel"), true);
    expect(document.activeElement).toBe(named("the tab strip"));
  });

  it("leaves Tab inside the card alone", async () => {
    // Not an edge, so nothing here should touch it: the browser moves focus,
    // and jsdom's does not, which is exactly what makes this assertable.
    setupTabbing(true);
    await pressTabFrom(named("Cancel"));
    expect(document.activeElement).toBe(named("Cancel"));
  });

  it("does not cancel itself when the reader tabs out of it", async () => {
    // Leaving is the thing that was asked for. It is not an answer to the
    // question, the same way clicking the tab strip is not.
    const { onCancel } = setupTabbing(true);
    await pressTabFrom(named("Delete"));
    await waitFor(() => expect(document.activeElement).toBe(named("the status bar")));
    expect(onCancel).not.toHaveBeenCalled();
  });

  it("keeps looping when there is nothing outside the tab to reach", async () => {
    // The fallback: with no neighbour, out-running the loop would strand focus
    // on nothing, so the loop is left to do its job.
    render(
      <Surface>
        <ConfirmDialog
          title="Delete pod?"
          message="m"
          confirmLabel="Delete"
          onConfirm={() => {}}
          onCancel={() => {}}
        />
      </Surface>,
    );
    await pressTabFrom(named("Delete"));
    expect(document.activeElement).toBe(named("Cancel"));
  });

  it("still loops inside itself when there is no surface", async () => {
    // A confirmation with no tab around it is modal over the whole document,
    // and being the only reachable thing is what that means.
    setupTabbing(false);
    await pressTabFrom(named("Delete"));
    expect(document.activeElement).toBe(named("Cancel"));
  });

  it("lets the reader leave the tab while the action is in flight", async () => {
    // `busy` closes every way out of the question, and Tab is not one of them:
    // it answers nothing. It is also the moment the card holds no tab stop at
    // all — both controls are disabled — so Radix stops Tab dead on the card
    // itself, and a reader who cannot leave has nothing left to do but watch a
    // request they can no longer cancel. (#357 review)
    setupTabbing(true, { busy: true });
    await pressTabFrom(screen.getByRole("dialog"));
    expect(document.activeElement).toBe(named("the status bar"));
  });

  it("still holds the reader while an unscoped action is in flight", async () => {
    // No tab to go back to: the document-wide modal is the whole window's
    // question and the loop is right.
    setupTabbing(false, { busy: true });
    await pressTabFrom(screen.getByRole("dialog"));
    expect(document.activeElement).toBe(screen.getByRole("dialog"));
  });
});

/** The answer is deferred to the end of the dispatch; this is the end of it. */
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

function openTwoTabs() {
  const onVisible = vi.fn();
  const onHidden = vi.fn();
  render(
    <>
      <Surface>
        <ConfirmDialog title="Delete pod?" message="m" onConfirm={() => {}} onCancel={onVisible} />
      </Surface>
      <Surface visible={false}>
        <ConfirmDialog title="Uninstall ingress-nginx?" message="m" onConfirm={() => {}} onCancel={onHidden} />
      </Surface>
    </>,
  );
  return { onVisible, onHidden };
}

/**
 * Escape belongs to the tab the reader is looking at.
 *
 * Radix routes it to the highest layer and orders layers by mount, so a
 * confirmation left open on a tab the reader switched away from swallows the
 * key from the one they opened afterwards on the tab they are looking at — and
 * with the first tab's dialog refusing to answer, as it must, nothing at all
 * happens. Two clicks and a keypress away: the Helm uninstall gate, Workloads,
 * a delete confirm, back. (#357 review)
 */
describe("ConfirmDialog and Escape across tabs", () => {
  it("answers Escape on the tab on screen, though a hidden tab's dialog opened later", async () => {
    const { onVisible } = openTwoTabs();
    fireEvent.keyDown(document, { key: "Escape" });
    await waitFor(() => expect(onVisible).toHaveBeenCalledTimes(1));
  });

  it("leaves the hidden tab's own question unanswered", async () => {
    const { onHidden } = openTwoTabs();
    fireEvent.keyDown(document, { key: "Escape" });
    await settle();
    expect(onHidden).not.toHaveBeenCalled();
  });

  it("answers once when its tab is the only one with a dialog open", async () => {
    const onCancel = vi.fn();
    openInSurface({ onCancel });
    fireEvent.keyDown(document, { key: "Escape" });
    await settle();
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it("still refuses while its own action is in flight", async () => {
    // The deferred answer is a second way in, and `busy` has to close it too.
    const onVisible = vi.fn();
    render(
      <>
        <Surface>
          <ConfirmDialog title="Delete pod?" message="m" busy onConfirm={() => {}} onCancel={onVisible} />
        </Surface>
        <Surface visible={false}>
          <ConfirmDialog title="Uninstall ingress-nginx?" message="m" onConfirm={() => {}} onCancel={() => {}} />
        </Surface>
      </>,
    );
    fireEvent.keyDown(document, { key: "Escape" });
    await settle();
    expect(onVisible).not.toHaveBeenCalled();
  });
});
