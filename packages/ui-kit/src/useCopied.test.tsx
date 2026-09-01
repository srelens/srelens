import { describe, it, expect, vi, afterEach } from "vitest";
import { act, fireEvent, render, screen } from "@testing-library/react";
import { useCopied } from "./useCopied";

afterEach(() => {
  vi.useRealTimers();
});

/** A control small enough that the test is about the hook and nothing else. */
function Probe({ copy }: { copy: () => unknown | Promise<unknown> }) {
  const { state, run } = useCopied();
  return (
    <button type="button" onClick={() => void run(copy)}>
      {state}
    </button>
  );
}

const control = () => screen.getByRole("button");
const click = async () => {
  await act(async () => {
    fireEvent.click(control());
  });
};

describe("useCopied", () => {
  it("starts idle, says copied, and takes it back", async () => {
    vi.useFakeTimers();
    render(<Probe copy={() => Promise.resolve()} />);
    expect(control().textContent).toBe("idle");

    await click();
    expect(control().textContent).toBe("copied");

    await act(async () => {
      vi.advanceTimersByTime(1400);
    });
    expect(control().textContent).toBe("idle");
  });

  it("never says copied when the copy throws", async () => {
    render(<Probe copy={() => Promise.reject(new Error("denied"))} />);
    await click();
    expect(control().textContent).toBe("failed");
  });

  // The rule that matters: a helper reporting failure by return value rather
  // than by rejecting must not be read as a success.
  it("never says copied when the copy resolves false", async () => {
    render(<Probe copy={() => Promise.resolve(false)} />);
    await click();
    expect(control().textContent).toBe("failed");
  });

  it("treats a synchronous copy that does not throw as a success", async () => {
    render(<Probe copy={() => undefined} />);
    await click();
    expect(control().textContent).toBe("copied");
  });

  // Without the sequence counter the second click sets an identical state,
  // React bails out, the effect never re-runs, and the confirmation would
  // vanish on the FIRST click's schedule.
  it("restarts the window on a rapid second copy rather than expiring early", async () => {
    vi.useFakeTimers();
    render(<Probe copy={() => Promise.resolve()} />);

    await click();
    await act(async () => {
      vi.advanceTimersByTime(1000);
    });
    expect(control().textContent).toBe("copied");

    await click();
    // 600ms further on: past the first click's 1400, well inside the second's.
    await act(async () => {
      vi.advanceTimersByTime(600);
    });
    expect(control().textContent).toBe("copied");

    await act(async () => {
      vi.advanceTimersByTime(800);
    });
    expect(control().textContent).toBe("idle");
  });

  it("clears its timer when the control goes away mid-confirmation", async () => {
    vi.useFakeTimers();
    const { unmount } = render(<Probe copy={() => Promise.resolve()} />);
    await click();

    unmount();
    // A timer left running would fire into an unmounted component here.
    await act(async () => {
      vi.advanceTimersByTime(1400);
    });
    expect(vi.getTimerCount()).toBe(0);
  });
});
