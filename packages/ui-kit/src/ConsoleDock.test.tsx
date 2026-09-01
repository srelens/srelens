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

  it("offers the full-view control only when a caller can act on it", async () => {
    // A kit component: a dock with no fuller view has nothing to offer here,
    // so the control is absent rather than present-and-dead.
    setup();
    expect(screen.queryByRole("button", { name: /full view/i })).toBeNull();
  });

  it("calls onExpand when the full-view control is used", async () => {
    const onExpand = vi.fn();
    setup({ onExpand });
    await userEvent.click(screen.getByRole("button", { name: /full view/i }));
    expect(onExpand).toHaveBeenCalledTimes(1);
  });

  /**
   * Three defects in this header and its row shipped together, none of them
   * catchable by the 1,200 tests in this package, because all three were
   * layout and jsdom has none. What CAN be pinned is pinned here.
   */
  it("draws ONE prompt input, from the shared bar rather than a copy of it", () => {
    setup();
    // Two implementations of the same row is what the extraction was for: the
    // dock had it inline, the agent screen grew its own, and for one commit
    // both existed.
    expect(screen.getAllByRole("textbox")).toHaveLength(1);
  });

  it("keeps the collapse control inside the prompt row, not floating over it", () => {
    setup();
    const input = screen.getByRole("textbox");
    const collapse = screen.getByRole("button", { name: /collapse console/i });
    // Same row: the chevron is the bar's `lead`. Rendered as a sibling of the
    // row instead, it landed on top of the prompt text.
    expect(input.parentElement?.contains(collapse)).toBe(true);
  });

  it("sizes the header's word controls by their content, not as icon boxes", () => {
    setup({ onExpand: () => {}, onClear: () => {} });
    const full = screen.getByRole("button", { name: /full view/i });
    const clear = screen.getByRole("button", { name: /clear console/i });
    // `.icon-btn` is `width: 24px` — a box for a glyph. These hold words, and
    // text in a 24px box overflows it, so the two overlapped. jsdom computes
    // no layout, so the class IS the assertion here; there is nothing else to
    // look at.
    for (const b of [full, clear]) {
      expect(b.className).toContain("text-btn");
      expect(b.className).not.toContain("icon-btn");
    }
  });

  /**
   * The only Stop in the app. The agent screen's own composer carried one and
   * that composer is deleted — the dock is the prompt on every screen — so a
   * turn in flight would have had nothing to stop it.
   */
  it("offers no Stop when nothing is in flight", () => {
    setup({ onStop: () => {} });
    expect(screen.queryByRole("button", { name: "Stop" })).toBeNull();
  });

  it("offers Stop beside the working spinner, and calls it", async () => {
    const onStop = vi.fn();
    setup({ busy: true, onStop });
    await userEvent.click(screen.getByRole("button", { name: "Stop" }));
    expect(onStop).toHaveBeenCalledTimes(1);
  });

  it("draws no Stop for a host that has none, rather than a dead control", () => {
    setup({ busy: true });
    expect(screen.queryByRole("button", { name: "Stop" })).toBeNull();
  });

  /**
   * "Pasting image is not allowed" — it was not possible at all. Not a
   * regression from deleting `Composer`: that had no image handling either.
   */
  it("hands a pasted image to the host", async () => {
    const onPasteImages = vi.fn();
    setup({ onPasteImages });
    const box = screen.getByRole("textbox");
    const file = new File([new Uint8Array([1, 2, 3])], "shot.png", { type: "image/png" });
    // jsdom builds no clipboard for `userEvent.paste`, so the event carries its
    // own `DataTransfer` — which is what the handler actually reads.
    const data = { files: [file], items: [], types: ["Files"] };
    fireEvent.paste(box, { clipboardData: data });
    expect(onPasteImages).toHaveBeenCalledWith([file]);
  });

  it("leaves a text paste alone", () => {
    const onPasteImages = vi.fn();
    setup({ onPasteImages });
    fireEvent.paste(screen.getByRole("textbox"), { clipboardData: { files: [], types: ["text/plain"] } });
    // Only an image paste is intercepted; text must land in the input as usual.
    expect(onPasteImages).not.toHaveBeenCalled();
  });

  it("offers no attach control to a host that takes no attachments", () => {
    setup();
    expect(screen.queryByRole("button", { name: /attach an image/i })).toBeNull();
  });

  it("sends on Enter, and makes a newline on Shift-Enter", async () => {
    const onSubmit = vi.fn();
    const onValueChange = vi.fn();
    setup({ value: "why is it restarting", onSubmit, onValueChange });
    const box = screen.getByRole("textbox");

    fireEvent.keyDown(box, { key: "Enter", shiftKey: true });
    // A question about a manifest wants more than one line, so Shift-Enter
    // must not send.
    expect(onSubmit).not.toHaveBeenCalled();

    fireEvent.keyDown(box, { key: "Enter" });
    expect(onSubmit).toHaveBeenCalledTimes(1);
  });

  /**
   * "Closed view is still too big." The expanded composer is a box with a
   * two-row input and a control row under it — right when a conversation is
   * open, far too tall for a dock meant to be out of the way.
   */
  it("is one line when closed, with no context row and no footer controls", () => {
    setup({ open: false, onPickImages: () => {}, promptContext: <span>prod-eu</span> });
    // No attach control, no context chips: the collapsed strip is a prompt and
    // nothing else.
    expect(screen.queryByRole("button", { name: /attach an image/i })).toBeNull();
    expect(screen.queryByText("prod-eu")).toBeNull();
    expect(screen.getByRole("textbox")).toBeTruthy();
  });

  it("shows the composer's own rows once open", () => {
    setup({ open: true, onPickImages: () => {}, promptContext: <span>prod-eu</span> });
    expect(screen.getByRole("button", { name: /attach an image/i })).toBeTruthy();
    expect(screen.getByText("prod-eu")).toBeTruthy();
  });

  it("continues the rail's column to the bottom, rather than leaving it blank", () => {
    const { container } = setup({ insetRight: 312 });
    // Not padding and not margin — both were tried and each left something
    // wrong. A margin pulled the dock's surface in and left a hole in the
    // corner; padding filled the chrome but left the strip blank, so the
    // sidebar still stopped short. The strip is a drawn continuation of that
    // column: same surface, same left rule.
    const strip = container.querySelector('[aria-hidden="true"][style*="312px"]');
    expect(strip).not.toBeNull();
    expect(strip?.getAttribute("style")).toContain("border-left");
    const section = container.querySelector("section");
    expect(section?.getAttribute("style")).toBeNull();
  });

  it("draws no strip for a screen with no rail of its own", () => {
    const { container } = setup();
    expect(container.querySelector('[aria-hidden="true"][style*="px"]')).toBeNull();
  });

});