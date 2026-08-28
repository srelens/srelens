import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import React from "react";
import { Button } from "./button";
import { TOOLTIP_DELAY_MS, TOOLTIP_SKIP_DELAY_MS } from "./tooltip";

const user = userEvent.setup({ delay: null });

// A control explains itself through a Radix tooltip, never a native `title`:
// the browser's bubble has a fixed ~1 s delay (#376) and the two would double
// up. `title` is the one prop every button already had, so the switch happens
// inside `Button` and every caller gets it without changing.
describe("Button title", () => {
  it("shows the title as a tooltip on hover and forwards no native title", async () => {
    render(<Button title="Copy as kubectl">Copy</Button>);
    const button = screen.getByRole("button", { name: "Copy" });
    expect(button.hasAttribute("title")).toBe(false);
    expect(button.getAttribute("data-slot")).toBe("button");

    await user.hover(button);
    expect((await screen.findByRole("tooltip")).textContent).toBe("Copy as kubectl");
  });

  it("renders a button without a title exactly as before: no trigger, no tooltip", async () => {
    const { container } = render(<Button>Plain</Button>);
    const button = screen.getByRole("button", { name: "Plain" });
    expect(button.parentElement).toBe(container);
    // Radix marks its trigger with data-state; a plain button has none.
    expect(button.hasAttribute("data-state")).toBe(false);
    expect(document.querySelector('[data-slot="tooltip-trigger"]')).toBeNull();
  });

  it("treats an empty title like none", () => {
    render(<Button title="">Plain</Button>);
    const button = screen.getByRole("button", { name: "Plain" });
    expect(button.hasAttribute("title")).toBe(false);
    expect(button.hasAttribute("data-state")).toBe(false);
  });

  it("keeps asChild rendering the child, and makes that child the trigger", async () => {
    render(
      <Button asChild title="Open the docs">
        <a href="#docs">Docs</a>
      </Button>,
    );
    expect(screen.queryByRole("button")).toBeNull();
    const link = screen.getByRole("link", { name: "Docs" });
    expect(link.getAttribute("data-slot")).toBe("button");
    expect(link.hasAttribute("title")).toBe(false);

    await user.hover(link);
    expect((await screen.findByRole("tooltip")).textContent).toBe("Open the docs");
  });

  it("keeps a disabled button's reason reachable through a wrapper trigger", async () => {
    render(
      <Button disabled title="You don't have permission to patch deployments in prod">
        Apply
      </Button>,
    );
    const button = screen.getByRole("button", { name: "Apply" }) as HTMLButtonElement;
    expect(button.disabled).toBe(true);
    expect(button.hasAttribute("title")).toBe(false);
    // A disabled <button> gets no pointer events, so the trigger has to be an
    // element around it.
    const trigger = button.parentElement;
    if (!trigger) throw new Error("disabled Button has no wrapper element");
    expect(trigger.getAttribute("data-slot")).toBe("tooltip-trigger");

    await user.hover(trigger);
    expect((await screen.findByRole("tooltip")).textContent).toBe(
      "You don't have permission to patch deployments in prod",
    );
  });

  it("does not wrap a disabled button that has nothing to explain", () => {
    const { container } = render(<Button disabled>Apply</Button>);
    expect(screen.getByRole("button", { name: "Apply" }).parentElement).toBe(container);
  });

  it("still fires onClick through the tooltip trigger", async () => {
    const onClick = vi.fn();
    render(
      <Button title="Refresh the list" onClick={onClick}>
        Refresh
      </Button>,
    );
    await user.click(screen.getByRole("button", { name: "Refresh" }));
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it("looks like the native title bubble it replaces: popover surface, hairline border, no arrow", async () => {
    render(<Button title="Refresh the list">Refresh</Button>);
    await user.hover(screen.getByRole("button", { name: "Refresh" }));
    await screen.findByRole("tooltip");
    const content = document.querySelector<HTMLElement>('[data-slot="tooltip-content"]');
    if (!content) throw new Error("tooltip content not rendered");
    expect(content.classList.contains("bg-popover")).toBe(true);
    expect(content.classList.contains("border")).toBe(true);
    expect(content.classList.contains("text-xs")).toBe(true);
    expect(content.classList.contains("bg-foreground")).toBe(false);
    expect(content.querySelector("svg")).toBeNull();
  });
});

describe("tooltip delay", () => {
  it("opens fast — 100 ms, not the browser's second — and sweeps siblings for 300 ms", () => {
    expect(TOOLTIP_DELAY_MS).toBe(100);
    expect(TOOLTIP_SKIP_DELAY_MS).toBe(300);
  });
});
