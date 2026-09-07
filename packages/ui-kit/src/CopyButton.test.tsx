import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { CopyButton } from "./CopyButton";

/** jsdom ships no clipboard at all, so there is nothing to spy on. */
function stubClipboard() {
  const writeText = vi.fn<(text: string) => Promise<void>>().mockResolvedValue(undefined);
  Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
  return writeText;
}

afterEach(() => {
  vi.useRealTimers();
});

describe("CopyButton", () => {
  it("puts the text on the clipboard", async () => {
    const writeText = stubClipboard();
    render(<CopyButton text="kubectl get pods" label="Copy" />);
    await userEvent.click(screen.getByRole("button", { name: /copy/i }));
    expect(writeText).toHaveBeenCalledWith("kubectl get pods");
  });

  /**
   * The icon-only form has no visible word to change and both glyphs are
   * `aria-hidden`, so a successful copy was something only a sighted reader
   * learned about. Reported in review.
   */
  it("announces an icon-only copy without renaming the button", async () => {
    stubClipboard();
    render(<CopyButton text="kubectl get pods" label="Copy the whole conversation" iconOnly />);

    // Nothing claimed before anything happened: the region is present so it is
    // being watched, and empty so it has said nothing.
    expect(screen.getByRole("status").textContent).toBe("");

    await userEvent.click(screen.getByRole("button", { name: "Copy the whole conversation" }));

    await waitFor(() => {
      expect(screen.getByRole("status").textContent).toBe("Copied");
    });
    // The button is still named for what it DOES. A control that renames itself
    // to its own outcome mid-interaction is the other half of this defect.
    expect(screen.getByRole("button", { name: "Copy the whole conversation" })).toBeTruthy();
  });

  it("keeps the announcement out of the button's own name", async () => {
    stubClipboard();
    render(<CopyButton text="x" label="Copy the whole conversation" iconOnly />);
    await userEvent.click(screen.getByRole("button", { name: "Copy the whole conversation" }));
    await waitFor(() => {
      expect(screen.getByRole("status").textContent).toBe("Copied");
    });
    // A sibling, not a child: content inside a button contributes to its
    // accessible name, so the name would have become "Copy the whole
    // conversation Copied".
    const button = screen.getByRole("button", { name: "Copy the whole conversation" });
    expect(button.contains(screen.getByRole("status"))).toBe(false);
  });

  it("adds no second announcement when the word is already visible", async () => {
    stubClipboard();
    render(<CopyButton text="x" label="Copy" />);
    await userEvent.click(screen.getByRole("button", { name: /copy/i }));
    // The visible text becomes "Copied", which a screen reader reads from the
    // name change. A live region beside it would say it twice.
    await screen.findByText("Copied");
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("says nothing when the clipboard refuses", async () => {
    const writeText = vi
      .fn<(text: string) => Promise<void>>()
      .mockRejectedValue(new Error("Document is not focused"));
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    render(<CopyButton text="x" label="Copy the whole conversation" iconOnly />);
    await userEvent.click(screen.getByRole("button", { name: "Copy the whole conversation" }));
    // "Copied" over an empty clipboard is the one outcome here that actually
    // misleads, so a refusal claims nothing.
    expect(screen.getByRole("status").textContent).toBe("");
  });
});
