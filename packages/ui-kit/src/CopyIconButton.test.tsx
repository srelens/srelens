import { describe, it, expect, vi, afterEach } from "vitest";
import { act, fireEvent, render, screen } from "@testing-library/react";
import { CopyIconButton } from "./CopyIconButton";

const Glyph = ({ size = 14 }: { size?: number }) => <svg data-testid="glyph" width={size} />;

/**
 * The icon-only copy control, which is the one case in this kit where the
 * outcome CANNOT be a word: there is no visible text on it to change. So the
 * split is the other way round from {@link CopyCommand} and a confirming
 * `ActionBar` action — the button keeps its name and the live region speaks —
 * and these tests are mostly about that being true in both directions. (#410,
 * #413 review)
 */
describe("CopyIconButton", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  const click = async (name: string) => {
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name }));
    });
  };

  it("keeps its name when it confirms, rather than renaming itself to the outcome", async () => {
    render(<CopyIconButton icon={Glyph} label="Copy address for web-0" onCopy={() => true} />);
    await click("Copy address for web-0");

    // Still called what it DOES. A control renamed to "Copied" changes under
    // the reader's own click, and leaves them nothing to say to activate it.
    expect(screen.getByRole("button", { name: "Copy address for web-0" })).toBeDefined();
  });

  it("says so out loud, since there is no word on it to change", async () => {
    render(<CopyIconButton icon={Glyph} label="Copy address for web-0" onCopy={() => true} />);
    await click("Copy address for web-0");

    expect(screen.getByRole("status").textContent).toBe("Copied to clipboard");
  });

  it("shows a copy that failed, in words a screen reader gets and a tooltip a sighted reader gets", async () => {
    render(<CopyIconButton icon={Glyph} label="Copy address for web-0" onCopy={() => false} />);
    await click("Copy address for web-0");

    expect(screen.getByRole("status").textContent).toBe("Could not copy to clipboard");
    // The only channel left for a sighted reader on a control with no word: a
    // check that never appears is not a message.
    expect(screen.getByRole("button", { name: "Copy address for web-0" }).title).toBe("Copy failed");
  });

  it("has nothing to announce before it is used, and nothing left after", async () => {
    vi.useFakeTimers();
    render(<CopyIconButton icon={Glyph} label="Copy address for web-0" onCopy={() => true} />);
    expect(screen.queryByRole("status")).toBeNull();

    await click("Copy address for web-0");
    expect(screen.getByRole("status")).toBeDefined();

    await act(async () => {
      vi.advanceTimersByTime(1400);
    });
    // Unmounted rather than emptied: a permanently present but silent
    // `role="status"` per copy control makes the real one ambiguous to anything
    // looking for it. See `CopyAnnounce`.
    expect(screen.queryByRole("status")).toBeNull();
  });
});
