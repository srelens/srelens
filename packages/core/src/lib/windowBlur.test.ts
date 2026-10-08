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
    // A build without `core:window:allow-set-effects`, or a platform with no
    // such effect, rejects. Appearance is applied during boot, and a rejection
    // escaping from here would be an unhandled one in the middle of it.
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
