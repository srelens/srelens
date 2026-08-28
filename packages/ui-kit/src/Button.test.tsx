import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { Button } from "./Button";

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

  it("exposes the size on the element for the stylesheet to act on", () => {
    const { rerender } = render(<Button size="xs">X</Button>);
    expect(screen.getByRole("button").dataset.size).toBe("xs");
    rerender(<Button size="icon-sm">X</Button>);
    expect(screen.getByRole("button").dataset.size).toBe("icon-sm");
  });

  it("leaves the button type alone", () => {
    // Defaulting to type="button" would stop every Button inside a form from
    // submitting it, which is a behaviour change the classic component did not
    // make. Callers that need it pass it.
    render(<Button>X</Button>);
    expect(screen.getByRole("button").getAttribute("type")).toBeNull();
  });
});
