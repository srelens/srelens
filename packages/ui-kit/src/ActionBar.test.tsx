import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ActionBar, type ActionBarAction } from "./ActionBar";

// jsdom has no ResizeObserver, and the overflow menu is a Radix popper, which
// watches trigger and content with one. The kit's shared setup does not stub
// it and is not this file's to edit. Inert: jsdom does no layout, so there is
// never a resize to report. (Popover.test.tsx carries the same stub.)
if (!("ResizeObserver" in globalThis)) {
  (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}

// Shaped like `IconComponent`, which is what the kit asks an icon to be: it
// takes a size, a class and the `aria-hidden` its host puts on it.
const Glyph = ({
  size = 12,
  className,
  "aria-hidden": ariaHidden,
}: {
  size?: number;
  className?: string;
  "aria-hidden"?: boolean | "true" | "false";
}) => (
  <svg
    data-testid="glyph"
    width={size}
    height={size}
    className={className}
    aria-hidden={ariaHidden}
  />
);

function action(label: string, extra: Partial<ActionBarAction> = {}): ActionBarAction {
  return { id: label.toLowerCase(), label, onSelect: () => {}, ...extra };
}

const four = [action("Logs"), action("Shell"), action("Restart"), action("Delete")];

function setup(props: Partial<Parameters<typeof ActionBar>[0]> = {}) {
  return render(<ActionBar label="Pod actions" actions={four} {...props} />);
}

const more = () => screen.getByRole("button", { name: "More actions" });

async function openMenu() {
  await userEvent.click(more());
  return screen.findByRole("dialog");
}

/**
 * The mock's bar was welded to the app: it read the action list off a resource
 * kind, checked each one against RBAC, and opened its own dialogs. None of that
 * can come into a design system, so what is left — and what these tests are
 * about — is the shape: how many actions stay on the bar, what happens to the
 * rest, and what an action you are not allowed to take looks like. (#320)
 */
describe("ActionBar", () => {
  it("renders an action as a button", () => {
    setup({ actions: [action("Logs")] });
    expect(screen.getByRole("button", { name: "Logs" })).toBeDefined();
  });

  it("runs the action when it is used", async () => {
    const onSelect = vi.fn();
    setup({ actions: [action("Logs", { onSelect })] });
    await userEvent.click(screen.getByRole("button", { name: "Logs" }));
    expect(onSelect).toHaveBeenCalledOnce();
  });

  it("groups the actions under a name", () => {
    // Otherwise a row of verbs sits in the page with nothing saying what they
    // act on, which for "Delete" is the whole of the question.
    setup();
    expect(screen.getByRole("group", { name: "Pod actions" })).toBeDefined();
  });

  it("renders nothing at all when there are no actions", () => {
    // An empty group is a box with padding and a name, announced to a screen
    // reader as a group containing nothing.
    const { container } = setup({ actions: [] });
    expect(container.firstChild).toBeNull();
  });

  it("does not submit the form it is standing in", async () => {
    const onSubmit = vi.fn((e: React.FormEvent) => e.preventDefault());
    render(
      <form onSubmit={onSubmit}>
        <ActionBar label="Pod actions" actions={[action("Logs")]} />
      </form>,
    );
    await userEvent.click(screen.getByRole("button", { name: "Logs" }));
    expect(onSubmit).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "Logs" }).getAttribute("type")).toBe("button");
  });

  it("draws the icon it is given, without letting it into the name", () => {
    setup({ actions: [action("Logs", { icon: Glyph })] });
    expect(screen.getByTestId("glyph").getAttribute("aria-hidden")).toBe("true");
    expect(screen.getByRole("button", { name: "Logs" })).toBeDefined();
  });

  it("marks a destructive action", () => {
    setup({ actions: [action("Delete", { danger: true })] });
    expect(screen.getByRole("button", { name: "Delete" }).className).toContain("btn-danger");
  });

  describe("when there are more actions than fit", () => {
    it("keeps the first few on the bar", () => {
      setup({ max: 2 });
      expect(screen.getByRole("button", { name: "Logs" })).toBeDefined();
      expect(screen.getByRole("button", { name: "Shell" })).toBeDefined();
    });

    it("takes the rest off the bar", () => {
      setup({ max: 2 });
      expect(screen.queryByRole("button", { name: "Restart" })).toBeNull();
      expect(screen.queryByRole("button", { name: "Delete" })).toBeNull();
    });

    it("opens the rest with a button that says what it is", async () => {
      // The mock's trigger was a <span title="More actions">: not focusable,
      // not operable by keyboard, and named by nothing an accessibility tree
      // can see.
      setup({ max: 2 });
      await openMenu();
      expect(screen.getByRole("button", { name: "Restart" })).toBeDefined();
      expect(screen.getByRole("button", { name: "Delete" })).toBeDefined();
    });

    it("reaches the overflow trigger by keyboard", async () => {
      setup({ max: 2 });
      await userEvent.tab();
      await userEvent.tab();
      await userEvent.tab();
      expect(document.activeElement).toBe(more());
    });

    it("runs an action chosen from the menu", async () => {
      const onSelect = vi.fn();
      setup({ max: 1, actions: [action("Logs"), action("Delete", { onSelect })] });
      await openMenu();
      await userEvent.click(screen.getByRole("button", { name: "Delete" }));
      expect(onSelect).toHaveBeenCalledOnce();
    });

    it("shuts the menu behind the action it ran", async () => {
      setup({ max: 1, actions: [action("Logs"), action("Delete")] });
      await openMenu();
      await userEvent.click(screen.getByRole("button", { name: "Delete" }));
      expect(screen.queryByRole("dialog")).toBeNull();
    });

    it("offers no menu when everything fits", () => {
      setup({ max: 4 });
      expect(screen.queryByRole("button", { name: "More actions" })).toBeNull();
    });

    it("takes a different word for the menu", async () => {
      setup({ max: 2, moreLabel: "More pod actions" });
      expect(screen.getByRole("button", { name: "More pod actions" })).toBeDefined();
    });

    it("renders the footer under the menu", async () => {
      setup({ max: 2, menuFooter: <button type="button">Copy kubectl</button> });
      await openMenu();
      expect(screen.getByRole("button", { name: "Copy kubectl" })).toBeDefined();
    });

    it("omits the footer when it resolved to nothing", async () => {
      // A ruled-off empty strip under the last row.
      setup({ max: 2, menuFooter: false });
      const panel = await openMenu();
      expect(panel.querySelector('[data-slot="menu-footer"]')).toBeNull();
    });
  });

  describe("when max is not a number of actions", () => {
    it("still leaves one action on the bar for a max of zero", () => {
      // `max={visibleColumns - 3}` is how a caller computes this, and the mock
      // handed the result straight to slice.
      setup({ max: 0 });
      expect(screen.getByRole("button", { name: "Logs" })).toBeDefined();
    });

    it("loses no action to a negative max", async () => {
      // `slice(0, -1)` keeps all but the last and `slice(-1)` keeps only the
      // last, so the mock silently dropped the middle of the list.
      setup({ max: -2 });
      await openMenu();
      for (const label of ["Logs", "Shell", "Restart", "Delete"]) {
        expect(screen.getByRole("button", { name: label })).toBeDefined();
      }
    });

    it("falls back for a max that is not a number at all", () => {
      setup({ max: Number.NaN });
      expect(screen.getByRole("button", { name: "Logs" })).toBeDefined();
      expect(screen.getByRole("button", { name: "Shell" })).toBeDefined();
    });
  });

  describe("an action that cannot be taken", () => {
    const blocked = [
      action("Delete", { disabledReason: "You cannot delete pods in kube-system" }),
    ];

    it("does not run when it is clicked", async () => {
      const onSelect = vi.fn();
      setup({ actions: [action("Delete", { disabledReason: "No access", onSelect })] });
      await userEvent.click(screen.getByRole("button", { name: "Delete" }));
      expect(onSelect).not.toHaveBeenCalled();
    });

    it("says so to assistive technology", () => {
      setup({ actions: blocked });
      expect(screen.getByRole("button", { name: "Delete" }).getAttribute("aria-disabled")).toBe(
        "true",
      );
    });

    it("stays focusable, so the reason can be read", async () => {
      // The mock disabled the button and wrapped it in a tooltip carrying the
      // reason. A disabled button takes no focus and, in several browsers, no
      // pointer events either — so the explanation was unreachable by keyboard
      // and unreliable by mouse.
      setup({ actions: blocked });
      const button = screen.getByRole("button", { name: "Delete" }) as HTMLButtonElement;
      expect(button.disabled).toBe(false);
      await userEvent.tab();
      expect(document.activeElement).toBe(button);
    });

    it("carries the reason as its description", () => {
      setup({ actions: blocked });
      expect(screen.getByRole("button", { name: "Delete" }).getAttribute("title")).toBe(
        "You cannot delete pods in kube-system",
      );
    });

    it("says so in words in the menu, not only in the dimming", () => {
      // Opacity is not a message.
      setup({ max: 1, actions: [action("Logs"), ...blocked] });
      return openMenu().then((panel) => {
        expect(panel.textContent).toContain("No access");
      });
    });

    it("does not run from the menu either", async () => {
      const onSelect = vi.fn();
      setup({
        max: 1,
        actions: [action("Logs"), action("Delete", { disabledReason: "No access", onSelect })],
      });
      await openMenu();
      await userEvent.click(screen.getByRole("button", { name: "Delete" }));
      expect(onSelect).not.toHaveBeenCalled();
    });

    it("leaves the menu open when a blocked row is clicked", async () => {
      // Shutting it would look like the action was taken.
      setup({ max: 1, actions: [action("Logs"), ...blocked] });
      await openMenu();
      await userEvent.click(screen.getByRole("button", { name: "Delete" }));
      expect(screen.queryByRole("dialog")).not.toBeNull();
    });
  });

  it("forwards className onto the bar", () => {
    const { container } = setup({ className: "extra" });
    expect(container.querySelector(".extra")).not.toBeNull();
  });
});
