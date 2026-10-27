import { expect, it, vi } from "vitest";
import { act, render, screen } from "@testing-library/react";
import { notify } from "@srelens/core";
import { Toaster } from "../components/ui/sonner";
import { installToastNotifier } from "./notifier";

// The new design draws `notify` itself (#374) by replacing this sink while its
// window is up. Classic never mounts that window, so it keeps this path: real
// sonner, its own Toaster, one toast per call.
it("draws a notify toast once in classic, through its own Toaster", () => {
  const restore = installToastNotifier();
  vi.useFakeTimers();
  try {
    render(<Toaster />);
    act(() => notify.error("Scale failed", "forbidden"));
    // sonner adds toasts on a timer. Run every timer it set, well past its
    // own delivery, so a second toast has landed before the count, however
    // slow the machine is.
    act(() => vi.advanceTimersByTime(1_000));
    expect(screen.getAllByText("Scale failed")).toHaveLength(1);
  } finally {
    vi.useRealTimers();
    restore();
  }
});
