import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import React from "react";

import { KubectlPreview } from "./KubectlPreview";

describe("KubectlPreview", () => {
  it("labels and renders the command", () => {
    render(<KubectlPreview command="kubectl get pods web-0 --context prod" />);
    expect(screen.getByText("Equivalent kubectl:")).toBeDefined();
    expect(screen.getByText("kubectl get pods web-0 --context prod")).toBeDefined();
  });

  it("renders a note instead of a command when there's no clean equivalent", () => {
    render(<KubectlPreview note="No single-line kubectl equivalent." />);
    expect(screen.getByText("No single-line kubectl equivalent.")).toBeDefined();
    expect(screen.queryByText("Equivalent kubectl:")).toBeNull();
  });

  it("omits the copy button when no onCopy handler is given", () => {
    render(<KubectlPreview command="kubectl get pods web-0 --context prod" />);
    expect(screen.queryByRole("button", { name: "Copy kubectl command" })).toBeNull();
  });

  it("fires onCopy when the copy button is clicked", () => {
    const onCopy = vi.fn();
    render(<KubectlPreview command="kubectl get pods web-0 --context prod" onCopy={onCopy} />);
    fireEvent.click(screen.getByRole("button", { name: "Copy kubectl command" }));
    expect(onCopy).toHaveBeenCalledTimes(1);
  });
});
