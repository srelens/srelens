import { expect, it } from "vitest";
import { act, render, screen } from "@testing-library/react";
import { notify } from "@srelens/core";
import { Toaster } from "../components/ui/sonner";
import { installToastNotifier } from "./notifier";

// The new design draws `notify` itself (#374) by replacing this sink while its
// window is up. Classic never mounts that window, so it keeps this path: real
// sonner, its own Toaster, one toast per call.
it("draws a notify toast once in classic, through its own Toaster", async () => {
  const restore = installToastNotifier();
  try {
    render(<Toaster />);
    act(() => notify.error("Scale failed", "forbidden"));
    await screen.findByText("Scale failed");
    // sonner adds toasts on a timer, so a second one lands after the first is
    // found; count once it has had the chance to.
    await act(() => new Promise((settle) => setTimeout(settle, 100)));
    expect(screen.getAllByText("Scale failed")).toHaveLength(1);
  } finally {
    restore();
  }
});
