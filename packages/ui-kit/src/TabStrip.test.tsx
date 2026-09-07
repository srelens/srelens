import { describe, it, expect, vi } from "vitest";
import { useState, type FormEvent } from "react";
import { render, screen, fireEvent } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { TabStrip, type StripTab } from "./TabStrip";

// jsdom has no ResizeObserver, and Radix's popper watches the trigger and the
// content with one. The kit's shared setup does not stub it, and that setup is
// not this file's to edit, so the stub lives here. Inert: jsdom does no layout,
// so there is never a resize to report. (ColumnPicker.test.tsx and
// ContextMenu.test.tsx carry the same stub, as does apps/desktop's setup.)
if (!("ResizeObserver" in globalThis)) {
  (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}

function Boxes({ className, ...rest }: { size?: number; className?: string }) {
  return <svg data-testid="tab-icon" className={className} viewBox="0 0 24 24" {...rest} />;
}

const TABS: StripTab[] = [
  { id: "pods", title: "Pods", sub: "prod-eu", icon: Boxes },
  { id: "logs", title: "checkout-api", sub: "logs" },
  { id: "shell", title: "nginx-7d4b" },
];

function setup(props: Partial<Parameters<typeof TabStrip>[0]> = {}) {
  const onSelect = vi.fn();
  const view = render(<TabStrip tabs={TABS} activeId="pods" onSelect={onSelect} {...props} />);
  return { onSelect, ...view };
}

/** By the leading title, since the accessible name carries the sub too. */
const tab = (title: string) => screen.getByRole("tab", { name: new RegExp(`^${title}`) });

/**
 * The app's document tabs: the strip along the top of the window holding every
 * resource, log stream and shell that is open.
 *
 * Not the kit's `Tabs`, which switches views inside one screen. These close,
 * pin, carry a cluster tag, overflow into a menu and answer a right-click, and
 * the two share only a word and a stylesheet. (#332)
 *
 * The mock had no keyboard contract at all behind its `role="tablist"` — its
 * tabs were `<div role="tab">` with a click handler and no `tabIndex`, so the
 * document tab bar of a desktop app could not be reached, never mind operated,
 * without a mouse. Most of what is asserted below is that fix.
 */
describe("TabStrip", () => {
  it("renders a tab per entry and marks the active one", () => {
    setup({ activeId: "logs" });
    expect(screen.getAllByRole("tab")).toHaveLength(3);
    expect(tab("checkout-api").getAttribute("aria-selected")).toBe("true");
    expect(tab("Pods").getAttribute("aria-selected")).toBe("false");
    expect(tab("nginx-7d4b").getAttribute("aria-selected")).toBe("false");
  });

  it("names the strip", () => {
    setup({ label: "Open documents" });
    expect(screen.getByRole("tablist", { name: "Open documents" })).toBeDefined();
  });

  it("selects on click", async () => {
    const { onSelect } = setup();
    await userEvent.click(tab("checkout-api"));
    expect(onSelect).toHaveBeenCalledWith("logs");
  });

  it("names a tab by its title and its sub, and carries no title attribute", () => {
    // The mock put the whole name in `title=`, which is a tooltip: it is not
    // reachable from a keyboard, not announced reliably, and not the tab's
    // accessible name. The name is the name. (#332)
    setup();
    expect(screen.getByRole("tab", { name: "Pods · prod-eu" })).toBeDefined();
    expect(tab("Pods").hasAttribute("title")).toBe(false);
  });

  it("omits the sub element on a tab without one", () => {
    setup();
    expect(tab("nginx-7d4b").querySelector(".tab-sub")).toBeNull();
    expect(tab("Pods").querySelector(".tab-sub")?.textContent).toBe("prod-eu");
  });

  it("renders a caller's icon, hidden from assistive technology", () => {
    // Which glyph means "workloads" is the product's vocabulary. The mock kept
    // a kind→icon map in the component; the kit takes the icon per tab, the
    // way NavIcon does. (#332)
    setup();
    const icon = screen.getByTestId("tab-icon");
    expect(tab("Pods").contains(icon)).toBe(true);
    expect(icon.getAttribute("aria-hidden")).toBe("true");
  });
});

/**
 * The contract `role="tablist"` promises and the mock did not keep. Manual
 * activation rather than the selection-follows-focus that `Tabs` uses: these
 * panels are terminals, log streams and editors, and arrowing past one to reach
 * the next should not open it.
 */
describe("TabStrip keyboard behaviour", () => {
  it("is a single tab stop, landing on the active tab", async () => {
    setup({ activeId: "logs" });
    expect(tab("checkout-api").getAttribute("tabindex")).toBe("0");
    expect(tab("Pods").getAttribute("tabindex")).toBe("-1");
    expect(tab("nginx-7d4b").getAttribute("tabindex")).toBe("-1");
  });

  it("takes one Tab to enter and one to leave, whatever is open", async () => {
    render(
      <>
        <button type="button">before</button>
        <TabStrip tabs={TABS} activeId="pods" onSelect={() => {}} onClose={() => {}} onNew={() => {}} />
      </>,
    );
    screen.getByRole("button", { name: "before" }).focus();
    await userEvent.tab();
    expect(document.activeElement).toBe(tab("Pods"));
    // Not the next tab, and not the close button inside this one: the strip is
    // one stop, and the controls after it are the strip's own.
    await userEvent.tab();
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "New tab" }));
  });

  it("moves focus right and left without selecting", async () => {
    const { onSelect } = setup();
    tab("Pods").focus();
    await userEvent.keyboard("{ArrowRight}");
    expect(document.activeElement).toBe(tab("checkout-api"));
    await userEvent.keyboard("{ArrowRight}");
    expect(document.activeElement).toBe(tab("nginx-7d4b"));
    await userEvent.keyboard("{ArrowLeft}");
    expect(document.activeElement).toBe(tab("checkout-api"));
    expect(onSelect).not.toHaveBeenCalled();
  });

  it("moves the tab stop along with the focus", async () => {
    setup();
    tab("Pods").focus();
    await userEvent.keyboard("{ArrowRight}");
    expect(tab("checkout-api").getAttribute("tabindex")).toBe("0");
    expect(tab("Pods").getAttribute("tabindex")).toBe("-1");
  });

  it("does not wrap at either end", async () => {
    // Unlike `Tabs`. See the component's comment: this strip scrolls, holds
    // however many documents are open, and wrapping would yank it end to end.
    setup();
    tab("Pods").focus();
    await userEvent.keyboard("{ArrowLeft}");
    expect(document.activeElement).toBe(tab("Pods"));
    await userEvent.keyboard("{End}");
    await userEvent.keyboard("{ArrowRight}");
    expect(document.activeElement).toBe(tab("nginx-7d4b"));
  });

  it("jumps to the first and last with Home and End", async () => {
    const { onSelect } = setup({ activeId: "logs" });
    tab("checkout-api").focus();
    await userEvent.keyboard("{End}");
    expect(document.activeElement).toBe(tab("nginx-7d4b"));
    await userEvent.keyboard("{Home}");
    expect(document.activeElement).toBe(tab("Pods"));
    expect(onSelect).not.toHaveBeenCalled();
  });

  it("selects the focused tab with Enter and with Space", async () => {
    const { onSelect } = setup();
    tab("nginx-7d4b").focus();
    await userEvent.keyboard("{Enter}");
    expect(onSelect).toHaveBeenLastCalledWith("shell");
    onSelect.mockClear();
    await userEvent.keyboard(" ");
    expect(onSelect).toHaveBeenLastCalledWith("shell");
  });

  it("leaves other keys alone", async () => {
    // Otherwise the strip swallows keys the window needs.
    const onSelect = vi.fn();
    const onClose = vi.fn();
    render(<TabStrip tabs={TABS} activeId="pods" onSelect={onSelect} onClose={onClose} />);
    tab("Pods").focus();
    await userEvent.keyboard("{ArrowDown}");
    await userEvent.keyboard("a");
    expect(onSelect).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(tab("Pods"));
  });

  it("binds nothing at the window", async () => {
    // The mock installed six accelerators per instance — ⌘W, ⌘T, ⌘⇧T, ⌘[, ⌘]
    // and ⌘1-9 — so two strips answered the same keystroke and the app could
    // not tell what else the component had taken. Stripped for the same reason
    // ConsoleDock's ⌘K was: the window's keys belong to the window. (#332)
    const onClose = vi.fn();
    const onNew = vi.fn();
    const onSelect = vi.fn();
    render(<TabStrip tabs={TABS} activeId="pods" onSelect={onSelect} onClose={onClose} onNew={onNew} />);
    document.body.focus();
    await userEvent.keyboard("{Meta>}w{/Meta}");
    await userEvent.keyboard("{Meta>}t{/Meta}");
    await userEvent.keyboard("{Meta>}]{/Meta}");
    await userEvent.keyboard("{Meta>}2{/Meta}");
    expect(onClose).not.toHaveBeenCalled();
    expect(onNew).not.toHaveBeenCalled();
    expect(onSelect).not.toHaveBeenCalled();
  });
});

/**
 * Closing. The mock's only keyboard route to it was a window-level ⌘W, which
 * the component no longer owns — so the close affordance needs one of its own,
 * and Delete/Backspace on the focused tab is what the ARIA practices guide
 * recommends for exactly this.
 */
describe("TabStrip closing", () => {
  it("closes the focused tab with Delete and with Backspace", async () => {
    const onClose = vi.fn();
    render(<TabStrip tabs={TABS} activeId="pods" onSelect={() => {}} onClose={onClose} />);
    tab("checkout-api").focus();
    await userEvent.keyboard("{Delete}");
    expect(onClose).toHaveBeenLastCalledWith("logs");
    onClose.mockClear();
    await userEvent.keyboard("{Backspace}");
    expect(onClose).toHaveBeenLastCalledWith("logs");
  });

  it("closes the focused tab, not the selected one", async () => {
    const onClose = vi.fn();
    render(<TabStrip tabs={TABS} activeId="pods" onSelect={() => {}} onClose={onClose} />);
    tab("Pods").focus();
    await userEvent.keyboard("{ArrowRight}{ArrowRight}");
    await userEvent.keyboard("{Delete}");
    expect(onClose).toHaveBeenCalledWith("shell");
  });

  it("does not close a pinned tab", async () => {
    // A pin is the user saying "not this one". It shows no close button, so
    // Delete must not be the way around that.
    const onClose = vi.fn();
    const pinned: StripTab[] = [{ id: "pods", title: "Pods", pinned: true }, ...TABS.slice(1)];
    render(<TabStrip tabs={pinned} activeId="logs" onSelect={() => {}} onClose={onClose} />);
    tab("Pods").focus();
    await userEvent.keyboard("{Delete}");
    expect(onClose).not.toHaveBeenCalled();
    expect(screen.queryByRole("button", { name: /^Close Pods/ })).toBeNull();
  });

  it("does nothing on Delete when the caller offers no close", async () => {
    setup();
    tab("Pods").focus();
    await userEvent.keyboard("{Delete}");
    expect(screen.getAllByRole("tab")).toHaveLength(3);
  });

  it("offers a named close button on each closable tab", async () => {
    const onClose = vi.fn();
    render(<TabStrip tabs={TABS} activeId="pods" onSelect={() => {}} onClose={onClose} />);
    await userEvent.click(screen.getByRole("button", { name: "Close checkout-api" }));
    expect(onClose).toHaveBeenCalledWith("logs");
  });

  it("does not select the tab it closes", async () => {
    // The close button sits inside the tab, so its click reaches the tab's own
    // handler unless stopped — and closing the tab you were not on should not
    // first switch you to it.
    const onSelect = vi.fn();
    const onClose = vi.fn();
    render(<TabStrip tabs={TABS} activeId="pods" onSelect={onSelect} onClose={onClose} />);
    await userEvent.click(screen.getByRole("button", { name: "Close checkout-api" }));
    expect(onSelect).not.toHaveBeenCalled();
  });

  it("keeps the close button out of the tab order", async () => {
    // Pointer-operable, but not a stop of its own: the tab stays one focusable
    // node rather than three, and Delete is the keyboard's way to it.
    const onClose = vi.fn();
    render(<TabStrip tabs={TABS} activeId="pods" onSelect={() => {}} onClose={onClose} />);
    for (const name of ["Close Pods", "Close checkout-api", "Close nginx-7d4b"]) {
      expect(screen.getByRole("button", { name }).getAttribute("tabindex")).toBe("-1");
    }
  });

  it("closes on a middle click", async () => {
    // Kept from the mock, where it was the only way to close without the
    // window accelerator. It is a shortcut now rather than the sole route.
    const onClose = vi.fn();
    render(<TabStrip tabs={TABS} activeId="pods" onSelect={() => {}} onClose={onClose} />);
    await userEvent.pointer({ keys: "[MouseMiddle]", target: tab("checkout-api") });
    expect(onClose).toHaveBeenCalledWith("logs");
  });

  it("moves focus to a neighbour when the focused tab goes away", async () => {
    // Otherwise focus falls to the body and a keyboard user is back at the top
    // of the window after every close.
    function Harness() {
      const [tabs, setTabs] = useState(TABS);
      return (
        <TabStrip
          tabs={tabs}
          activeId="pods"
          onSelect={() => {}}
          onClose={(id) => setTabs((rest) => rest.filter((t) => t.id !== id))}
        />
      );
    }
    render(<Harness />);
    tab("checkout-api").focus();
    await userEvent.keyboard("{Delete}");
    expect(screen.queryByRole("tab", { name: /^checkout-api/ })).toBeNull();
    expect(document.activeElement).toBe(tab("nginx-7d4b"));
  });

  it("shows a pin instead of a close on a pinned tab", () => {
    const pinned: StripTab[] = [{ id: "pods", title: "Pods", pinned: true }, ...TABS.slice(1)];
    render(<TabStrip tabs={pinned} activeId="pods" onSelect={() => {}} onClose={() => {}} />);
    expect(tab("Pods").querySelector(".tab-pin")).not.toBeNull();
    expect(tab("checkout-api").querySelector(".tab-pin")).toBeNull();
  });
});

/** The two controls at the end of the strip, and what the mock made of them. */
describe("TabStrip controls", () => {
  it("offers no new-tab control unless the caller wants one", () => {
    setup();
    expect(screen.queryByRole("button", { name: "New tab" })).toBeNull();
  });

  it("names the new-tab control and reports it", async () => {
    const onNew = vi.fn();
    render(<TabStrip tabs={TABS} activeId="pods" onSelect={() => {}} onNew={onNew} newLabel="Open resource" />);
    await userEvent.click(screen.getByRole("button", { name: "Open resource" }));
    expect(onNew).toHaveBeenCalled();
  });

  it("shows the accelerator as a hint without folding it into the name", async () => {
    // The keystroke is the app's to bind; the strip only says what it is. Kept
    // out of the accessible name for the reason ContextMenu keeps its hints
    // out: "New tab command T" is not what the control does.
    render(<TabStrip tabs={TABS} activeId="pods" onSelect={() => {}} onNew={() => {}} newHint="⌘T" />);
    const button = screen.getByRole("button", { name: "New tab" });
    expect(button.getAttribute("title")).toContain("⌘T");
  });

  it("does not submit the form it is standing in", async () => {
    // A bare <button> inside a form submits it, and the tab bar of a desktop
    // app stands above whatever screen is open.
    const onSubmit = vi.fn((e: FormEvent) => e.preventDefault());
    render(
      <form onSubmit={onSubmit}>
        <TabStrip tabs={TABS} activeId="pods" onSelect={() => {}} onClose={() => {}} onNew={() => {}} />
      </form>,
    );
    await userEvent.click(screen.getByRole("button", { name: "New tab" }));
    await userEvent.click(screen.getByRole("button", { name: "Close Pods" }));
    await userEvent.click(screen.getByRole("button", { name: "All open tabs" }));
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("opens every tab in the overflow list and selects from it", async () => {
    // The mock's trigger was a `<span title="All open tabs">`: not focusable,
    // not a button, named only by a tooltip. Same fault WorkspaceSwitcher and
    // ActionBar both had. (#332)
    const { onSelect } = setup();
    const trigger = screen.getByRole("button", { name: "All open tabs" });
    expect(trigger.tagName).toBe("BUTTON");
    await userEvent.click(trigger);
    const panel = await screen.findByRole("dialog");
    for (const title of ["Pods", "checkout-api", "nginx-7d4b"]) {
      expect(screen.getByRole("button", { name: new RegExp(`^${title}`) })).toBeDefined();
    }
    await userEvent.click(screen.getByRole("button", { name: /^nginx-7d4b/ }));
    expect(onSelect).toHaveBeenCalledWith("shell");
    expect(panel.isConnected).toBe(false);
  });

  it("marks the active tab in the overflow list", async () => {
    setup({ activeId: "logs" });
    await userEvent.click(screen.getByRole("button", { name: "All open tabs" }));
    await screen.findByRole("dialog");
    expect(screen.getByRole("button", { name: /^checkout-api/ }).getAttribute("aria-current")).toBe("true");
  });
});

/** Per-tab actions, supplied by the caller rather than known here. */
describe("TabStrip context menu", () => {
  it("opens the caller's menu on a right-click and reports a pick", async () => {
    const onPick = vi.fn();
    setup({ menuFor: (t: StripTab) => [{ label: `Duplicate ${t.title}`, onPick }] });
    fireEvent.contextMenu(tab("checkout-api"));
    await screen.findByRole("menu");
    await userEvent.click(screen.getByRole("menuitem", { name: "Duplicate checkout-api" }));
    expect(onPick).toHaveBeenCalled();
  });

  it("has no menu when the caller supplies none", async () => {
    setup();
    fireEvent.contextMenu(tab("Pods"));
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("leaves the tab a tab, wrapped or not", () => {
    // Radix's trigger clones its child, and a component that swallowed the
    // roving tabIndex or the role would render correctly and never work.
    setup({ menuFor: () => [{ label: "Close", onPick: () => {} }] });
    expect(tab("Pods").getAttribute("role")).toBe("tab");
    expect(tab("Pods").getAttribute("tabindex")).toBe("0");
  });
});

describe("TabStrip with nothing open", () => {
  it("renders an empty strip that is still a strip", () => {
    render(<TabStrip tabs={[]} activeId="" onSelect={() => {}} onNew={() => {}} />);
    expect(screen.getByRole("tablist")).toBeDefined();
    expect(screen.queryAllByRole("tab")).toHaveLength(0);
    expect(screen.getByRole("button", { name: "New tab" })).toBeDefined();
    // Nothing to list, so no list.
    expect(screen.queryByRole("button", { name: "All open tabs" })).toBeNull();
  });

  it("stays reachable when the active id matches nothing", () => {
    // A caller mid-transition, or an id that has just been closed. The strip
    // must still have a tab stop, or it drops out of the tab order entirely.
    render(<TabStrip tabs={TABS} activeId="gone" onSelect={() => {}} />);
    expect(tab("Pods").getAttribute("tabindex")).toBe("0");
  });
});
