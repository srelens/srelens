import { describe, it, expect, vi, beforeEach } from "vitest";

const { toastMock } = vi.hoisted(() => {
  const fn = vi.fn() as unknown as ReturnType<typeof vi.fn> & {
    success: ReturnType<typeof vi.fn>;
    error: ReturnType<typeof vi.fn>;
  };
  fn.success = vi.fn();
  fn.error = vi.fn();
  return { toastMock: fn };
});
vi.mock("sonner", () => ({ toast: toastMock }));

import { toastNotifier } from "./notifier";

/**
 * The shape of each toast — its description, action label and duration — is a
 * UI decision, so it is asserted here rather than in `@srelens/core`, which
 * only says *what* happened. These assertions moved with the sonner call when
 * notify was inverted into a sink (#311 review).
 */
beforeEach(() => {
  toastMock.mockClear();
  toastMock.success.mockClear();
  toastMock.error.mockClear();
});

describe("toastNotifier", () => {
  it("renders success and error through sonner's typed helpers", () => {
    toastNotifier.success("Scaled web to 3", "took 2s");
    expect(toastMock.success).toHaveBeenCalledWith("Scaled web to 3", { description: "took 2s" });

    toastNotifier.error("Scale failed", "forbidden");
    expect(toastMock.error).toHaveBeenCalledWith("Scale failed", { description: "forbidden" });
  });

  it("omits the options object when there is no description", () => {
    // sonner renders an empty description slot if handed `{ description: undefined }`.
    toastNotifier.info("Nothing to do");
    expect(toastMock).toHaveBeenCalledWith("Nothing to do", undefined);
  });

  it("gives the update toast a View-update action that runs the callback", () => {
    const onView = vi.fn();
    toastNotifier.updateAvailable("0.3.0", onView);
    expect(toastMock).toHaveBeenCalledTimes(1);
    const [message, opts] = toastMock.mock.calls[0];
    expect(String(message)).toContain("Update available");
    expect(opts.description).toContain("0.3.0");
    expect(opts.action.label).toBe("View update");
    opts.action.onClick();
    expect(onView).toHaveBeenCalled();
  });

  it("gives the sign-in toast a Sign-in action and a long duration", () => {
    const onSignIn = vi.fn();
    toastNotifier.clusterSignIn("prod needs auth", "session expired", onSignIn);
    const [message, opts] = toastMock.mock.calls[0];
    expect(String(message)).toContain("prod needs auth");
    expect(opts.description).toBe("session expired");
    expect(opts.action.label).toBe("Sign in");
    // An action prompt the user may not react to immediately.
    expect(opts.duration).toBeGreaterThanOrEqual(30000);
    opts.action.onClick();
    expect(onSignIn).toHaveBeenCalled();
  });
});
