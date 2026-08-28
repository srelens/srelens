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
