import { describe, it, expect, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ConsoleDock } from "./ConsoleDock";

function setup(props: Partial<Parameters<typeof ConsoleDock>[0]> = {}) {
  const onOpenChange = vi.fn();
  const onValueChange = vi.fn();
  const onSubmit = vi.fn();
  const view = render(
    <ConsoleDock
      open
      onOpenChange={onOpenChange}
      value=""
      onValueChange={onValueChange}
      onSubmit={onSubmit}
      {...props}
    >
      {"children" in props ? props.children : <p>transcript</p>}
    </ConsoleDock>,
  );
  return { ...view, onOpenChange, onValueChange, onSubmit };
}

describe("ConsoleDock", () => {
  it("is a named region", () => {
    setup();
    expect(screen.getByRole("region", { name: "Console" })).toBeDefined();
  });

  it("takes the caller's name for the console", () => {
    setup({ label: "Agent console" });
    expect(screen.getByRole("region", { name: "Agent console" })).toBeDefined();
    expect(screen.getByRole("textbox", { name: "Agent console prompt" })).toBeDefined();
  });

  it("names the prompt", () => {
    // A placeholder is not a label: it disappears the moment anything is
    // typed, and the mock gives the input nothing else.
    setup();
    expect(screen.getByRole("textbox", { name: "Console prompt" })).toBeDefined();
  });

  it("keeps the prompt when collapsed", () => {
    setup({ open: false });
    expect(screen.getByRole("textbox", { name: "Console prompt" })).toBeDefined();
  });

  it("shows no output region when collapsed", () => {
    setup({ open: false });
    expect(screen.queryByRole("log")).toBeNull();
    expect(screen.queryByText("transcript")).toBeNull();
  });

  it("renders the output in a named log region", () => {
    setup();
    const log = screen.getByRole("log", { name: "Console output" });
    expect(log.textContent).toContain("transcript");
    expect(log.getAttribute("aria-live")).toBe("polite");
  });

  it("stops announcing when the body is not a transcript", () => {
    // A command palette filtered on every keystroke is not something to read
    // out; the dock's body is whatever the caller puts there.
    setup({ live: false });
    expect(screen.getByRole("log").getAttribute("aria-live")).toBe("off");
  });

  it("says so when there is nothing in the output yet", () => {
    setup({ children: null });
    expect(screen.getByText("Nothing yet")).toBeDefined();
  });

  it("takes the caller's wording for the empty output", () => {
    setup({ children: false, emptyLabel: "Ask anything to start" });
    expect(screen.getByText("Ask anything to start")).toBeDefined();
  });

  it("renders the mode, the context and the status line", () => {
    setup({ mode: "Command", context: "prod-eu / checkout-api", status: "3 exchanges" });
    expect(screen.getByText("Command")).toBeDefined();
    expect(screen.getByText("prod-eu / checkout-api")).toBeDefined();
    expect(screen.getByText("3 exchanges")).toBeDefined();
  });

  it("gives every button it owns an explicit type", () => {
    // A bare button inside a form submits it. The kit's Button deliberately
    // does not default `type`, so each component sets its own.
    const { container } = setup({ onClear: () => {} });
    const buttons = [...container.querySelectorAll("button")];
    expect(buttons.length).toBeGreaterThan(0);
    expect(buttons.every((b) => b.getAttribute("type") === "button")).toBe(true);
  });
});

describe("ConsoleDock collapsing", () => {
  it("offers one toggle, named for what it will do", async () => {
    const { onOpenChange } = setup({ open: false });
    const toggle = screen.getByRole("button", { name: "Expand console" });
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    await userEvent.click(toggle);
    expect(onOpenChange).toHaveBeenCalledWith(true);
  });

  it("collapses from the same control", async () => {
    const { onOpenChange } = setup();
    const toggle = screen.getByRole("button", { name: "Collapse console" });
    expect(toggle.getAttribute("aria-expanded")).toBe("true");
    await userEvent.click(toggle);
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it("points the toggle at the output it opens, and only while there is one", () => {
    const { rerender, onOpenChange, onValueChange, onSubmit } = setup({ open: false });
    expect(screen.getByRole("button", { name: "Expand console" }).getAttribute("aria-controls")).toBeNull();
    rerender(
      <ConsoleDock
        open
        onOpenChange={onOpenChange}
        value=""
        onValueChange={onValueChange}
        onSubmit={onSubmit}
      >
        <p>transcript</p>
      </ConsoleDock>,
    );
    const controls = screen.getByRole("button", { name: "Collapse console" }).getAttribute("aria-controls");
    expect(controls).toBeTruthy();
    expect(screen.getByRole("log").id).toBe(controls);
  });

  it("expands when the prompt takes focus", async () => {
    const { onOpenChange } = setup({ open: false });
    await userEvent.click(screen.getByRole("textbox", { name: "Console prompt" }));
    expect(onOpenChange).toHaveBeenCalledWith(true);
  });

  it("collapses on Escape from the prompt", async () => {
    const { onOpenChange } = setup();
    const input = screen.getByRole("textbox", { name: "Console prompt" });
    input.focus();
    onOpenChange.mockClear();
    await userEvent.keyboard("{Escape}");
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it("leaves Escape alone when it is already collapsed", async () => {
    // Otherwise the console eats the key that something behind it wanted.
    const { onOpenChange } = setup({ open: false });
    const input = screen.getByRole("textbox", { name: "Console prompt" });
    input.focus();
    onOpenChange.mockClear();
    await userEvent.keyboard("{Escape}");
    expect(onOpenChange).not.toHaveBeenCalledWith(false);
  });

  it("moves focus to the prompt when it opens", () => {
    const { rerender, onOpenChange, onValueChange, onSubmit } = setup({ open: false });
    rerender(
      <ConsoleDock
        open
        onOpenChange={onOpenChange}
        value=""
        onValueChange={onValueChange}
        onSubmit={onSubmit}
      >
        <p>transcript</p>
      </ConsoleDock>,
    );
    expect(document.activeElement).toBe(screen.getByRole("textbox", { name: "Console prompt" }));
  });

  it("does not grab focus from whatever is on screen when it mounts open", () => {
    render(<button type="button">elsewhere</button>);
    const other = screen.getByRole("button", { name: "elsewhere" });
    other.focus();
    setup();
    expect(document.activeElement).toBe(other);
  });
});

/**
 * The dock prints the accelerator and does not bind it. Binding a window-level
 * key is the app's to do: a component cannot know what else the window has
 * bound, and two docks on one screen answered the same keystroke twice. The
 * hint is still the component's, because what the key is has to be said next to
 * the prompt it opens. (#320)
 */
describe("ConsoleDock shortcut", () => {
  it("focuses the prompt when the caller opens it", async () => {
    // The half that is still the dock's: told to open, it puts the cursor
    // where the user is about to type.
    const { rerender } = setup({ open: false });
    rerender(
      <ConsoleDock
        open
        onOpenChange={() => {}}
        value=""
        onValueChange={() => {}}
        onSubmit={() => {}}
      />,
    );
    expect(document.activeElement).toBe(screen.getByRole("textbox", { name: "Console prompt" }));
  });

  it("shows the accelerator beside the prompt", () => {
    setup({ shortcutHint: "Ctrl K" });
    expect(screen.getByText("Ctrl K")).toBeDefined();
  });

  it("hides the hint while it is working, where the spinner goes", () => {
    setup({ busy: true, shortcutHint: "Ctrl K" });
    expect(screen.queryByText("Ctrl K")).toBeNull();
  });
});

describe("ConsoleDock submitting", () => {
  it("reports what was typed", async () => {
    const { onValueChange } = setup();
    await userEvent.type(screen.getByRole("textbox", { name: "Console prompt" }), "w");
    expect(onValueChange).toHaveBeenCalledWith("w");
  });

  it("submits the trimmed query on Enter", async () => {
    const { onSubmit } = setup({ value: "  why is checkout down?  " });
    screen.getByRole("textbox", { name: "Console prompt" }).focus();
    await userEvent.keyboard("{Enter}");
    expect(onSubmit).toHaveBeenCalledWith("why is checkout down?");
  });

  it("leaves the prompt alone after submitting", async () => {
    // Controlled: the caller decides whether a submitted query is cleared or
    // kept for correction. The mock cleared it from inside its own state.
    const { onValueChange, onSubmit } = setup({ value: "why" });
    screen.getByRole("textbox", { name: "Console prompt" }).focus();
    await userEvent.keyboard("{Enter}");
    expect(onSubmit).toHaveBeenCalledTimes(1);
    expect(onValueChange).not.toHaveBeenCalled();
  });

  it("ignores an empty query", async () => {
    const { onSubmit } = setup({ value: "   " });
    screen.getByRole("textbox", { name: "Console prompt" }).focus();
    await userEvent.keyboard("{Enter}");
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("submits from the send button", async () => {
    const { onSubmit } = setup({ value: "why" });
    await userEvent.click(screen.getByRole("button", { name: "Send" }));
    expect(onSubmit).toHaveBeenCalledWith("why");
  });

  it("disables send with nothing to send", () => {
    setup({ value: "   " });
    expect(screen.getByRole("button", { name: "Send" }).hasAttribute("disabled")).toBe(true);
  });

  it("says it is working, and takes the send control away while it is", () => {
    setup({ busy: true, value: "why" });
    expect(screen.getByRole("status", { name: "Working" })).toBeDefined();
    expect(screen.queryByRole("button", { name: "Send" })).toBeNull();
  });

  it("does not queue a second query while it is working", async () => {
    const { onSubmit } = setup({ busy: true, value: "why" });
    screen.getByRole("textbox", { name: "Console prompt" }).focus();
    await userEvent.keyboard("{Enter}");
    expect(onSubmit).not.toHaveBeenCalled();
  });
});

describe("ConsoleDock clearing", () => {
  it("offers Clear only when the caller can clear", () => {
    setup();
    expect(screen.queryByRole("button", { name: "Clear console" })).toBeNull();
  });

  it("clears through the caller", async () => {
    const onClear = vi.fn();
    setup({ onClear });
    await userEvent.click(screen.getByRole("button", { name: "Clear console" }));
    expect(onClear).toHaveBeenCalledTimes(1);
  });

  it("hides Clear when there is no output to clear", () => {
    setup({ onClear: () => {}, open: false });
    expect(screen.queryByRole("button", { name: "Clear console" })).toBeNull();
  });

  it("forwards className onto the dock", () => {
    const { container } = setup({ className: "extra" });
    expect(container.querySelector(".console-dock.extra")).not.toBeNull();
  });
  it("installs no accelerator of its own", () => {
    // A window-level shortcut is the app's to own, not a component's. Two docks
    // on one screen both answered the same ⌘K, and a kit component that binds a
    // global key fights whatever else the app has bound — the same reasoning
    // that kept Inspector's ⌘⏎ out. The hint is still printed beside the
    // prompt; the caller wires the key to `onOpenChange`. (#320)
    const onOpenChange = vi.fn();
    render(
      <ConsoleDock
        open={false}
        onOpenChange={onOpenChange}
        value=""
        onValueChange={() => {}}
        onSubmit={() => {}}
        shortcutHint="⌘K"
      />,
    );
    fireEvent.keyDown(window, { key: "k", metaKey: true });
    fireEvent.keyDown(window, { key: "k", ctrlKey: true });
    expect(onOpenChange).not.toHaveBeenCalled();
  });
});
