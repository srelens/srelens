import { readFileSync } from "node:fs";
import { join } from "node:path";
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

describe("tab navigation enhancements", () => {
  it("does not open a neighbour's tooltip when a pointer close transfers focus", async () => {
    function Harness() {
      const [tabs, setTabs] = useState<StripTab[]>([{ id: "home", title: "Home", pinned: true }, { id: "pods", title: "Pods" }]);
      return <TabStrip tabs={tabs} activeId="pods" onSelect={() => {}} onClose={id => setTabs(rest => rest.filter(t => t.id !== id))} />;
    }
    render(<Harness />);
    await userEvent.click(screen.getByRole("button", { name: "Close Pods" }));
    expect(document.activeElement).toBe(tab("Home"));
    expect(screen.queryByRole("tooltip")).toBeNull();
    await userEvent.hover(tab("Home"));
    expect(await screen.findByRole("tooltip")).toBeDefined();
  });
  it("shows full identity on keyboard focus and dismisses it with Escape", async () => {
    const title = "production-monitoring-collector-configuration";
    setup({ tabs: [{ id: "one", title, sub: "short", context: "long-cluster-context", detail: "ConfigMap · monitoring" }] });
    await userEvent.tab();
    expect((await screen.findByRole("tooltip")).textContent).toContain("long-cluster-context");
    expect(screen.getByRole("tooltip").textContent).toContain("ConfigMap · monitoring");
    expect(screen.getByRole("tooltip").textContent).toContain(title);
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("tooltip")).toBeNull();
  });
  it("requests a keyboard reorder without activating a tab", () => {
    const onMove = vi.fn(); const {onSelect}=setup({onMove});
    fireEvent.keyDown(tab("checkout-api"),{key:"ArrowLeft",ctrlKey:true,shiftKey:true});
    expect(onMove).toHaveBeenCalledWith("logs",0);
    expect(onSelect).not.toHaveBeenCalled();
  });
  it("keeps edge controls visible and out of the tab order when nothing overflows", () => {
    setup();
    for(const label of ["Scroll tabs left","Scroll tabs right"]){
      const button=screen.getByRole("button",{name:label});
      expect(button).toHaveProperty("disabled",true);
      expect(button.tabIndex).toBe(-1);
    }
  });
  it("does not select or reorder a tab dropped back in its own position", () => {
    const onMove=vi.fn();const {onSelect}=setup({onMove});
    const node=tab("checkout-api");
    fireEvent.dragStart(node,{dataTransfer:{setData:vi.fn()}});
    fireEvent.dragOver(node,{clientX:0});
    fireEvent.drop(node,{clientX:0});
    fireEvent.dragEnd(node);
    fireEvent.click(node);
    expect(onMove).not.toHaveBeenCalled();expect(onSelect).not.toHaveBeenCalled();
  });
});

it("announces a completed keyboard move and keeps focus and selection separate", async () => {
  function Reorderable() {
    const [tabs,setTabs]=useState(TABS);
    return <TabStrip tabs={tabs} activeId="pods" onSelect={()=>{}} onMove={(id,to)=>setTabs(old=>{
      const next=[...old]; const [moved]=next.splice(next.findIndex(t=>t.id===id),1);next.splice(to,0,moved);return next;
    })}/>;
  }
  render(<Reorderable/>);
  tab("checkout-api").focus();
  fireEvent.keyDown(tab("checkout-api"),{key:"ArrowLeft",metaKey:true,shiftKey:true});
  expect(screen.getAllByRole("tab")[0]).toBe(tab("checkout-api"));
  expect(document.activeElement).toBe(tab("checkout-api"));
  expect(tab("Pods").getAttribute("aria-selected")).toBe("true");
  expect(screen.getByRole("status").textContent).toBe("checkout-api moved to position 1 of 3");
});

it("shows an insertion marker and requests a drop without selecting", () => {
  const onMove=vi.fn();const {onSelect}=setup({onMove});
  fireEvent.dragStart(tab("nginx-7d4b"),{dataTransfer:{setData:vi.fn()}});
  fireEvent.dragOver(tab("Pods"),{clientX:0});
  expect(tab("Pods").getAttribute("data-drop")).toBe("before");
  fireEvent.drop(tab("Pods"),{clientX:0});
  expect(onMove).toHaveBeenCalledWith("shell",0);
  expect(onSelect).not.toHaveBeenCalled();
});


it("rejects a tab drag that began on its close button and permits the next body drag", () => {
  const onMove = vi.fn();
  const onClose = vi.fn();
  const { onSelect } = setup({ onMove, onClose });
  const source = tab("nginx-7d4b");
  const close = screen.getByRole("button", { name: "Close nginx-7d4b" });
  const dataTransfer = { setData: vi.fn() };
  fireEvent.pointerDown(close.querySelector("svg") ?? close);
  // Native dragstart targets the draggable ancestor, not the pressed button.
  expect(fireEvent.dragStart(source, { dataTransfer })).toBe(false);
  fireEvent.dragOver(tab("Pods"), { clientX: 0 });
  fireEvent.drop(tab("Pods"), { clientX: 0 });
  expect(onMove).not.toHaveBeenCalled();
  expect(dataTransfer.setData).not.toHaveBeenCalled();
  expect(onSelect).not.toHaveBeenCalled();
  fireEvent.click(close);
  expect(onClose).toHaveBeenCalledWith("shell");

  fireEvent.pointerDown(source);
  expect(fireEvent.dragStart(source, { dataTransfer })).toBe(true);
  fireEvent.drop(tab("Pods"), { clientX: 0 });
  expect(onMove).toHaveBeenCalledWith("shell", 0);
});


it("keeps long tooltip identifiers on one horizontally scrollable line", () => {
  const css = readFileSync(join(__dirname, "styles/kit.css"), "utf8");
  const rule = css.match(/\.tab-tooltip\s*\{([^}]+)\}/)?.[1] ?? "";
  expect(rule).toMatch(/white-space:\s*nowrap/);
  expect(rule).toMatch(/overflow-x:\s*auto/);
  expect(rule).not.toMatch(/overflow-wrap:\s*anywhere/);
});

/**
 * #828: thirteen tabs fitted their bar exactly at the 108px minimum, so the
 * scroll controls had nothing to scroll — and each tab's title had been cut to
 * a letter beside a legible cluster name, so the tabs could not be told apart.
 */
describe("a narrow tab", () => {
  const css = readFileSync(join(__dirname, "styles/kit.css"), "utf8");
  const rule = (selector: string) =>
    css.match(new RegExp(`\\n\\s*${selector.replace(".", "\\.")}\\s*\\{([^}]+)\\}`))?.[1] ?? "";

  it("marks the title and the cluster as the two parts that share the row", () => {
    setup();
    const title = tab("Pods").querySelector(".tab-title");
    expect(title?.textContent).toBe("Pods");
    expect(tab("Pods").querySelector(".tab-sub")?.textContent).toBe("prod-eu");
    // Still truncating, both of them: neither may push the tab past its width.
    expect(title?.className).toContain("truncate");
    expect(tab("Pods").querySelector(".tab-sub")?.className).toContain("truncate");
  });

  it("gives the cluster only what the title leaves, so the cluster is what goes first", () => {
    // The title sizes to its own text and shrinks only when it alone will not
    // fit. The cluster starts from nothing and takes the leftover — the
    // opposite of two items shrinking in proportion, where the longer cluster
    // name kept the row and the title lost it.
    expect(rule(".tab-title")).toMatch(/flex:\s*0 1 auto/);
    expect(rule(".tab-title")).toMatch(/min-width:\s*0/);
    expect(rule(".tab-sub")).toMatch(/flex:\s*1 1 0/);
    expect(rule(".tab-sub")).toMatch(/min-width:\s*0/);
  });

  it("keeps the 108px floor that makes a full strip overflow and scroll", () => {
    expect(rule(".tab")).toMatch(/min-width:\s*108px/);
  });
});

describe("keeping the active tab on screen", () => {
  const MANY: StripTab[] = Array.from({ length: 14 }, (_, i) => ({ id: `t${i}`, title: `Tab ${i}`, sub: "prod-eu" }));

  /** jsdom has no layout and no `scrollIntoView`; stand one in and watch it. */
  function watchScroll() {
    const calls: { title: string | null; options: unknown }[] = [];
    const proto = window.HTMLElement.prototype as unknown as { scrollIntoView?: (options?: unknown) => void };
    const original = proto.scrollIntoView;
    proto.scrollIntoView = function (this: HTMLElement, options?: unknown) {
      calls.push({ title: this.querySelector(".tab-title")?.textContent ?? null, options });
    };
    return {
      calls,
      restore: () => {
        if (original) proto.scrollIntoView = original;
        else delete proto.scrollIntoView;
      },
    };
  }

  it("scrolls a newly activated tab into view — the one just opened at the far end", () => {
    const scroll = watchScroll();
    try {
      const view = render(<TabStrip tabs={MANY.slice(0, 13)} activeId="t0" onSelect={() => {}} />);
      scroll.calls.length = 0;

      // What opening a tab does: append it, and make it the active one.
      view.rerender(<TabStrip tabs={MANY} activeId="t13" onSelect={() => {}} />);

      expect(scroll.calls.at(-1)?.title).toBe("Tab 13");
      // Only as far as the edge that cut it off; a tab already in view stays put.
      expect(scroll.calls.at(-1)?.options).toEqual({ block: "nearest", inline: "nearest" });
    } finally {
      scroll.restore();
    }
  });

  it("brings the active tab into view when the strip first appears", () => {
    const scroll = watchScroll();
    try {
      render(<TabStrip tabs={MANY} activeId="t13" onSelect={() => {}} />);
      expect(scroll.calls.some((c) => c.title === "Tab 13")).toBe(true);
    } finally {
      scroll.restore();
    }
  });

  it("does not scroll on a render that changes neither the active tab nor the number of tabs", () => {
    const scroll = watchScroll();
    try {
      const view = render(<TabStrip tabs={MANY} activeId="t3" onSelect={() => {}} />);
      scroll.calls.length = 0;
      // A retitled tab, as a route change makes: same tabs, same active one.
      view.rerender(
        <TabStrip tabs={MANY.map((t) => (t.id === "t5" ? { ...t, title: "Renamed" } : t))} activeId="t3" onSelect={() => {}} />,
      );
      expect(scroll.calls).toEqual([]);
    } finally {
      scroll.restore();
    }
  });

  it("checks again when a tab closes, since the strip under the active tab has changed length", () => {
    const scroll = watchScroll();
    try {
      const view = render(<TabStrip tabs={MANY} activeId="t13" onSelect={() => {}} />);
      scroll.calls.length = 0;
      view.rerender(<TabStrip tabs={MANY.filter((t) => t.id !== "t2")} activeId="t13" onSelect={() => {}} />);
      expect(scroll.calls.at(-1)?.title).toBe("Tab 13");
    } finally {
      scroll.restore();
    }
  });

  describe("when the strip itself is resized (PR #837 review)", () => {
    /** The observers the strip created, so a test can fire the one on the tab list. */
    function watchResize() {
      const observers: { callback: () => void; targets: Element[]; disconnected: boolean }[] = [];
      const holder = window as unknown as { ResizeObserver?: unknown };
      const original = holder.ResizeObserver;
      holder.ResizeObserver = class {
        private entry: (typeof observers)[number];
        constructor(callback: () => void) {
          this.entry = { callback, targets: [], disconnected: false };
          observers.push(this.entry);
        }
        observe(target: Element) {
          this.entry.targets.push(target);
        }
        unobserve() {}
        disconnect() {
          this.entry.disconnected = true;
        }
      };
      return {
        /** Fire every live observer watching the tab list, as a resize does. */
        resize: () => {
          const list = screen.getByRole("tablist");
          for (const o of observers) if (!o.disconnected && o.targets.includes(list)) o.callback();
        },
        observers,
        restore: () => {
          holder.ResizeObserver = original;
        },
      };
    }

    /** jsdom reports 0 for every width; give the tab list one a test can change. */
    function setListWidth(width: number) {
      Object.defineProperty(screen.getByRole("tablist"), "clientWidth", { value: width, configurable: true });
    }

    it("brings the active tab back into view when the strip gets narrower", () => {
      const resize = watchResize();
      const scroll = watchScroll();
      try {
        render(<TabStrip tabs={MANY} activeId="t13" onSelect={() => {}} />);
        scroll.calls.length = 0;

        // The window narrowed, or the sidebar widened: same tabs, same active one.
        setListWidth(900);
        resize.resize();

        expect(scroll.calls.at(-1)?.title).toBe("Tab 13");
        expect(scroll.calls.at(-1)?.options).toEqual({ block: "nearest", inline: "nearest" });
      } finally {
        scroll.restore();
        resize.restore();
      }
    });

    it("follows the tab that is active now, not the one that was when the strip mounted", () => {
      const resize = watchResize();
      const scroll = watchScroll();
      try {
        const view = render(<TabStrip tabs={MANY} activeId="t13" onSelect={() => {}} />);
        view.rerender(<TabStrip tabs={MANY} activeId="t2" onSelect={() => {}} />);
        scroll.calls.length = 0;

        setListWidth(900);
        resize.resize();

        expect(scroll.calls.at(-1)?.title).toBe("Tab 2");
      } finally {
        scroll.restore();
        resize.restore();
      }
    });

    it("does nothing when the observer reports and the width has not changed", () => {
      // First observe, and a change of height alone, both arrive here.
      const resize = watchResize();
      const scroll = watchScroll();
      try {
        render(<TabStrip tabs={MANY} activeId="t13" onSelect={() => {}} />);
        scroll.calls.length = 0;
        resize.resize();
        expect(scroll.calls).toEqual([]);
      } finally {
        scroll.restore();
        resize.restore();
      }
    });

    it("stops watching when the strip goes away", () => {
      const resize = watchResize();
      try {
        const view = render(<TabStrip tabs={MANY} activeId="t13" onSelect={() => {}} />);
        const list = screen.getByRole("tablist");
        const watching = resize.observers.filter((o) => o.targets.includes(list));
        expect(watching.length).toBeGreaterThan(0);
        view.unmount();
        expect(watching.every((o) => o.disconnected)).toBe(true);
      } finally {
        resize.restore();
      }
    });
  });

  it("does not fail where there is no scrollIntoView at all", () => {
    expect(() => render(<TabStrip tabs={MANY} activeId="t13" onSelect={() => {}} />)).not.toThrow();
  });
});
