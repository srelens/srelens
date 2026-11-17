import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { LoadingState } from "./LoadingState";

/** The classic component's tests, carried over. (#318) */
describe("LoadingState", () => {
  it("shows the default caption and an accessible spinner", () => {
    render(<LoadingState />);
    expect(screen.getByText("Loading")).toBeDefined();
    expect(screen.getByRole("status").getAttribute("aria-label")).toBe("Loading");
  });

  it("captions the load with a custom label", () => {
    render(<LoadingState label="Loading pods" />);
    expect(screen.getByText("Loading pods")).toBeDefined();
    expect(screen.getByRole("status").getAttribute("aria-label")).toBe("Loading pods");
  });

  it("says the same thing once, not twice", () => {
    // The caption and the spinner's label are the same words by design. If the
    // caption were also exposed to assistive technology as a status, a screen
    // reader would announce "Loading pods" twice for one placeholder.
    render(<LoadingState label="Loading pods" />);
    expect(screen.getAllByRole("status")).toHaveLength(1);
  });

  it("forwards className", () => {
    const { container } = render(<LoadingState className="extra" />);
    expect(container.querySelector(".extra")).not.toBeNull();
  });

  it("draws one line inside a block, rather than a page's worth of waiting", () => {
    // The page-sized form is 64px of padding around a 32px spinner, under a
    // heading that may hold one table row (#1001).
    const { container } = render(<LoadingState compact label="Loading pods" />);
    expect(container.innerHTML).not.toContain("py-16");
    expect(container.innerHTML).not.toContain("size-8");
    expect(screen.getByText("Loading pods")).toBeDefined();
    expect(screen.getAllByRole("status")).toHaveLength(1);
    expect(screen.getByRole("status").getAttribute("aria-label")).toBe("Loading pods");
  });

  it("forwards className in the compact form too", () => {
    const { container } = render(<LoadingState compact className="extra" />);
    expect(container.querySelector(".extra")).not.toBeNull();
  });
});
