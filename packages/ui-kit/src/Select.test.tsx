import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Select } from "./Select";

const options = [{ value: "default" }, { value: "kube-system", label: "kube-system" }];

describe("Select", () => {
  it("shows the selected value", () => {
    render(<Select value="kube-system" onValueChange={() => {}} options={options} aria-label="Namespace" />);
    const select = screen.getByRole("combobox", { name: "Namespace" }) as HTMLSelectElement;
    expect(select.value).toBe("kube-system");
  });

  it("emits value-first changes when an option is picked", async () => {
    // Picked with selectOptions rather than two clicks: this is a native
    // select, so there is no popup to open — the classic version's clicks were
    // driving Radix's listbox. The contract under test is unchanged.
    const onValueChange = vi.fn();
    render(<Select value="default" onValueChange={onValueChange} options={options} aria-label="Namespace" />);
    await userEvent.selectOptions(screen.getByRole("combobox", { name: "Namespace" }), "kube-system");
    expect(onValueChange).toHaveBeenCalledWith("kube-system");
  });

  it("treats an empty string as a real value, with no sentinel", () => {
    // Radix forbids an empty-string item value, which is the only reason the
    // classic component encoded one. A native select has no such rule.
    render(
      <Select
        value=""
        onValueChange={() => {}}
        options={[{ value: "", label: "All namespaces" }, { value: "default" }]}
        aria-label="Namespace"
      />,
    );
    const select = screen.getByRole("combobox", { name: "Namespace" }) as HTMLSelectElement;
    expect(select.value).toBe("");
    expect(screen.getByRole("option", { name: "All namespaces" })).toBeDefined();
  });

  it("falls back to the value when an option has no label", () => {
    render(<Select value="default" onValueChange={() => {}} options={options} aria-label="Namespace" />);
    expect(screen.getByRole("option", { name: "default" })).toBeDefined();
  });

  it("shows a placeholder only while nothing is selected", () => {
    const { rerender } = render(
      <Select value="" onValueChange={() => {}} options={options} placeholder="Pick one" aria-label="Namespace" />,
    );
    expect(screen.getByRole("option", { name: "Pick one" })).toBeDefined();
    // Once a real option is selected the placeholder would be a second,
    // meaningless entry competing with it.
    rerender(
      <Select value="default" onValueChange={() => {}} options={options} placeholder="Pick one" aria-label="Namespace" />,
    );
    expect(screen.queryByRole("option", { name: "Pick one" })).toBeNull();
  });

  it("shows the placeholder, not the first option, when the value matches nothing", () => {
    // A controlled value matching no option leaves the browser to pick, and it
    // picks the first enabled one — so the control claimed "a" was selected
    // while the parent state said otherwise. Showing a value nobody chose is
    // worse than showing none, and this is exactly the state where nothing is
    // chosen yet. (#322 review)
    render(
      <Select
        value="none"
        onValueChange={() => {}}
        options={[{ value: "a" }, { value: "b" }]}
        placeholder="Pick a context"
        aria-label="Context"
      />,
    );
    const select = screen.getByRole("combobox", { name: "Context" }) as HTMLSelectElement;
    expect(select.options[select.selectedIndex]?.text).toBe("Pick a context");
  });

  it("selects nothing rather than inventing a value when there is no placeholder", () => {
    render(
      <Select
        value="none"
        onValueChange={() => {}}
        options={[{ value: "a" }, { value: "b" }]}
        aria-label="Context"
      />,
    );
    const select = screen.getByRole("combobox", { name: "Context" }) as HTMLSelectElement;
    // Not "a": with nothing to land on the browser picks the first real option,
    // which is the same misleading display as the placeholder case.
    expect(select.options[select.selectedIndex]?.text).toBe("");
  });

  it("does not let the placeholder be chosen", () => {
    render(
      <Select value="" onValueChange={() => {}} options={options} placeholder="Pick one" aria-label="Namespace" />,
    );
    expect((screen.getByRole("option", { name: "Pick one" }) as HTMLOptionElement).disabled).toBe(true);
  });
});
