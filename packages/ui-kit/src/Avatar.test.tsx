import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { Avatar } from "./Avatar";

describe("Avatar", () => {
  it("shows the initials", () => {
    render(<Avatar name="Devesh Kumar" />);
    expect(screen.getByText("DK")).toBeDefined();
  });

  it("is named by the person, not by their initials", () => {
    // "DK" is not a name. The circle is a picture of a person and says so, so
    // the full name is what gets announced. (#320)
    render(<Avatar name="Devesh Kumar" />);
    expect(screen.getByRole("img", { name: "Devesh Kumar" })).toBeDefined();
  });

  it("does not lean on a title attribute for its name", () => {
    // A `title` on a non-interactive element is announced inconsistently and
    // never on touch; alongside an aria-label it becomes the accessible
    // description, so the name is read twice. (#320)
    const { container } = render(<Avatar name="Devesh Kumar" />);
    expect(container.querySelector("[title]")).toBeNull();
  });

  it("takes at most two initials", () => {
    render(<Avatar name="Devesh Kumar Sharma" />);
    expect(screen.getByText("DK")).toBeDefined();
  });

  it("takes a one-word name", () => {
    render(<Avatar name="devesh" />);
    expect(screen.getByRole("img", { name: "devesh" }).textContent).toBe("D");
  });

  it("survives a double space in the name", () => {
    // `name.split(" ").map(p => p[0])` yields undefined for the empty segment
    // between two spaces, and React prints the string "undefined". (#320)
    const { container } = render(<Avatar name="Devesh  Kumar" />);
    expect(container.textContent).not.toContain("undefined");
    expect(screen.getByText("DK")).toBeDefined();
  });

  it("survives padding and tabs around the name", () => {
    // Braced rather than a bare JSX attribute: a JSX string attribute is
    // literal text, so `name="a\tb"` is a backslash and a t.
    const { container } = render(<Avatar name={"  Devesh\tKumar "} />);
    expect(container.textContent).not.toContain("undefined");
    expect(screen.getByText("DK")).toBeDefined();
  });

  it("keeps an astral first character whole", () => {
    // `p[0]` is a UTF-16 code unit, not a character: it splits a surrogate pair
    // and renders half of one.
    render(<Avatar name={"\u{1D4A5}ane Doe"} />);
    expect(screen.getByText("\u{1D4A5}D")).toBeDefined();
  });

  it("shows an initial from a lowercase name in the case initials are written in", () => {
    render(<Avatar name="devesh kumar" />);
    expect(screen.getByText("DK")).toBeDefined();
  });

  it("claims nothing to assistive technology when there is no name to give", () => {
    // An ARIA img with no accessible name is a defect, not a nameless picture;
    // with nothing to say, the circle is decoration.
    const { container } = render(<Avatar name="   " />);
    expect(screen.queryByRole("img")).toBeNull();
    expect(container.firstElementChild?.getAttribute("aria-hidden")).toBe("true");
  });

  it("takes its colour from the tone", () => {
    const { container } = render(<Avatar name="Devesh Kumar" tone="ok" />);
    const root = container.firstElementChild as HTMLElement;
    expect(root.style.color).toContain("--ok");
    expect(root.getAttribute("data-tone")).toBe("ok");
  });

  it("forwards className onto the root without replacing its own", () => {
    const { container } = render(<Avatar name="Devesh Kumar" className="extra" />);
    const root = container.firstElementChild as HTMLElement;
    expect(root.classList.contains("extra")).toBe(true);
    expect(root.className.trim()).not.toBe("extra");
  });
});
