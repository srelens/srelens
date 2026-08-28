import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import React from "react";
import { Trash2 } from "lucide-react";
import { IconButton } from "./IconButton";

// No waits between the pointer events, so the hover completes before the
// tooltip's open delay can elapse and the "not instant" pin is deterministic.
const user = userEvent.setup({ delay: null });

describe("IconButton", () => {
  it("names the button by its label and shows the label as the tooltip on hover", async () => {
    render(<IconButton icon={Trash2} label="Delete" />);
    const button = screen.getByRole("button", { name: "Delete" });
    // The browser's own tooltip has a fixed ~1s delay (#376) and would double
    // up with ours, so the native title must be gone.
    expect(button.hasAttribute("title")).toBe(false);

    await user.hover(button);
    expect((await screen.findByRole("tooltip")).textContent).toBe("Delete");
  });

  it("waits the shared delay rather than opening on the first pointer pass", async () => {
    render(<IconButton icon={Trash2} label="Delete" />);
    await user.hover(screen.getByRole("button", { name: "Delete" }));
    expect(screen.queryByRole("tooltip")).toBeNull();
    expect((await screen.findByRole("tooltip")).textContent).toBe("Delete");
  });

  it("shows the title override in place of the label", async () => {
    render(<IconButton icon={Trash2} label="Shell" title="Shell — 2 containers" />);
    await user.hover(screen.getByRole("button", { name: "Shell" }));
    expect((await screen.findByRole("tooltip")).textContent).toBe("Shell — 2 containers");
  });

  it("keeps a disabled button's reason reachable through a wrapper trigger", async () => {
    render(<IconButton icon={Trash2} label="Delete" disabled title="You don't have permission to delete pods" />);
    const button = screen.getByRole("button", { name: "Delete" }) as HTMLButtonElement;
    expect(button.disabled).toBe(true);
    expect(button.hasAttribute("title")).toBe(false);
    // A disabled <button> receives no pointer events, so the tooltip's trigger
    // has to be an element around it, not the button itself.
    expect(button.getAttribute("data-slot")).not.toBe("tooltip-trigger");
    const trigger = button.parentElement;
    if (!trigger) throw new Error("disabled IconButton has no wrapper element");
    expect(trigger.getAttribute("data-slot")).toBe("tooltip-trigger");

    await user.hover(trigger);
    expect((await screen.findByRole("tooltip")).textContent).toBe("You don't have permission to delete pods");
  });

  it("looks like the native title bubble it replaces: light surface, no arrow", async () => {
    render(<IconButton icon={Trash2} label="Delete" />);
    await user.hover(screen.getByRole("button", { name: "Delete" }));
    await screen.findByRole("tooltip");
    const content = document.querySelector<HTMLElement>('[data-slot="tooltip-content"]');
    if (!content) throw new Error("tooltip content not rendered");
    // Only the delay changes for the reader: the app's popover surface with a
    // hairline border, not shadcn's dark pill.
    expect(content.classList.contains("bg-popover")).toBe(true);
    expect(content.classList.contains("border")).toBe(true);
    expect(content.classList.contains("bg-primary")).toBe(false);
    expect(content.classList.contains("bg-foreground")).toBe(false);
    expect(content.querySelector("svg")).toBeNull();
    expect(document.querySelector('[data-slot="tooltip-content"] [class*="rotate-45"]')).toBeNull();
  });

  it("still fires onClick through the tooltip trigger", async () => {
    const onClick = vi.fn();
    render(<IconButton icon={Trash2} label="Delete" onClick={onClick} />);
    await user.click(screen.getByRole("button", { name: "Delete" }));
    expect(onClick).toHaveBeenCalledTimes(1);
  });
});
