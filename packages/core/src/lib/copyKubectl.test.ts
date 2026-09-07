import { describe, it, expect, vi, beforeEach } from "vitest";

const { notifyMock } = vi.hoisted(() => ({
  notifyMock: { success: vi.fn(), error: vi.fn(), info: vi.fn() },
}));
vi.mock("./notify", () => ({ notify: notifyMock }));

import { copyKubectlCommand } from "./copyKubectl";

beforeEach(() => {
  notifyMock.success.mockReset();
});

describe("copyKubectlCommand", () => {
  it("writes the command to the clipboard and shows a success toast", async () => {
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText: vi.fn().mockResolvedValue(undefined) },
      configurable: true,
    });
    await expect(copyKubectlCommand("kubectl get pods web-0 --context prod")).resolves.toBe(true);
    expect(navigator.clipboard.writeText).toHaveBeenCalledWith("kubectl get pods web-0 --context prod");
    expect(notifyMock.success).toHaveBeenCalledWith("Copied kubectl command");
  });

  // Reports the refusal rather than swallowing it: a caller drawing a
  // confirmation has to be able to tell a copy that happened from one that did
  // not. (#410)
  it("reports false on a denied clipboard, and raises no toast", async () => {
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText: vi.fn().mockRejectedValue(new Error("denied")) },
      configurable: true,
    });
    await expect(copyKubectlCommand("kubectl get pods web-0 --context prod")).resolves.toBe(false);
    expect(notifyMock.success).not.toHaveBeenCalled();
  });

  // The other half of "denied/unavailable", which the rejecting mock above does
  // not reach: there is no clipboard object to reject. It used to be the worse
  // of the two — `clipboard?.writeText` short-circuits to `undefined`, the
  // `await` resolves, and the function reported a success it had not had. Any
  // non-secure origin lands here. (#413 review)
  it("reports false when there is no clipboard at all, and raises no toast", async () => {
    Object.defineProperty(navigator, "clipboard", { value: undefined, configurable: true });
    await expect(copyKubectlCommand("kubectl get pods web-0 --context prod")).resolves.toBe(false);
    expect(notifyMock.success).not.toHaveBeenCalled();
  });
});
