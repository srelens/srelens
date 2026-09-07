import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { FilterBar } from "./FilterBar";

/**
 * The bar above every list. What it owns is one text field and the way out of
 * it, so the tests are about naming that field, clearing it, and keeping the
 * clear button from submitting whatever form the list is standing in. (#320)
 */
describe("FilterBar", () => {
  const field = () => screen.getByRole("searchbox", { name: "Filter pods" });
  const clear = () => screen.getByRole("button", { name: "Clear filter" });

  function setup(props: Partial<Parameters<typeof FilterBar>[0]> = {}) {
    return render(
      <FilterBar label="Filter pods" value="" onValueChange={() => {}} {...props} />,
    );
  }

  it("names the field, rather than leaving a placeholder to do it", () => {
    // A placeholder is not a label: it is gone the moment anything is typed,
    // and it is not the accessible name.
    setup({ placeholder: "Filter pods…" });
    expect(field()).toBeDefined();
  });

  it("still shows the placeholder", () => {
    setup({ placeholder: "Filter pods…" });
    expect(field().getAttribute("placeholder")).toBe("Filter pods…");
  });

  it("is a search field", () => {
    setup();
    expect(field().getAttribute("type")).toBe("search");
  });

  it("is a search landmark, named, so it can be jumped to", () => {
    setup();
    expect(screen.getByRole("search", { name: "Filter pods" })).toBeDefined();
  });

  it("reports what is typed into it", async () => {
    const onValueChange = vi.fn();
    setup({ onValueChange });
    await userEvent.type(field(), "n");
    expect(onValueChange).toHaveBeenCalledWith("n");
  });

  it("shows the current value", () => {
    setup({ value: "nginx" });
    expect((field() as HTMLInputElement).value).toBe("nginx");
  });

  it("offers no way to clear an empty filter", () => {
    setup({ value: "" });
    expect(screen.queryByRole("button", { name: "Clear filter" })).toBeNull();
  });

  it("offers a way out once there is something to clear", () => {
    setup({ value: "nginx" });
    expect(clear()).toBeDefined();
  });

  it("clears the filter when the button is used", async () => {
    const onValueChange = vi.fn();
    setup({ value: "nginx", onValueChange });
    await userEvent.click(clear());
    expect(onValueChange).toHaveBeenCalledWith("");
  });

  it("puts focus back in the field after clearing", async () => {
    // Otherwise the next keystroke goes nowhere and the filter is over.
    setup({ value: "nginx" });
    await userEvent.click(clear());
    expect(document.activeElement).toBe(field());
  });

  it("clears the filter on Escape", async () => {
    const onValueChange = vi.fn();
    setup({ value: "nginx", onValueChange });
    await userEvent.type(field(), "{Escape}");
    expect(onValueChange).toHaveBeenCalledWith("");
  });

  it("lets Escape past when there is nothing to clear", async () => {
    // Escape closes the drawer the list is in. It only belongs to this field
    // while this field has something to give up.
    const onKeyDown = vi.fn();
    render(
      <div onKeyDown={onKeyDown}>
        <FilterBar label="Filter pods" value="" onValueChange={() => {}} />
      </div>,
    );
    await userEvent.type(field(), "{Escape}");
    expect(onKeyDown).toHaveBeenCalled();
  });

  it("keeps Escape to itself while it has a value to clear", async () => {
    const onKeyDown = vi.fn();
    render(
      <div onKeyDown={onKeyDown}>
        <FilterBar label="Filter pods" value="nginx" onValueChange={() => {}} />
      </div>,
    );
    await userEvent.type(field(), "{Escape}");
    expect(onKeyDown).not.toHaveBeenCalled();
  });

  it("does not submit the form it is standing in", async () => {
    const onSubmit = vi.fn((e: React.FormEvent) => e.preventDefault());
    render(
      <form onSubmit={onSubmit}>
        <FilterBar label="Filter pods" value="nginx" onValueChange={() => {}} />
      </form>,
    );
    await userEvent.click(clear());
    expect(onSubmit).not.toHaveBeenCalled();
    expect(clear().getAttribute("type")).toBe("button");
  });

  it("renders the controls beside it", () => {
    setup({ children: <button type="button">Only failing</button> });
    expect(screen.getByRole("button", { name: "Only failing" })).toBeDefined();
  });

  it("omits the slot beside it when it resolved to nothing", () => {
    // `{canFilter && <Toggle/>}` is how that slot gets filled, and an empty box
    // still takes its share of the row's gap.
    const { container } = setup({ children: false });
    expect(container.querySelector('[data-slot="controls"]')).toBeNull();
  });

  it("keeps the search glyph out of the reading order", () => {
    const { container } = setup();
    const glyph = container.querySelector("svg");
    expect(glyph?.getAttribute("aria-hidden")).toBe("true");
  });

  it("can be disabled while the list behind it is loading", () => {
    setup({ value: "nginx", disabled: true });
    expect((field() as HTMLInputElement).disabled).toBe(true);
    expect((clear() as HTMLButtonElement).disabled).toBe(true);
  });

  it("forwards className onto the bar", () => {
    const { container } = setup({ className: "extra" });
    expect(container.querySelector(".extra")).not.toBeNull();
  });
});
