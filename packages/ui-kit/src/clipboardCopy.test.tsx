import { afterEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen } from "@testing-library/react";
import { ClipboardCopyStatus, useClipboardCopy } from "./clipboardCopy";

function stubClipboard(writeText: (text: string) => Promise<void>) {
  Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
}

function Harness({ text = "kubectl get pods" }: { text?: string }) {
  const copy = useClipboardCopy();
  const status = copy.statusFor("subject");
  return (
    <>
      <button type="button" onClick={() => void copy.write("subject", text)}>
        {status === "copied" ? "Copied" : status === "failed" ? "Copy failed" : "Copy"}
      </button>
      <ClipboardCopyStatus feedback={copy.feedback} />
    </>
  );
}

afterEach(() => {
  vi.useRealTimers();
});

describe("useClipboardCopy", () => {
  it("copies, announces politely, and reverts after the shared 1.4 second window", async () => {
    const writeText = vi.fn<(text: string) => Promise<void>>().mockResolvedValue(undefined);
    stubClipboard(writeText);
    vi.useFakeTimers();
    render(<Harness />);

    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Copy" })));

    expect(writeText).toHaveBeenCalledWith("kubectl get pods");
    expect(screen.getByRole("button", { name: "Copied" })).toBeDefined();
    const announcement = screen.getByRole("status");
    expect(announcement.getAttribute("aria-live")).toBe("polite");
    expect(announcement.textContent).toBe("Copied to clipboard");

    act(() => vi.advanceTimersByTime(1_400));
    expect(screen.getByRole("button", { name: "Copy" })).toBeDefined();
  });

  it("restarts the window on a rapid second copy instead of stacking timers", async () => {
    stubClipboard(vi.fn<(text: string) => Promise<void>>().mockResolvedValue(undefined));
    vi.useFakeTimers();
    render(<Harness />);

    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Copy" })));
    act(() => vi.advanceTimersByTime(1_000));
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Copied" })));
    act(() => vi.advanceTimersByTime(1_000));
    expect(screen.getByRole("button", { name: "Copied" })).toBeDefined();
    expect(vi.getTimerCount()).toBe(1);

    act(() => vi.advanceTimersByTime(400));
    expect(screen.getByRole("button", { name: "Copy" })).toBeDefined();
  });

  it("does not flicker back to Copy while a rapid second write is still pending", async () => {
    let resolveSecond: (() => void) | undefined;
    const writeText = vi
      .fn<(text: string) => Promise<void>>()
      .mockResolvedValueOnce(undefined)
      .mockImplementationOnce(
        () =>
          new Promise<void>((resolve) => {
            resolveSecond = resolve;
          }),
      );
    stubClipboard(writeText);
    vi.useFakeTimers();
    render(<Harness />);

    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Copy" })));
    act(() => vi.advanceTimersByTime(1_000));
    act(() => fireEvent.click(screen.getByRole("button", { name: "Copied" })));
    act(() => vi.advanceTimersByTime(1_000));
    expect(screen.getByRole("button", { name: "Copied" })).toBeDefined();
    expect(vi.getTimerCount()).toBe(0);

    await act(async () => resolveSecond?.());
    act(() => vi.advanceTimersByTime(1_399));
    expect(screen.getByRole("button", { name: "Copied" })).toBeDefined();
    act(() => vi.advanceTimersByTime(1));
    expect(screen.getByRole("button", { name: "Copy" })).toBeDefined();
  });

  it("reports a rejected or unavailable clipboard without ever claiming success", async () => {
    stubClipboard(
      vi.fn<(text: string) => Promise<void>>().mockRejectedValue(new Error("permission denied")),
    );
    render(<Harness />);

    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Copy" })));

    expect(screen.queryByRole("button", { name: "Copied" })).toBeNull();
    expect(screen.getByRole("button", { name: "Copy failed" })).toBeDefined();
    expect(screen.getByRole("status").textContent).toBe("Could not copy to clipboard");
  });

  it("reports the same failure when the browser exposes no Clipboard API", async () => {
    Object.defineProperty(navigator, "clipboard", { value: undefined, configurable: true });
    render(<Harness />);

    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Copy" })));

    expect(screen.queryByRole("button", { name: "Copied" })).toBeNull();
    expect(screen.getByRole("button", { name: "Copy failed" })).toBeDefined();
    expect(screen.getByRole("status").textContent).toBe("Could not copy to clipboard");
  });

  it("does not let an older, slower attempt overwrite the latest result", async () => {
    let rejectFirst: ((reason?: unknown) => void) | undefined;
    const writeText = vi
      .fn<(text: string) => Promise<void>>()
      .mockImplementationOnce(
        () =>
          new Promise<void>((_resolve, reject) => {
            rejectFirst = reject;
          }),
      )
      .mockResolvedValueOnce(undefined);
    stubClipboard(writeText);
    render(<Harness />);

    fireEvent.click(screen.getByRole("button", { name: "Copy" }));
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Copy" })));
    expect(screen.getByRole("button", { name: "Copied" })).toBeDefined();

    await act(async () => rejectFirst?.(new Error("late denial")));
    expect(screen.getByRole("button", { name: "Copied" })).toBeDefined();
    expect(screen.getByRole("status").textContent).toBe("Copied to clipboard");
  });

  it("clears the pending revert when its surface unmounts", async () => {
    stubClipboard(vi.fn<(text: string) => Promise<void>>().mockResolvedValue(undefined));
    vi.useFakeTimers();
    const view = render(<Harness />);
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Copy" })));
    expect(vi.getTimerCount()).toBe(1);

    view.unmount();
    expect(vi.getTimerCount()).toBe(0);
  });
});
