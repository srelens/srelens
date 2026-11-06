import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { Button } from "./Button";

const kit = readFileSync(join(__dirname, "styles/kit.css"), "utf8").replace(/\/\*[\s\S]*?\*\//g, "");

/** The declarations of the first rule whose selector list names `selector` exactly. */
function rule(selector: string): string {
  for (const [, selectors, body] of kit.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    if (selectors.split(",").some((s) => s.trim() === selector)) return body;
  }
  throw new Error(`no rule for ${selector}`);
}

/**
 * The first three are the classic component's own tests, carried over
 * unchanged: the merge rule for a component both designs have is that the API
 * and behaviour survive and only the appearance moves. (#318)
 */
describe("Button", () => {
  it("renders children and fires onClick", () => {
    const onClick = vi.fn();
    render(<Button onClick={onClick}>Save</Button>);
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it("renders each variant as a button", () => {
    const { rerender } = render(<Button>X</Button>);
    expect(screen.getByRole("button", { name: "X" })).toBeDefined();
    rerender(<Button variant="ghost">X</Button>);
    expect(screen.getByRole("button", { name: "X" })).toBeDefined();
    rerender(<Button variant="danger">X</Button>);
    expect(screen.getByRole("button", { name: "X" })).toBeDefined();
  });

  it("does not fire onClick when disabled and forwards className", () => {
    const onClick = vi.fn();
    render(
      <Button disabled className="extra" onClick={onClick}>
        X
      </Button>,
    );
    const btn = screen.getByRole("button");
    fireEvent.click(btn);
    expect(onClick).not.toHaveBeenCalled();
    expect(btn.className).toContain("extra");
  });

  it("carries the design's button classes rather than utilities", () => {
    // The kit's stylesheet owns the appearance; a component that spells its own
    // padding and colour out in utilities drifts from the rest of the system.
    const { rerender } = render(<Button variant="primary">X</Button>);
    expect(screen.getByRole("button").className).toContain("btn");
    expect(screen.getByRole("button").className).toContain("btn-accent");
    rerender(<Button variant="danger">X</Button>);
    expect(screen.getByRole("button").className).toContain("btn-danger");
    rerender(<Button variant="ghost">X</Button>);
    expect(screen.getByRole("button").className).toContain("btn-ghost");
    rerender(<Button variant="secondary">X</Button>);
    expect(screen.getByRole("button").className).toBe("btn");
  });

  it("draws a text button as the kit's borderless one, sized by its words", () => {
    // `.text-btn`, not `.btn` with overrides: a bordered surface button that
    // has its border, fill and padding taken back off is two looks fighting.
    render(<Button variant="text">Open Settings</Button>);
    expect(screen.getByRole("button", { name: "Open Settings" }).className).toBe("text-btn");
  });

  it("draws a link button as text with a dotted underline, in the colour around it", () => {
    render(<Button variant="link">Open node a</Button>);
    expect(screen.getByRole("button", { name: "Open node a" }).className).toBe("link-btn");
  });

  it("exposes the size on the element for the stylesheet to act on", () => {
    const { rerender } = render(<Button size="xs">X</Button>);
    expect(screen.getByRole("button").dataset.size).toBe("xs");
    rerender(<Button size="icon-sm">X</Button>);
    expect(screen.getByRole("button").dataset.size).toBe("icon-sm");
  });

  // jsdom attaches no stylesheet, so the look these two promise is read off
  // the sheet itself.
  it.each([".btn", ".icon-btn", ".text-btn", ".link-btn"])("dims %s when it is disabled", (cls) => {
    expect(rule(`${cls}:disabled`)).toContain("opacity: 0.5");
    expect(rule(`${cls}[aria-disabled="true"]`)).toContain("opacity: 0.5");
  });

  it("underlines a link button dotted at rest and solid under the pointer or the keyboard", () => {
    expect(rule(".link-btn")).toContain("text-decoration-style: dotted");
    expect(rule(".link-btn:hover")).toContain("text-decoration-style: solid");
    expect(rule(".link-btn:focus-visible")).toContain("text-decoration-style: solid");
  });

  it("leaves the button type alone", () => {
    // Defaulting to type="button" would stop every Button inside a form from
    // submitting it, which is a behaviour change the classic component did not
    // make. Callers that need it pass it.
    render(<Button>X</Button>);
    expect(screen.getByRole("button").getAttribute("type")).toBeNull();
  });
});
