import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useRef, useState, type ReactNode } from "react";
import { ConfirmDialog, FilterBar, Popover, Tooltip } from "@srelens/ui-kit";
import { useEscapeToClose } from "./escapeToClose";

/**
 * A screen with a list and a panel beside it, as a resource list is: the
 * panel is open while `open` is true, and Escape is asked to close it.
 */
function Host({
  open = true,
  onClose,
  hidden = false,
  panel,
  children,
}: {
  open?: boolean;
  onClose: () => void;
  hidden?: boolean;
  panel?: ReactNode;
  children?: ReactNode;
}) {
  const screenRef = useRef<HTMLDivElement>(null);
  useEscapeToClose(open, onClose, screenRef);
  return (
    <div hidden={hidden} data-testid="tab">
      <div ref={screenRef}>
        <button type="button">row</button>
        <input aria-label="filter" />
        {open && (
          <div data-testid="panel">
            <button type="button">in the panel</button>
            {panel}
          </div>
        )}
      </div>
      {children}
    </div>
  );
}

const esc = (target: Element | Document = document.body, init: KeyboardEventInit = {}) =>
  fireEvent.keyDown(target, { key: "Escape", ...init });

afterEach(cleanup);

/**
 * With the app's real floating layers, which are Radix's. Radix takes Escape
 * in the capture phase, closes its top layer and marks the key handled — for
 * a tooltip exactly as for a dialog — so a stand-in element that never
 * handles the key proves nothing about how the two share one press.
 */
describe("useEscapeToClose, beside the real layers", () => {
  const user = () => userEvent.setup();

  it("closes a tooltip and the panel on one press: a tooltip is not a claim on the key", async () => {
    const onClose = vi.fn();
    render(
      <Host onClose={onClose}>
        <Tooltip label="scheduled on node-a">
          <button type="button">cell</button>
        </Tooltip>
      </Host>,
    );
    // Focus shows a tooltip at once, as resting the pointer on a cell does.
    screen.getByRole("button", { name: "cell" }).focus();
    await screen.findByRole("tooltip");

    await user().keyboard("{Escape}");

    expect(onClose).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(screen.queryByRole("tooltip")).toBeNull());
  });

  it("closes only a popover on the first press, and the panel on the second", async () => {
    const onClose = vi.fn();
    render(
      <Host onClose={onClose}>
        <Popover trigger="Columns" label="Columns">
          <button type="button">Name</button>
        </Popover>
      </Host>,
    );
    const u = user();
    await u.click(screen.getByRole("button", { name: "Columns" }));
    await screen.findByRole("dialog", { name: "Columns" });

    await u.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Columns" })).toBeNull());
    expect(onClose).not.toHaveBeenCalled();

    await u.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("closes only a dialog on the first press, and the panel on the second", async () => {
    const onClose = vi.fn();
    function WithDialog() {
      const [asking, setAsking] = useState(true);
      return (
        <Host onClose={onClose}>
          {asking && (
            <ConfirmDialog
              title="Delete web-0?"
              message="This cannot be undone."
              confirmLabel="Delete"
              onConfirm={() => setAsking(false)}
              onCancel={() => setAsking(false)}
            />
          )}
        </Host>
      );
    }
    render(<WithDialog />);
    await screen.findByRole("dialog", { name: "Delete web-0?" });
    const u = user();

    await u.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Delete web-0?" })).toBeNull());
    expect(onClose).not.toHaveBeenCalled();

    await u.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("leaves Escape to the real filter while it has text, and takes it once the filter is empty", async () => {
    const onClose = vi.fn();
    function WithFilter() {
      const [value, setValue] = useState("");
      return (
        <Host onClose={onClose}>
          <FilterBar value={value} onValueChange={setValue} label="Filter pods" />
        </Host>
      );
    }
    render(<WithFilter />);
    const u = user();
    const filter = screen.getByRole("searchbox", { name: "Filter pods" }) as HTMLInputElement;
    await u.type(filter, "web");

    await u.keyboard("{Escape}");
    expect(filter.value).toBe("");
    expect(onClose).not.toHaveBeenCalled();

    // Empty now: the kit leaves the key to whatever the list is inside.
    await u.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("does not let a tooltip excuse a field that takes the key", async () => {
    // Both at once: a cell's tooltip is up while the reader types in a field.
    // Radix dismisses the tooltip and marks the key handled, which is exactly
    // what the tooltip exception forgives — so the field has to be what holds
    // the panel open, and it is read before anything acts.
    const onClose = vi.fn();
    render(
      <Host onClose={onClose}>
        <div data-radix-popper-content-wrapper="">
          <span role="tooltip">node-a</span>
        </div>
      </Host>,
    );
    // What Radix does for its top layer: capture phase, on the document.
    const radix = (event: KeyboardEvent) => {
      if (event.key === "Escape") event.preventDefault();
    };
    document.addEventListener("keydown", radix, true);
    try {
      // A plain text field: it claims nothing and stops nothing itself.
      screen.getByLabelText("filter").focus();
      await user().keyboard("{Escape}");
      expect(onClose).not.toHaveBeenCalled();
      // Without the field, the same press with the same tooltip does close.
      screen.getByRole("button", { name: "row" }).focus();
      await user().keyboard("{Escape}");
      expect(onClose).toHaveBeenCalledTimes(1);
    } finally {
      document.removeEventListener("keydown", radix, true);
    }
  });
});

describe("useEscapeToClose", () => {
  /**
   * #883: a row is clicked, the panel opens, and focus is still on the row —
   * so the Escape that follows was a key pressed outside the panel, and did
   * nothing.
   */
  it("closes on Escape with focus on the row that opened the panel", () => {
    const onClose = vi.fn();
    render(<Host onClose={onClose} />);
    screen.getByRole("button", { name: "row" }).focus();
    esc(document.activeElement!);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("closes on Escape with focus nowhere in particular", () => {
    const onClose = vi.fn();
    render(<Host onClose={onClose} />);
    esc();
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("does nothing while there is nothing open to close", () => {
    const onClose = vi.fn();
    render(<Host open={false} onClose={onClose} />);
    esc();
    expect(onClose).not.toHaveBeenCalled();
  });

  it("answers Escape alone, not a chord with it, and no other key", () => {
    const onClose = vi.fn();
    render(<Host onClose={onClose} />);
    for (const chord of [{ metaKey: true }, { ctrlKey: true }, { altKey: true }, { shiftKey: true }]) esc(document.body, chord);
    fireEvent.keyDown(document.body, { key: "Enter" });
    expect(onClose).not.toHaveBeenCalled();
  });

  it("leaves Escape to a field: the filter box, a text area, an editor", () => {
    const onClose = vi.fn();
    render(
      <Host
        onClose={onClose}
        panel={
          <>
            <textarea aria-label="notes" />
            <div contentEditable suppressContentEditableWarning data-testid="editor" tabIndex={0} />
          </>
        }
      />,
    );
    for (const field of [screen.getByLabelText("filter"), screen.getByLabelText("notes"), screen.getByTestId("editor")]) {
      // jsdom does not derive `isContentEditable` from the attribute.
      if (field.hasAttribute("contenteditable")) Object.defineProperty(field, "isContentEditable", { value: true });
      field.focus();
      esc(field);
    }
    expect(onClose).not.toHaveBeenCalled();
  });

  it("leaves alone a keypress something else has already handled", () => {
    // The panel's own handler, when focus is inside it: it closes the panel
    // and says so, and a second close on the same press would be one too many.
    const onClose = vi.fn();
    const inner = vi.fn((event: React.KeyboardEvent) => event.preventDefault());
    render(<Host onClose={onClose} panel={<div onKeyDown={inner}><button type="button">handled</button></div>} />);
    const button = screen.getByRole("button", { name: "handled" });
    button.focus();
    esc(button);
    expect(inner).toHaveBeenCalledTimes(1);
    expect(onClose).not.toHaveBeenCalled();
  });

  it.each([
    ["a dialog", <div role="dialog" data-state="open" key="l" />],
    ["a confirmation", <div role="alertdialog" data-state="open" key="l" />],
    ["a menu", <div role="menu" data-state="open" key="l" />],
    ["a listbox", <div role="listbox" data-state="open" key="l" />],
    ["a popover", <div data-radix-popper-content-wrapper="" key="l" />],
  ])("leaves Escape to %s open on top: one layer per press", (_name, layer) => {
    const onClose = vi.fn();
    const view = render(<Host onClose={onClose}>{layer}</Host>);
    esc();
    expect(onClose).not.toHaveBeenCalled();
    // The layer closes on that press; the next one reaches the panel.
    view.rerender(<Host onClose={onClose} />);
    esc();
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("is not held back by a tooltip, which the reader did not open and Escape is not for", () => {
    // The pointer rests on the row just clicked, and a cell's tooltip is up.
    const onClose = vi.fn();
    render(
      <Host onClose={onClose}>
        <div data-radix-popper-content-wrapper="">
          <div data-state="delayed-open">
            node-a<span role="tooltip">node-a</span>
          </div>
        </div>
      </Host>,
    );
    esc();
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("is not held back by a layer that has closed, or one in a tab the reader cannot see", () => {
    const onClose = vi.fn();
    render(
      <>
        <Host onClose={onClose}>
          <div role="menu" data-state="closed" />
        </Host>
        <div hidden>
          <div role="dialog" data-state="open" />
        </div>
      </>,
    );
    esc();
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  /**
   * Every tab of the window stays mounted, hidden. A panel open in one the
   * reader is not looking at must not close on a key pressed in another.
   */
  it("does not close a panel in a tab that is hidden", () => {
    const background = vi.fn();
    const foreground = vi.fn();
    render(
      <>
        <Host onClose={background} hidden />
        <Host onClose={foreground} />
      </>,
    );
    esc();
    expect(background).not.toHaveBeenCalled();
    expect(foreground).toHaveBeenCalledTimes(1);
  });

  it("calls the close it was last given, without re-listening for every new one", () => {
    const add = vi.spyOn(document, "addEventListener");
    const first = vi.fn();
    const second = vi.fn();
    const view = render(<Host onClose={first} />);
    const listening = add.mock.calls.filter(([type]) => type === "keydown").length;
    view.rerender(<Host onClose={second} />);
    expect(add.mock.calls.filter(([type]) => type === "keydown").length).toBe(listening);
    esc();
    expect(first).not.toHaveBeenCalled();
    expect(second).toHaveBeenCalledTimes(1);
    add.mockRestore();
  });

  it("stops listening when the panel closes, and when the screen goes", () => {
    const onClose = vi.fn();
    const view = render(<Host onClose={onClose} />);
    view.rerender(<Host open={false} onClose={onClose} />);
    esc();
    view.rerender(<Host onClose={onClose} />);
    view.unmount();
    esc();
    expect(onClose).not.toHaveBeenCalled();
  });
});
