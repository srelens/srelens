import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import React from "react";

import { Combobox } from "./Combobox";

const options = [
  { value: "", label: "All namespaces" },
  { value: "default" },
  { value: "kube-system" },
  { value: "monitoring" },
];

beforeEach(() => vi.clearAllMocks());

describe("Combobox", () => {
  it("filters options by the search box and selects one", async () => {
    const onValueChange = vi.fn();
    render(
      <Combobox value="" onValueChange={onValueChange} options={options} ariaLabel="Namespace" searchPlaceholder="Search namespaces…" />,
    );

    await userEvent.click(screen.getByRole("combobox", { name: "Namespace" }));
    await userEvent.type(screen.getByPlaceholderText("Search namespaces…"), "kube");

    // Only the matching option remains.
    expect(screen.getByRole("option", { name: "kube-system" })).toBeDefined();
    expect(screen.queryByRole("option", { name: "default" })).toBeNull();

    await userEvent.click(screen.getByRole("option", { name: "kube-system" }));
    expect(onValueChange).toHaveBeenCalledWith("kube-system");
  });

  it("shows the selected option's label on the trigger", () => {
    render(<Combobox value="" onValueChange={vi.fn()} options={options} ariaLabel="Namespace" />);
    expect(screen.getByRole("combobox", { name: "Namespace" }).textContent).toContain("All namespaces");
  });

  it("carries a form field's invalid, required and description state on its trigger", () => {
    render(
      <>
        <p id="help">Pick one</p>
        <Combobox value="" onValueChange={vi.fn()} options={options} ariaLabel="Mode"
          ariaInvalid ariaRequired ariaDescribedBy="help" />
      </>,
    );
    const trigger = screen.getByRole("combobox", { name: "Mode" });
    expect(trigger.getAttribute("aria-invalid")).toBe("true");
    expect(trigger.getAttribute("aria-required")).toBe("true");
    expect(trigger.getAttribute("aria-describedby")).toBe("help");
  });
});
