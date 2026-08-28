import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { IconButton } from "./IconButton";

// Forwards props, as lucide's icons do — an icon that swallowed them would
// make the aria-hidden assertion below pass without the component doing
// anything.
function Dot(props: { "aria-hidden"?: boolean | "true" | "false" }) {
  return <svg data-testid="icon" {...props} />;
}

describe("IconButton", () => {
  it("names itself from the label, since the icon cannot", () => {
    render(<IconButton icon={Dot} label="Delete" />);
    expect(screen.getByRole("button", { name: "Delete" })).toBeDefined();
  });

  it("uses the label as the tooltip unless one is given", () => {
    const { rerender } = render(<IconButton icon={Dot} label="Logs" />);
    expect(screen.getByRole("button").title).toBe("Logs");
    // The override exists to explain why a button is disabled.
    rerender(<IconButton icon={Dot} label="Logs" title="No container selected" disabled />);
    expect(screen.getByRole("button").title).toBe("No container selected");
  });

  it("hides the icon from assistive technology", () => {
    // The button already has a name; the icon announcing itself repeats it.
    render(<IconButton icon={Dot} label="Delete" />);
    expect(screen.getByTestId("icon").getAttribute("aria-hidden")).toBe("true");
  });

  it("fires onClick, and does not when disabled", () => {
    const onClick = vi.fn();
    const { rerender } = render(<IconButton icon={Dot} label="Go" onClick={onClick} />);
    fireEvent.click(screen.getByRole("button"));
    expect(onClick).toHaveBeenCalledTimes(1);
    rerender(<IconButton icon={Dot} label="Go" onClick={onClick} disabled />);
    fireEvent.click(screen.getByRole("button"));
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it("is a button, never a submit", () => {
    // Icon buttons sit inside forms all over the app — a delete icon that
    // submits the form it is standing in is a data-loss bug.
    render(<IconButton icon={Dot} label="Delete" />);
    expect(screen.getByRole("button").getAttribute("type")).toBe("button");
  });

  it("tints with the severity token when danger", () => {
    render(<IconButton icon={Dot} label="Delete" danger />);
    expect(screen.getByRole("button").style.color).toContain("--sev");
  });
  it("forwards the props and the ref a wrapper needs to drive it", () => {
    // Radix's `asChild` — which Tooltip, Popover and ContextMenu all use —
    // clones its child and hands it event handlers, aria attributes and a ref.
    // A component that drops them renders perfectly and then never opens: the
    // gallery found a Tooltip around an IconButton doing exactly that. (#320)
    const ref = { current: null as HTMLButtonElement | null };
    const onFocus = vi.fn();
    render(
      <IconButton
        icon={Dot}
        label="Delete"
        ref={ref}
        onFocus={onFocus}
        aria-describedby="hint-1"
        data-state="closed"
      />,
    );
    const button = screen.getByRole("button", { name: "Delete" });
    expect(ref.current).toBe(button);
    expect(button.getAttribute("aria-describedby")).toBe("hint-1");
    expect(button.dataset.state).toBe("closed");
    fireEvent.focus(button);
    expect(onFocus).toHaveBeenCalled();
  });

  it("still refuses to have its type changed out from under it", () => {
    // Spreading the rest must not reopen the submit-button hole: the type is
    // this component's promise, not a default a caller can overwrite by
    // accident. (#320)
    render(<IconButton icon={Dot} label="Delete" {...({ type: "submit" } as object)} />);
    expect(screen.getByRole("button", { name: "Delete" }).getAttribute("type")).toBe("button");
  });
});
