import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { Tabs } from "./Tabs";

const tabs = [
  { id: "pods", label: "Pods" },
  { id: "services", label: "Services" },
  { id: "events", label: "Events" },
];

describe("Tabs", () => {
  it("marks the active tab", () => {
    render(<Tabs tabs={tabs} active="services" onChange={() => {}} />);
    expect(screen.getByRole("tab", { name: "Services" }).getAttribute("aria-selected")).toBe("true");
    expect(screen.getByRole("tab", { name: "Pods" }).getAttribute("aria-selected")).toBe("false");
  });

  it("emits the tab id on click", async () => {
    const onChange = vi.fn();
    render(<Tabs tabs={tabs} active="pods" onChange={onChange} />);
    await userEvent.click(screen.getByRole("tab", { name: "Services" }));
    expect(onChange).toHaveBeenCalledWith("services");
  });
});

/**
 * The keyboard contract Radix used to supply. These are not extras: the ARIA
 * roles promise this behaviour, so a tablist without it is worse than a row of
 * plain buttons — it tells assistive technology to expect arrow keys that do
 * nothing. (#318)
 */
describe("Tabs keyboard behaviour", () => {
  it("is a single tab stop, landing on the active tab", async () => {
    render(<Tabs tabs={tabs} active="services" onChange={() => {}} />);
    expect(screen.getByRole("tab", { name: "Services" }).getAttribute("tabindex")).toBe("0");
    expect(screen.getByRole("tab", { name: "Pods" }).getAttribute("tabindex")).toBe("-1");
    expect(screen.getByRole("tab", { name: "Events" }).getAttribute("tabindex")).toBe("-1");
  });

  it("moves right and left with the arrow keys", async () => {
    const onChange = vi.fn();
    render(<Tabs tabs={tabs} active="pods" onChange={onChange} />);
    screen.getByRole("tab", { name: "Pods" }).focus();
    await userEvent.keyboard("{ArrowRight}");
    expect(onChange).toHaveBeenLastCalledWith("services");
    onChange.mockClear();
    // Back to Pods, not on to Events. Navigation follows the focused tab, not
    // the `active` prop: a controlled parent may not have committed the change
    // yet, and computing from stale state made a second arrow key jump from a
    // tab the user was no longer on. (#323 review)
    await userEvent.keyboard("{ArrowLeft}");
    expect(onChange).toHaveBeenLastCalledWith("pods");
  });

  it("wraps at both ends", async () => {
    const onChange = vi.fn();
    const { rerender } = render(<Tabs tabs={tabs} active="events" onChange={onChange} />);
    screen.getByRole("tab", { name: "Events" }).focus();
    await userEvent.keyboard("{ArrowRight}");
    expect(onChange).toHaveBeenLastCalledWith("pods");

    onChange.mockClear();
    rerender(<Tabs tabs={tabs} active="pods" onChange={onChange} />);
    screen.getByRole("tab", { name: "Pods" }).focus();
    await userEvent.keyboard("{ArrowLeft}");
    expect(onChange).toHaveBeenLastCalledWith("events");
  });

  it("jumps to the first and last with Home and End", async () => {
    const onChange = vi.fn();
    render(<Tabs tabs={tabs} active="services" onChange={onChange} />);
    screen.getByRole("tab", { name: "Services" }).focus();
    await userEvent.keyboard("{End}");
    expect(onChange).toHaveBeenLastCalledWith("events");
    onChange.mockClear();
    await userEvent.keyboard("{Home}");
    expect(onChange).toHaveBeenLastCalledWith("pods");
  });

  it("leaves other keys alone", async () => {
    // Otherwise the strip would swallow keys the surrounding screen needs.
    const onChange = vi.fn();
    render(<Tabs tabs={tabs} active="pods" onChange={onChange} />);
    screen.getByRole("tab", { name: "Pods" }).focus();
    await userEvent.keyboard("{ArrowDown}");
    await userEvent.keyboard("a");
    expect(onChange).not.toHaveBeenCalled();
  });

  it("names the strip when given a label", () => {
    render(<Tabs tabs={tabs} active="pods" onChange={() => {}} label="Resource views" />);
    expect(screen.getByRole("tablist", { name: "Resource views" })).toBeDefined();
  });
});

describe("Tabs in a narrow container", () => {
  it("carries its own horizontal overflow", () => {
    // Each tab has a 108px minimum and flex items do not shrink past it, so
    // three tabs need 324px. A Drawer at its 320px minimum offers about 296px,
    // so without containment the strip is clipped or the whole panel body
    // scrolls sideways — and the resource detail already ships three tabs,
    // Helm four. Asserted on the stylesheet because the overflow lives there
    // and jsdom does no layout. (#323 review)
    const css = readFileSync(join(__dirname, "styles/kit.css"), "utf8");
    const strip = css.match(/\.tabstrip\s*\{[^}]*\}/)?.[0] ?? "";
    expect(strip, "the .tabstrip rule should exist").toBeTruthy();
    expect(strip).toContain("overflow-x: auto");
  });

  it("does not spend the strip's height on a scrollbar", () => {
    // The strip is 33px tall; a visible horizontal bar would eat the tabs.
    const css = readFileSync(join(__dirname, "styles/kit.css"), "utf8");
    expect(css).toMatch(/\.tabstrip\s*\{[^}]*scrollbar-width:\s*none/);
    expect(css).toContain(".tabstrip::-webkit-scrollbar");
  });
});

/**
 * The detail pane's mock draws its five panes as a compact rounded segmented
 * control, not as the window chrome's flat strip. A variant rather than a
 * rewrite: `.tabstrip`/`.tab` are the chrome's own CSS, worn by `TabStrip`,
 * and restyling them would restyle every document tab in the window. (#331)
 */
describe("Tabs variants", () => {
  it("wears the chrome's strip by default", () => {
    const { container } = render(<Tabs tabs={tabs} active="pods" onChange={() => {}} />);
    expect(container.querySelector(".tabstrip")).not.toBeNull();
    expect(container.querySelectorAll(".tab").length).toBe(3);
    expect(container.querySelector(".seg")).toBeNull();
  });

  it("wears the design system's segmented control when asked", () => {
    const { container } = render(<Tabs tabs={tabs} active="pods" onChange={() => {}} variant="segmented" />);
    expect(container.querySelector(".seg")).not.toBeNull();
    expect(container.querySelectorAll(".seg-btn").length).toBe(3);
    expect(container.querySelector(".tabstrip")).toBeNull();
  });

  it("keeps the tablist contract in either variant", () => {
    // The look changes; the roles and the roving tabindex do not.
    render(<Tabs tabs={tabs} active="services" onChange={() => {}} variant="segmented" label="Resource views" />);
    expect(screen.getByRole("tablist", { name: "Resource views" })).toBeDefined();
    expect(screen.getByRole("tab", { name: "Services" }).getAttribute("aria-selected")).toBe("true");
    expect(screen.getByRole("tab", { name: "Services" }).getAttribute("tabindex")).toBe("0");
    expect(screen.getByRole("tab", { name: "Pods" }).getAttribute("tabindex")).toBe("-1");
  });

  it("moves between segmented tabs with the arrow keys", async () => {
    const onChange = vi.fn();
    render(<Tabs tabs={tabs} active="pods" onChange={onChange} variant="segmented" />);
    screen.getByRole("tab", { name: "Pods" }).focus();
    await userEvent.keyboard("{ArrowRight}");
    expect(onChange).toHaveBeenLastCalledWith("services");
  });

  it("marks the active segmented tab with the same attribute as the strip", () => {
    // One attribute for both looks; the stylesheet reads it, not the variant.
    const { container } = render(<Tabs tabs={tabs} active="events" onChange={() => {}} variant="segmented" />);
    const on = container.querySelectorAll('[data-active="true"]');
    expect(on.length).toBe(1);
    expect(on[0].textContent).toBe("Events");
  });
});

describe("the tab strip's minimum width", () => {
  const css = readFileSync(join(__dirname, "styles/kit.css"), "utf8");
  const components = css.slice(css.indexOf("@layer components {"), css.indexOf("@layer utilities {"));

  it("does not force a Tabs strip past the width of the pane it sits in", () => {
    // `.tab` carries a 108px minimum for the window's document tabs, where a
    // one-character filename should still be a target. Five panes at that
    // minimum need 540px and the peek defaults to 352, so the strip scrolled
    // sideways at every ordinary width. Tabs opts out; the chrome keeps it.
    expect(components).toContain('.tabstrip[data-variant] .tab { min-width: 0; }');
  });

  it("leaves the window chrome's document tabs their minimum", () => {
    const rule = components.slice(components.indexOf("\n  .tab {"));
    const body = rule.slice(0, rule.indexOf("}"));
    expect(body).toContain("min-width: 108px");
  });

  it("draws the active segmented tab as a raised pill", () => {
    // The mock's look, and the same treatment `.seg-btn[data-on]` already had
    // — one segmented control in the design system, driven by either flag.
    expect(components).toMatch(/\.seg-btn\[data-active="true"\]/);
  });

  it("lets a five-tab segmented control scroll rather than clip", () => {
    const rule = components.slice(components.indexOf('.seg[data-variant="segmented"] {'));
    const body = rule.slice(0, rule.indexOf("}"));
    expect(body).toContain("overflow-x: auto");
    expect(body).toContain("scrollbar-width: none");
  });
});

/**
 * The third appearance, and the reason there is a third: the design's full
 * resource tab draws its panes as words on the page surface with an accent
 * underline beneath the active one — not the window chrome's filled strip and
 * not the peek's rounded pill. The kit resisted a third skin on principle
 * (see `SKIN`'s own note) right up until the design asked for one.
 */
describe("Tabs, underlined", () => {
  it("wears the underline skin without changing the control", async () => {
    const onChange = vi.fn();
    const { container } = render(
      <Tabs tabs={tabs} active="pods" onChange={onChange} variant="underline" label="Resource views" />,
    );
    expect(container.querySelector(".utabs")).toBeTruthy();
    expect(screen.getAllByRole("tab").every((t) => t.classList.contains("utab"))).toBe(true);
    // Same contract as the other two skins: roving tab stop, and a click emits.
    expect(screen.getByRole("tab", { name: "Pods" }).getAttribute("tabindex")).toBe("0");
    await userEvent.click(screen.getByRole("tab", { name: "Events" }));
    expect(onChange).toHaveBeenCalledWith("events");
  });

  it("marks the active tab with the accent rule and nothing else", () => {
    const css = readFileSync(join(__dirname, "styles/kit.css"), "utf8");
    const components = css.slice(css.indexOf("@layer components {"), css.indexOf("@layer utilities {"));
    const rule = components.slice(components.indexOf('  .utab[data-active="true"] {'));
    const body = rule.slice(0, rule.indexOf("}"));
    expect(body).toContain("border-bottom-color: var(--accent)");
    // No filled background: that is the window chrome's tab, and a page's
    // tabs sit on the page.
    expect(body).not.toContain("background");
  });
});
