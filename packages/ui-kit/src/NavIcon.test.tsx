import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { NavIcon } from "./NavIcon";

// Forwards props, as lucide's icons do — an icon that swallowed them would let
// the assertions below pass without the component having done anything.
function Dot(props: { size?: number; className?: string; "aria-hidden"?: boolean | "true" | "false" }) {
  return <svg data-testid="icon" {...props} />;
}

describe("NavIcon", () => {
  it("renders the icon it is given", () => {
    render(<NavIcon icon={Dot} />);
    expect(screen.getByTestId("icon")).toBeDefined();
  });

  it("hides the icon from assistive technology", () => {
    // Every nav row has its label beside the icon; announcing both reads the
    // destination twice.
    render(<NavIcon icon={Dot} />);
    expect(screen.getByTestId("icon").getAttribute("aria-hidden")).toBe("true");
  });

  it("does not shrink when its row runs out of room", () => {
    // The icon sits in a flex row next to a truncating label. Without this the
    // icon is what gives way, and rows collapse to squashed glyphs.
    render(<NavIcon icon={Dot} />);
    expect(screen.getByTestId("icon").getAttribute("class")).toContain("shrink-0");
  });

  it("forwards className alongside its own", () => {
    render(<NavIcon icon={Dot} className="extra" />);
    const cls = screen.getByTestId("icon").getAttribute("class") ?? "";
    expect(cls).toContain("extra");
    expect(cls).toContain("shrink-0");
  });
});
