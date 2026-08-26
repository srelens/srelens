import { describe, it, expect, vi } from "vitest";
import { cleanup, render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { Dialog } from "./Dialog";
import { PortalScopeProvider, usePortalHost } from "./portal";

function setup(props: Partial<Parameters<typeof Dialog>[0]> = {}) {
  const onClose = vi.fn();
  const view = render(
    <Dialog title="Customise kind-local" onClose={onClose} footer={<button type="button">Done</button>} {...props}>
      <label>
        Display name
        <input />
      </label>
    </Dialog>,
  );
  return { onClose, ...view };
}

/**
 * What this component owns: the frame around a compact modal — its name, its
 * title, the way out of it, and where the caller's controls sit.
 *
 * Deliberately absent: the focus trap, the scroll lock and the layering. Those
 * are Radix's, for the reason {@link ConfirmDialog} sets out at length.
 * Escape and the returned focus are asserted here because both are seams this
 * component holds itself — it is mounted only while open, so there is no
 * `Dialog.Trigger` for Radix to hand focus back to.
 */
describe("Dialog", () => {
  it("is a modal named by its title", () => {
    setup();
    const dialog = screen.getByRole("dialog", { name: "Customise kind-local" });
    expect(dialog.getAttribute("aria-modal")).toBe("true");
  });

  it("shows the body and the footer it was given", () => {
    setup();
    const dialog = screen.getByRole("dialog");
    expect(within(dialog).getByLabelText(/Display name/)).toBeDefined();
    expect(within(dialog).getByRole("button", { name: "Done" })).toBeDefined();
  });

  it("closes on the header's own control", () => {
    const { onClose } = setup();
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("closes on Escape", async () => {
    const { onClose } = setup();
    fireEvent.keyDown(document.activeElement ?? document.body, { key: "Escape" });
    await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
  });

  it("moves focus into itself, and hands it back to whatever opened it", async () => {
    const opener = document.createElement("button");
    document.body.appendChild(opener);
    opener.focus();

    const { unmount } = setup();
    await waitFor(() => expect(screen.getByRole("dialog").contains(document.activeElement)).toBe(true));

    unmount();
    await waitFor(() => expect(document.activeElement).toBe(opener));
    opener.remove();
  });

  it("renders in a portal, outside the tree that mounted it", () => {
    const { container } = setup();
    expect(container.contains(screen.getByRole("dialog"))).toBe(false);
  });

  it("draws no footer rule when the caller offers no controls", () => {
    setup({ footer: undefined });
    expect(screen.getByRole("dialog").querySelector("[data-slot='dialog-footer']")).toBeNull();
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

function setupInSurface(props: Partial<Parameters<typeof Dialog>[0]> = {}, visible = true) {
  const onClose = vi.fn();
  const view = render(
    <Surface visible={visible}>
      <Dialog title="Customise kind-local" onClose={onClose} {...props}>
        <label>
          Display name
          <input />
        </label>
      </Dialog>
    </Surface>,
  );
  return { onClose, ...view };
}

const overlay = () => document.querySelector('[data-slot="dialog-overlay"]') as HTMLElement;

/**
 * A dialog opened inside a tab belongs to that tab and to nothing else.
 *
 * The window is a strip of tabs over one screen each, all of them mounted at
 * once. A dialog portalled to `document.body` covered the strip, the cluster
 * rail and the status bar, and Radix's focus trap and `aria-hidden` isolation
 * made every other tab unreachable until it was dismissed — and, because a
 * portal escapes the `hidden` attribute that hides an inactive tab, it would
 * have followed the reader to whatever tab they moved to. (#357)
 *
 * What replaces the trap is not a weaker trap: the reader is *meant* to be able
 * to leave. The dialog is modal within its tab — an overlay that covers only
 * the tab, and `inert` on the tab's own content — and non-modal outside it.
 */
describe("Dialog inside a surface", () => {
  it("mounts into the surface's own node, so hiding the tab hides it too", () => {
    setupInSurface();
    expect(screen.getByRole("dialog").parentElement).toBe(screen.getByTestId("host"));
    expect(screen.getByTestId("surface").contains(screen.getByRole("dialog"))).toBe(true);
  });

  it("mounts into the document body when there is no surface", () => {
    // The fallback the gallery, the frozen classic app and most of this kit's
    // tests rely on. Asserted as the literal parent rather than "somewhere in
    // the document": a container that was wrong but still attached would pass
    // the looser check.
    setup();
    expect(screen.getByRole("dialog").parentElement).toBe(document.body);
  });

  it("covers only its surface: the overlay is positioned against the tab, not the window", () => {
    setupInSurface();
    expect(overlay().className).toContain("absolute");
    expect(overlay().className).not.toContain("fixed");
  });

  it("centres itself in its surface rather than in the window", () => {
    setupInSurface();
    expect(screen.getByRole("dialog").className).toContain("absolute");
    expect(screen.getByRole("dialog").className).not.toContain("fixed");
  });

  it("still covers the whole window when there is no surface", () => {
    setup();
    expect(overlay().className).toContain("fixed");
    expect(overlay().className).not.toContain("absolute");
    expect(screen.getByRole("dialog").className).toContain("fixed");
  });

  it("does not claim the whole document as its modal", () => {
    // `aria-modal` tells assistive technology to ignore everything outside the
    // dialog — the tab strip and the cluster rail included, which is the exact
    // thing this change exists to keep reachable. Inside a surface the
    // isolation is `inert` on that surface's content instead, which stops at
    // the tab. Outside one there is nothing narrower to scope to, so the
    // document-wide modal is still right.
    setupInSurface();
    expect(screen.getByRole("dialog").getAttribute("aria-modal")).toBeNull();
    cleanup();
    setup();
    expect(screen.getByRole("dialog").getAttribute("aria-modal")).toBe("true");
  });

  it("leaves the rest of the window in the accessibility tree", () => {
    // The defect itself. Radix's `aria-hidden` isolation takes the *whole
    // document* out of the accessibility tree, which is what made the tab
    // strip, the cluster rail and every other tab unreachable until the dialog
    // was dismissed. Scoped, the isolation is the surface's own `inert`, and
    // it stops at the tab.
    const behind = document.createElement("div");
    behind.innerHTML = "<button>the tab strip</button>";
    document.body.appendChild(behind);
    try {
      setupInSurface();
      expect(behind.getAttribute("aria-hidden")).toBeNull();
    } finally {
      behind.remove();
    }
  });

  it("makes the tab's own content unreachable while it is open, and reachable again after", () => {
    const { unmount } = setupInSurface();
    expect(screen.getByTestId("content").hasAttribute("inert")).toBe(true);
    unmount();
  });

  it("leaves the tab's content reachable when no dialog is open", () => {
    render(<Surface><p>the table</p></Surface>);
    expect(screen.getByTestId("content").hasAttribute("inert")).toBe(false);
  });

  it("closes on Escape", async () => {
    const { onClose } = setupInSurface();
    fireEvent.keyDown(document.activeElement ?? document.body, { key: "Escape" });
    await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
  });

  it("closes on a click on the overlay", async () => {
    const { onClose } = setupInSurface();
    await userEvent.click(overlay());
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("does not close when the click lands on the shell outside its tab", async () => {
    // The half of the fix that only appears once the other half works: with
    // the overlay no longer covering the tab strip, switching tabs became an
    // outside pointer-down, and a non-modal Radix layer dismisses on any of
    // those. So a dialog would close the moment the reader clicked away to
    // another tab — losing whatever they had typed in it, on a surface that is
    // hidden rather than unmounted precisely so it would survive. The strip,
    // the cluster rail and the status bar are not this tab's, so an
    // interaction with them is not an answer to this tab's dialog. (#357)
    const strip = document.createElement("button");
    strip.textContent = "another tab";
    document.body.appendChild(strip);
    try {
      const { onClose } = setupInSurface();
      await userEvent.click(strip);
      expect(onClose).not.toHaveBeenCalled();
    } finally {
      strip.remove();
    }
  });

  it("ignores Escape while its tab is hidden", async () => {
    // Radix listens for Escape on the document and routes it to the top layer,
    // which is whichever dialog was opened last — not whichever one the reader
    // can see. A hidden tab's dialog answering a key press meant for the tab on
    // screen is the same bug as a portal escaping `hidden`, in the keyboard.
    const { onClose } = setupInSurface({}, false);
    fireEvent.keyDown(document, { key: "Escape" });
    await waitFor(() => expect(onClose).not.toHaveBeenCalled());
  });

  it("does not close on a click inside itself", async () => {
    const { onClose } = setupInSurface();
    await userEvent.click(screen.getByRole("dialog"));
    expect(onClose).not.toHaveBeenCalled();
  });

  it("still hands focus back to whatever opened it", async () => {
    const opener = document.createElement("button");
    document.body.appendChild(opener);
    opener.focus();

    const { unmount } = setupInSurface();
    await waitFor(() => expect(screen.getByRole("dialog").contains(document.activeElement)).toBe(true));

    unmount();
    await waitFor(() => expect(document.activeElement).toBe(opener));
    opener.remove();
  });
});

function setupTabbing(inSurface: boolean) {
  const onClose = vi.fn();
  const dialog = (
    <Dialog title="Customise kind-local" onClose={onClose} footer={<button type="button">Done</button>}>
      <label>
        Display name
        <input />
      </label>
    </Dialog>
  );
  const view = render(
    <>
      <button type="button">the tab strip</button>
      {inSurface ? <Surface>{dialog}</Surface> : dialog}
      <button type="button">the status bar</button>
    </>,
  );
  return { onClose, ...view };
}

const named = (name: string) => screen.getByRole("button", { name });

async function pressTabFrom(from: HTMLElement, shift = false) {
  await waitFor(() => expect(screen.getByRole("dialog").contains(document.activeElement)).toBe(true));
  from.focus();
  fireEvent.keyDown(from, { key: "Tab", shiftKey: shift });
}

/**
 * Tab is how a keyboard user leaves.
 *
 * The rest of the fix gave the reader the tab strip back, and gave it back to
 * the pointer only: Radix hands its focus scope `loop: true` unconditionally,
 * so Tab off the last control in the card came back round to the first one and
 * a keyboard user was still shut in — the one thing the scoping was for. The
 * flag is hardcoded and there is no prop for it, so the loop is not turned off
 * but out-run: this component answers Tab at the card's edge first, moves focus
 * to the neighbour outside, and Radix's own handler then finds focus somewhere
 * that is neither edge and does nothing. (#357)
 *
 * Outside a surface the loop is still right and is left alone — a document-wide
 * modal is meant to be the only thing you can reach.
 */
describe("Dialog and the Tab key", () => {
  it("lets Tab off the last control reach the window past the tab", async () => {
    setupTabbing(true);
    await pressTabFrom(named("Done"));
    expect(document.activeElement).toBe(named("the status bar"));
  });

  it("lets Shift+Tab off the first control reach the window before the tab", async () => {
    setupTabbing(true);
    await pressTabFrom(named("Close"), true);
    expect(document.activeElement).toBe(named("the tab strip"));
  });

  it("leaves Tab inside the card alone", async () => {
    setupTabbing(true);
    // Not an edge, so nothing here should touch it: the browser moves focus,
    // and jsdom's does not, which is exactly what makes this assertable.
    await pressTabFrom(screen.getByLabelText(/Display name/));
    expect(document.activeElement).toBe(screen.getByLabelText(/Display name/));
  });

  it("does not dismiss itself when the reader tabs out of it", async () => {
    // Leaving is the thing that was asked for. It is not an answer to the
    // question the dialog is asking, the same way clicking the strip is not.
    const { onClose } = setupTabbing(true);
    await pressTabFrom(named("Done"));
    await waitFor(() => expect(document.activeElement).toBe(named("the status bar")));
    expect(onClose).not.toHaveBeenCalled();
  });

  it("keeps looping when there is nothing outside the tab to reach", async () => {
    // The fallback: with no neighbour, out-running the loop would strand focus
    // on nothing, so the loop is left to do its job.
    const onClose = vi.fn();
    render(
      <Surface>
        <Dialog title="Customise kind-local" onClose={onClose} footer={<button type="button">Done</button>}>
          <label>
            Display name
            <input />
          </label>
        </Dialog>
      </Surface>,
    );
    await pressTabFrom(named("Done"));
    expect(document.activeElement).toBe(named("Close"));
  });

  it("still loops inside itself when there is no surface", async () => {
    // A dialog with no tab around it is modal over the whole document, and
    // being the only reachable thing is what that means.
    setupTabbing(false);
    await pressTabFrom(named("Done"));
    expect(document.activeElement).toBe(named("Close"));
  });
});
