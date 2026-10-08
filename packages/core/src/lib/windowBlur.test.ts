import { beforeEach, describe, expect, it, vi } from "vitest";

const transportMocks = vi.hoisted(() => ({ setWindowBlur: vi.fn() }));
vi.mock("../transport/transport", () => transportMocks);

import { applyWindowBlur } from "./windowBlur";

beforeEach(() => transportMocks.setWindowBlur.mockReset());

describe("applyWindowBlur", () => {
  it("asks the host window for the blur, on and off", () => {
    transportMocks.setWindowBlur.mockResolvedValue(undefined);
    applyWindowBlur(true);
    applyWindowBlur(false);
    expect(transportMocks.setWindowBlur.mock.calls).toEqual([[true], [false]]);
  });

  it("never lets a refused effect reach the caller", async () => {
    // A host that does not register `set_window_blur`, or a window that can
    // no longer be reached, rejects. Appearance is applied during boot, and a
    // rejection escaping from here would be an unhandled one in the middle of it.
    const handled = vi.fn();
    transportMocks.setWindowBlur.mockReturnValue({
      catch: (onRejected: (error: unknown) => void) => {
        handled();
        onRejected(new Error("not allowed"));
      },
    });
    expect(() => applyWindowBlur(true)).not.toThrow();
    expect(handled).toHaveBeenCalledTimes(1);
  });
});
