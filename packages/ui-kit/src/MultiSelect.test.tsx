import { describe, it, expect, vi } from "vitest";
import { useState } from "react";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MultiSelect } from "./MultiSelect";

/** See the note in Combobox.test.tsx — same two jsdom gaps, same stubs. */
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

const options = [{ value: "alpha" }, { value: "beta", label: "Beta service" }, { value: "gamma" }];

function open(props: Partial<Parameters<typeof MultiSelect>[0]> = {}) {
  return render(
    <MultiSelect options={options} selection={[]} onChange={() => {}} ariaLabel="Scope" {...props} />,
  );
}

const trigger = () => screen.getByRole("combobox", { name: "Scope" });
// The option's accessible name rather than its text: rows carry a trailing
// count or an "only" action, which are not part of what the option is called.
const rows = () => screen.getAllByRole("option").map((el) => el.getAttribute("aria-label"));

/** A caller that actually holds the selection, so toggling behaves as it does in an app. */
function Live({ allLabel }: { allLabel?: string }) {
  const [selection, setSelection] = useState<string[]>([]);
  return (
    <MultiSelect
      options={options}
      selection={selection}
      onChange={setSelection}
      allLabel={allLabel}
      ariaLabel="Scope"
    />
  );
}

describe("MultiSelect", () => {
  it("names the trigger with ariaLabel", () => {
    open({ ariaLabel: "Scope filter" });
    expect(screen.getByRole("combobox", { name: "Scope filter" })).toBeDefined();
  });

  it("gives the trigger an explicit button type", () => {
    open();
    expect(trigger().getAttribute("type")).toBe("button");
  });

  it("merges the caller's className onto the trigger", () => {
    open({ className: "w-40" });
    expect(trigger().className).toContain("w-40");
  });

  it("summarises one selection by name and several by count", () => {
    const { rerender } = open({ selection: ["beta"] });
    expect(trigger().textContent).toContain("Beta service");
    rerender(
      <MultiSelect options={options} selection={["beta", "gamma"]} onChange={() => {}} ariaLabel="Scope" />,
    );
    expect(trigger().textContent).toContain("2 selected");
  });

  it("summarises an empty selection as allLabel when there is one", () => {
    open({ allLabel: "Everything" });
    expect(trigger().textContent).toContain("Everything");
  });

  it("summarises an empty selection as the placeholder when there is no allLabel", () => {
    open({ placeholder: "Nothing picked" });
    expect(trigger().textContent).toContain("Nothing picked");
  });

  it("opens the popover and lists every option", async () => {
    open();
    expect(screen.queryByRole("option")).toBeNull();
    await userEvent.click(trigger());
    expect(rows()).toEqual(["alpha", "Beta service", "gamma"]);
  });

  it("adds an option to the selection when toggled on", async () => {
    const onChange = vi.fn();
    open({ onChange });
    await userEvent.click(trigger());
    await userEvent.click(screen.getByRole("option", { name: "Beta service" }));
    expect(onChange).toHaveBeenCalledWith(["beta"]);
  });

  it("removes an option from the selection when toggled off", async () => {
    const onChange = vi.fn();
    open({ onChange, selection: ["alpha", "beta"] });
    await userEvent.click(trigger());
    await userEvent.click(screen.getByRole("option", { name: "alpha" }));
    expect(onChange).toHaveBeenCalledWith(["beta"]);
  });

  it("marks the selected rows as checked", async () => {
    open({ selection: ["beta"] });
    await userEvent.click(trigger());
    expect(screen.getByRole("option", { name: "Beta service" }).getAttribute("aria-checked")).toBe("true");
    expect(screen.getByRole("option", { name: "gamma" }).getAttribute("aria-checked")).toBe("false");
  });

  it("stays open while toggling, so several can be picked at once", async () => {
    render(<Live />);
    await userEvent.click(trigger());
    await userEvent.click(screen.getByRole("option", { name: "alpha" }));
    // Still open, and still showing the same list.
    expect(screen.getByRole("option", { name: "gamma" })).toBeDefined();
    await userEvent.click(screen.getByRole("option", { name: "gamma" }));
    expect(screen.getAllByRole("option").length).toBe(3);
    expect(trigger().textContent).toContain("2 selected");
  });

  it("offers a search box named by searchPlaceholder", async () => {
    open({ searchPlaceholder: "Search scopes…" });
    await userEvent.click(trigger());
    expect(screen.getByPlaceholderText("Search scopes…")).toBeDefined();
  });
});

/**
 * `allLabel` is the general form of the classic picker's "an empty selection
 * means everything" sentinel: opt in by naming the row, leave it out and an
 * empty selection is simply an empty selection.
 */
describe("MultiSelect's all row", () => {
  it("leads the list when allLabel is given", async () => {
    open({ allLabel: "Everything" });
    await userEvent.click(trigger());
    expect(rows()).toEqual(["Everything", "alpha", "Beta service", "gamma"]);
  });

  it("clears the selection", async () => {
    const onChange = vi.fn();
    open({ onChange, selection: ["alpha"], allLabel: "Everything" });
    await userEvent.click(trigger());
    await userEvent.click(screen.getByRole("option", { name: "Everything" }));
    expect(onChange).toHaveBeenCalledWith([]);
  });

  it("is checked exactly when nothing is selected", async () => {
    const { rerender } = open({ allLabel: "Everything" });
    await userEvent.click(trigger());
    expect(screen.getByRole("option", { name: "Everything" }).getAttribute("aria-checked")).toBe("true");
    rerender(
      <MultiSelect
        options={options}
        selection={["alpha"]}
        onChange={() => {}}
        allLabel="Everything"
        ariaLabel="Scope"
      />,
    );
    expect(screen.getByRole("option", { name: "Everything" }).getAttribute("aria-checked")).toBe("false");
  });

  it("is absent when allLabel is not given", async () => {
    open();
    await userEvent.click(trigger());
    expect(screen.getAllByRole("option").length).toBe(options.length);
  });

  it("is absent for a label that renders nothing", async () => {
    // `allLabel={showAll && "Everything"}` is how a caller makes the row
    // conditional, and it hands over `false`, not nothing. An emptiness test
    // that only looks for null would leave an unlabelled row in the list.
    open({ allLabel: "" });
    await userEvent.click(trigger());
    expect(screen.getAllByRole("option").length).toBe(options.length);
    expect(trigger().textContent).toContain("Select…");
  });

  it("survives an option list with nothing in it", async () => {
    // Clearing the selection has to stay reachable even when the options have
    // not arrived yet.
    open({ options: [], allLabel: "Everything", selection: ["alpha"] });
    await userEvent.click(trigger());
    expect(rows()).toEqual(["Everything"]);
  });
});

/**
 * Selected first, in the order the caller gave them, then the rest — a
 * selection made from dozens of options must not be lost among them.
 */
describe("MultiSelect's ordering", () => {
  it("hoists the selected options above the unselected ones", async () => {
    open({ selection: ["gamma"] });
    await userEvent.click(trigger());
    expect(rows()).toEqual(["gamma", "alpha", "Beta service"]);
  });

  it("keeps the selected options in the order they were given", async () => {
    open({ selection: ["gamma", "alpha"] });
    await userEvent.click(trigger());
    expect(rows()).toEqual(["gamma", "alpha", "Beta service"]);
  });

  it("leaves the caller's order alone when nothing is selected", async () => {
    open({ selection: [] });
    await userEvent.click(trigger());
    expect(rows()).toEqual(["alpha", "Beta service", "gamma"]);
  });

  it("ignores selected values that are not among the options", async () => {
    // A stale selection outlives the option list it was made from — a filter
    // restored from storage, an option that has since disappeared. It must not
    // conjure a row of its own.
    open({ selection: ["ghost", "gamma"] });
    await userEvent.click(trigger());
    expect(rows()).toEqual(["gamma", "alpha", "Beta service"]);
  });

  it("shows the empty state when there is nothing to choose from", async () => {
    open({ options: [] });
    await userEvent.click(trigger());
    expect(screen.queryByRole("option")).toBeNull();
    expect(screen.getByText("No results")).toBeDefined();
  });
});

/**
 * The design's own picker, which this took its API from but not its shape.
 * #318's rule for a component both sides have is to keep the old behaviour and
 * take the mock's visuals, and the first port kept the classic's: a check mark
 * where the design has a checkbox, and none of the affordances around the list.
 */
describe("MultiSelect matches the design's picker", () => {
  /** Render and open the popover, which is where all of this lives. */
  async function opened(props: Partial<Parameters<typeof MultiSelect>[0]> = {}) {
    const view = open(props);
    await userEvent.click(trigger());
    await screen.findByRole("listbox");
    return view;
  }

  const row = (name: RegExp) => screen.getByRole("option", { name });

  it("marks each option with a checkbox, not a check mark", async () => {
    // Options you turn on and off independently read as checkboxes; a check
    // mark is what a single-choice list uses, and Combobox is the one of those.
    await opened();
    expect(row(/alpha/).querySelector("[data-box]")).not.toBeNull();
  });

  it("says how many options there are on the all row", async () => {
    await opened({ allLabel: "Everything" });
    expect(row(/Everything/).textContent).toContain("3");
  });

  it("offers 'only' on a row, to narrow to a single option", async () => {
    // The other move people make. Doing it by hand means clearing every other
    // box first.
    const onChange = vi.fn();
    await opened({ onChange, selection: ["alpha", "gamma"] });
    await userEvent.click(screen.getByRole("button", { name: "Only Beta service" }));
    expect(onChange).toHaveBeenCalledWith(["beta"]);
  });

  it("closes after 'only', because that is the whole choice", async () => {
    await opened({ selection: ["alpha"] });
    await userEvent.click(screen.getByRole("button", { name: "Only alpha" }));
    await waitFor(() => expect(screen.queryByRole("listbox")).toBeNull());
  });

  /** The footer's count, which the trigger's summary also happens to say. */
  const footerCount = () => document.querySelector(".eyebrow")?.textContent;

  it("counts the selection in a footer", async () => {
    await opened({ selection: ["alpha", "gamma"] });
    expect(footerCount()).toContain("2 selected");
  });

  it("says 'all' in the footer when nothing is selected", async () => {
    // Not "0 selected": an empty selection is the filter switched off, not an
    // empty result.
    await opened();
    expect(footerCount()).toContain("all selected");
  });

  it("selects every option at once", async () => {
    const onChange = vi.fn();
    await opened({ onChange });
    await userEvent.click(screen.getByRole("button", { name: "Select all" }));
    expect(onChange).toHaveBeenCalledWith(["alpha", "beta", "gamma"]);
  });

  it("resets to nothing selected", async () => {
    const onChange = vi.fn();
    await opened({ onChange, selection: ["alpha"] });
    await userEvent.click(screen.getByRole("button", { name: "Reset" }));
    expect(onChange).toHaveBeenCalledWith([]);
  });

  it("gives every control it owns an explicit button type", async () => {
    // The footer sits inside a popover, which sits inside a toolbar, which
    // sits inside a form often enough to matter.
    await opened({ selection: ["alpha"] });
    for (const name of ["Select all", "Reset", "Only alpha"]) {
      expect(screen.getByRole("button", { name }).getAttribute("type")).toBe("button");
    }
  });
});
