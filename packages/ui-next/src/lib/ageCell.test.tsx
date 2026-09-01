import { describe, it, expect, vi, afterEach } from "vitest";
import { act, render, screen, cleanup } from "@testing-library/react";
import { AgeCell } from "./ageCell";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

const CREATED = "2026-09-01T12:00:00Z";
const AT = (ms: number) => new Date(CREATED).getTime() + ms;

describe("AgeCell (#405)", () => {
  it("advances as time passes instead of freezing at the value it first rendered", () => {
    vi.useFakeTimers();
    vi.setSystemTime(AT(0));
    render(<AgeCell created={CREATED} />);
    // The #405 repro: an object created while its list is open.
    expect(screen.getByText("0s")).toBeDefined();

    act(() => void vi.advanceTimersByTime(30_000));
    expect(screen.getByText("30s")).toBeDefined();

    act(() => void vi.advanceTimersByTime(150_000));
    expect(screen.getByText("3m")).toBeDefined();

    // The bug this guards: the cell used to render a string the backend had
    // rendered once, so all three assertions above read "0s".
    expect(screen.queryByText("0s")).toBeNull();
  });

  it("falls back to the backend's age string for a kind that carries no timestamp", () => {
    vi.useFakeTimers();
    vi.setSystemTime(AT(0));
    render(<AgeCell age="119d" />);
    expect(screen.getByText("119d")).toBeDefined();
  });

  it("renders nothing numeric when it has neither, rather than a wrong age", () => {
    render(<AgeCell />);
    expect(screen.getByText("—")).toBeDefined();
  });

  it("stops its interval once the last cell unmounts", () => {
    vi.useFakeTimers();
    const clear = vi.spyOn(globalThis, "clearInterval");
    const a = render(<AgeCell created={CREATED} />);
    const b = render(<AgeCell created={CREATED} />);
    a.unmount();
    expect(clear).not.toHaveBeenCalled(); // one cell left, clock still needed
    b.unmount();
    expect(clear).toHaveBeenCalled();
  });
});
