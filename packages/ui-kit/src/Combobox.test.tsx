import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Combobox } from "./Combobox";

/**
 * jsdom does not implement the two browser APIs a floating, scrolling list is
 * built on: Radix positions the popover with a ResizeObserver watching the
 * trigger, and cmdk scrolls the highlighted row into view. Both are stubbed
 * here rather than in `test-setup.ts` because these two suites are the only
 * ones in the kit that open a popover, and a global stub hides from the next
 * component that it needs one.
 */
if (!("ResizeObserver" in window)) {
  (window as unknown as { ResizeObserver: unknown }).ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}
const proto = window.HTMLElement.prototype as unknown as Record<string, unknown>;
proto.scrollIntoView ??= () => {};
proto.hasPointerCapture ??= () => false;
proto.setPointerCapture ??= () => {};
proto.releasePointerCapture ??= () => {};

const options = [
  { value: "", label: "Everything" },
  { value: "alpha" },
  { value: "beta", label: "Beta service" },
  { value: "gamma" },
];

function open(props: Partial<Parameters<typeof Combobox>[0]> = {}) {
  const result = render(
    <Combobox value="alpha" onValueChange={() => {}} options={options} ariaLabel="Scope" {...props} />,
  );
  return result;
}

const trigger = () => screen.getByRole("combobox", { name: "Scope" });
const rows = () => screen.getAllByRole("option").map((el) => el.textContent);

/**
 * What this component owns: the trigger's summary, the value-first change
 * contract, and closing on a choice. Deliberately absent are cmdk's scoring
 * algorithm and Radix's outside-click and focus handling — this suite checks
 * that the libraries are wired up and that the seams around them behave, not
 * how they work inside. (#318)
 */
describe("Combobox", () => {
  it("names the trigger with ariaLabel", () => {
    open({ ariaLabel: "Namespace filter" });
    expect(screen.getByRole("combobox", { name: "Namespace filter" })).toBeDefined();
  });

  it("summarises the current value on the trigger, preferring the option's label", () => {
    const { rerender } = open({ value: "beta" });
    expect(trigger().textContent).toContain("Beta service");
    rerender(<Combobox value="alpha" onValueChange={() => {}} options={options} ariaLabel="Scope" />);
    expect(trigger().textContent).toContain("alpha");
  });

  it("summarises an empty-string value like any other, when an option carries it", () => {
    // `""` is a real value here, not a sentinel for "nothing chosen": callers
    // use it for the option that means "no filter".
    open({ value: "" });
    expect(trigger().textContent).toContain("Everything");
  });

  it("falls back to the placeholder when the value matches no option", () => {
    open({ value: "nothing-like-this", placeholder: "Pick one…" });
    expect(trigger().textContent).toContain("Pick one…");
  });

  it("gives the trigger an explicit button type", () => {
    // These sit in toolbars, and a toolbar can stand inside a form; a bare
    // button would submit it.
    open();
    expect(trigger().getAttribute("type")).toBe("button");
  });

  it("merges the caller's className onto the trigger", () => {
    open({ className: "w-40" });
    expect(trigger().className).toContain("w-40");
  });

  it("opens the popover and lists every option", async () => {
    open();
    expect(screen.queryByRole("option")).toBeNull();
    await userEvent.click(trigger());
    expect(rows()).toEqual(["Everything", "alpha", "Beta service", "gamma"]);
  });

  it("announces on the trigger whether the popover is open", async () => {
    open();
    expect(trigger().getAttribute("aria-expanded")).toBe("false");
    await userEvent.click(trigger());
    expect(trigger().getAttribute("aria-expanded")).toBe("true");
  });

  it("reports the chosen option's value, not the label it was shown under", async () => {
    const onValueChange = vi.fn();
    open({ onValueChange });
    await userEvent.click(trigger());
    await userEvent.click(screen.getByRole("option", { name: "Beta service" }));
    expect(onValueChange).toHaveBeenCalledWith("beta");
  });

  it("closes once an option is chosen", async () => {
    open();
    await userEvent.click(trigger());
    await userEvent.click(screen.getByRole("option", { name: "gamma" }));
    expect(screen.queryByRole("option")).toBeNull();
    expect(trigger().getAttribute("aria-expanded")).toBe("false");
  });

  it("marks the current option as checked", async () => {
    open({ value: "beta" });
    await userEvent.click(trigger());
    // cmdk owns `aria-selected` on these rows — it means "highlighted", and it
    // follows the pointer and the arrow keys. So the chosen row says so with
    // `aria-checked`, which nothing else is using.
    expect(screen.getByRole("option", { name: "Beta service" }).getAttribute("aria-checked")).toBe("true");
    expect(screen.getByRole("option", { name: "gamma" }).getAttribute("aria-checked")).toBe("false");
  });

  it("offers a search box named by searchPlaceholder", async () => {
    open({ searchPlaceholder: "Search scopes…" });
    await userEvent.click(trigger());
    expect(screen.getByPlaceholderText("Search scopes…")).toBeDefined();
  });

  it("shows the empty state when there is nothing to choose from", async () => {
    open({ options: [] });
    await userEvent.click(trigger());
    expect(screen.queryByRole("option")).toBeNull();
    expect(screen.getByText("No results")).toBeDefined();
  });

  it("caps the list's height and scrolls it", async () => {
    // The point of reaching for this over `Select`: the option set is large.
    // Structural, because jsdom does no layout. (#318)
    open();
    await userEvent.click(trigger());
    const list = screen.getByRole("option", { name: "alpha" }).closest("[cmdk-list]");
    expect(list?.className).toContain("max-h-[240px]");
    expect(list?.className).toContain("scroll");
  });

  it("dresses the popover in the design's own surface, without fixing it in place", async () => {
    open();
    await userEvent.click(trigger());
    const panel = screen.getByRole("dialog");
    expect(panel.className).toContain("popover");
    // `.popover` positions itself with `position: fixed`, which Radix's popper
    // cannot work with: it fixes and translates a wrapper of its own, and a
    // fixed child collapses that wrapper to nothing — the very box the
    // collision logic measures. Pinned because losing it is invisible in jsdom
    // and shows up only as a popover placed against the wrong edge. (#318)
    expect(panel.style.position).toBe("relative");
  });
});
