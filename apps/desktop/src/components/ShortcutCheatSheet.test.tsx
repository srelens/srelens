import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import React from "react";
import { ShortcutCheatSheet } from "./ShortcutCheatSheet";
import { visibleShortcuts } from "@srelens/core";

describe("ShortcutCheatSheet", () => {
  it("lists every shortcut the registry declares", () => {
    // The point of the registry: the sheet cannot fall behind the bindings.
    render(<ShortcutCheatSheet open onOpenChange={vi.fn()} desktop apple={false} />);
    for (const shortcut of visibleShortcuts(true, false)) {
      expect(screen.getByText(shortcut.description)).toBeDefined();
    }
  });

  it("labels the modifier for the platform", () => {
    const { rerender } = render(
      <ShortcutCheatSheet open onOpenChange={vi.fn()} desktop apple={false} />,
    );
    expect(screen.getByText("Ctrl+K")).toBeDefined();

    rerender(<ShortcutCheatSheet open onOpenChange={vi.fn()} desktop apple />);
    expect(screen.getByText("⌘K")).toBeDefined();
  });

  it("omits the keys the browser owns in a web build", () => {
    render(<ShortcutCheatSheet open onOpenChange={vi.fn()} desktop={false} apple={false} />);
    expect(screen.queryByText("Make the interface larger")).toBeNull();
    expect(screen.getByText("Open the command palette")).toBeDefined();
  });

  it("renders nothing while closed", () => {
    render(<ShortcutCheatSheet open={false} onOpenChange={vi.fn()} desktop apple={false} />);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("gives each group a usable accessible name", () => {
    // An IDREF list is whitespace-separated: "shortcuts-Command palette" reads
    // as two references, neither of which exists, and the group is unnamed.
    render(<ShortcutCheatSheet open onOpenChange={vi.fn()} desktop apple={false} />);
    const named = screen.getByRole("region", { name: "Command palette" });
    const id = named.getAttribute("aria-labelledby")!;
    expect(id).not.toContain(" ");
    expect(document.getElementById(id)).not.toBeNull();
  });

  it("omits Cmd-W off macOS, where nothing implements it", () => {
    render(<ShortcutCheatSheet open onOpenChange={vi.fn()} desktop apple={false} />);
    expect(screen.queryByText(/Close the current tab/)).toBeNull();
  });

  it("is a labelled dialog", () => {
    // The overlay is the one screen that exists purely for keyboard users, so
    // it has to announce itself to a screen reader as more than a box.
    render(<ShortcutCheatSheet open onOpenChange={vi.fn()} desktop apple={false} />);
    expect(screen.getByRole("dialog", { name: "Keyboard shortcuts" })).toBeDefined();
  });
});
