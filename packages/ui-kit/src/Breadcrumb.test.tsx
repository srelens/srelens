import { describe, it, expect } from "vitest";
import { render, screen, within } from "@testing-library/react";
import { Breadcrumb } from "./Breadcrumb";

const TRAIL = ["prod-eu", "kube-system", "coredns-7f6cbbb7b8"];

describe("Breadcrumb", () => {
  it("is a navigation landmark with a name of its own", () => {
    render(<Breadcrumb parts={TRAIL} />);
    expect(screen.getByRole("navigation", { name: "Breadcrumb" })).toBeDefined();
  });

  it("renders the trail as an ordered list, because a trail has an order", () => {
    // A row of sibling spans says nothing about sequence or depth; a list says
    // how many steps there are and which one you are on. (#320)
    const { container } = render(<Breadcrumb parts={TRAIL} />);
    expect(container.querySelector("ol")).not.toBeNull();
    expect(screen.getAllByRole("listitem")).toHaveLength(3);
  });

  it("renders the parts in the order it was given them", () => {
    render(<Breadcrumb parts={TRAIL} />);
    const items = screen.getAllByRole("listitem").map((li) => li.textContent ?? "");
    expect(items[0]).toContain("prod-eu");
    expect(items[1]).toContain("kube-system");
    expect(items[2]).toContain("coredns-7f6cbbb7b8");
  });

  it("marks the last part as the current page", () => {
    // The mock only styled it differently, which says nothing to a screen
    // reader about where in the trail you are. (#320)
    render(<Breadcrumb parts={TRAIL} />);
    expect(screen.getByText("coredns-7f6cbbb7b8").getAttribute("aria-current")).toBe("page");
  });

  it("marks only the last part", () => {
    render(<Breadcrumb parts={TRAIL} />);
    expect(screen.getByText("prod-eu").getAttribute("aria-current")).toBeNull();
    expect(screen.getByText("kube-system").getAttribute("aria-current")).toBeNull();
  });

  it("marks a single-part trail as the current page too", () => {
    render(<Breadcrumb parts={["prod-eu"]} />);
    expect(screen.getByText("prod-eu").getAttribute("aria-current")).toBe("page");
  });

  it("hides the separators from assistive technology", () => {
    // "prod-eu slash kube-system slash coredns" is punctuation read as content.
    const { container } = render(<Breadcrumb parts={TRAIL} />);
    expect(container.querySelectorAll('[aria-hidden="true"]')).toHaveLength(2);
    const first = screen.getAllByRole("listitem")[0];
    expect(within(first).queryByText("/")).toBeNull();
  });

  it("does not put a separator before the first part", () => {
    const { container } = render(<Breadcrumb parts={["prod-eu"]} />);
    expect(container.querySelectorAll('[aria-hidden="true"]')).toHaveLength(0);
  });

  it("renders nothing at all for an empty trail", () => {
    // An empty landmark is still a landmark: it turns up in the list a screen
    // reader offers and leads nowhere.
    const { container } = render(<Breadcrumb parts={[]} />);
    expect(container.firstChild).toBeNull();
  });

  it("survives a repeated name in the trail", () => {
    render(<Breadcrumb parts={["default", "pods", "default"]} />);
    expect(screen.getAllByRole("listitem")).toHaveLength(3);
    expect(screen.getAllByText("default")).toHaveLength(2);
  });

  it("forwards className onto the nav without replacing its own", () => {
    render(<Breadcrumb parts={TRAIL} className="extra" />);
    const nav = screen.getByRole("navigation", { name: "Breadcrumb" });
    expect(nav.classList.contains("extra")).toBe(true);
    expect(nav.className.trim()).not.toBe("extra");
  });
});
