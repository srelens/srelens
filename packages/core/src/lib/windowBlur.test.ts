import { beforeEach, describe, expect, it, vi } from "vitest";

// A plain function rather than `vi.fn()`: a mock records how a returned
// promise settles by chaining on it, and that chain re-raises a rejection as
// an unhandled one — failing the very test that is checking it gets handled.
const host = vi.hoisted(() => ({
  calls: [] as boolean[],
  answer: (): Promise<void> => Promise.resolve(),
}));
vi.mock("../transport/transport", () => ({
  setWindowBlur: (on: boolean) => {
    host.calls.push(on);
    return host.answer();
  },
}));

import { applyWindowBlur } from "./windowBlur";

beforeEach(() => {
  host.calls = [];
  host.answer = () => Promise.resolve();
});

describe("applyWindowBlur", () => {
  it("asks the host window for the blur, on and off, and says it was done", async () => {
    await expect(applyWindowBlur(true)).resolves.toBe(true);
    await expect(applyWindowBlur(false)).resolves.toBe(true);
    expect(host.calls).toEqual([true, false]);
  });

  it("says when the host refused, with the reason, instead of passing it off as done", async () => {
    // A refused blur and an applied one must not look the same to the caller:
    // it keeps a record of what the window is wearing, and a failure recorded
    // as success is never retried. A host that does not register
    // `set_window_blur`, or a window that can no longer be reached, rejects.
    const warned = vi.spyOn(console, "warn").mockImplementation(() => {});
    host.answer = () => Promise.reject(new Error("window server said no"));
    await expect(applyWindowBlur(true)).resolves.toBe(false);
    expect(warned).toHaveBeenCalledTimes(1);
    expect(warned.mock.calls[0].map(String).join(" ")).toContain("window server said no");
    warned.mockRestore();
  });
});
