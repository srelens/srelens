import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { notify, setNotifier, type Notifier } from "./notify";

/**
 * `notify` no longer knows how anything is rendered — it dispatches to whatever
 * sink the UI installed. So these assert the dispatch and the default silence;
 * the shape of the toast itself is asserted in the app, next to the sonner
 * implementation that builds it.
 */
function spySink() {
  return {
    success: vi.fn(),
    error: vi.fn(),
    info: vi.fn(),
    updateAvailable: vi.fn(),
    clusterSignIn: vi.fn(),
  } satisfies Notifier;
}

let sink = spySink();
let restore = () => {};

beforeEach(() => {
  sink = spySink();
  restore = setNotifier(sink);
});
afterEach(() => restore());

describe("notify", () => {
  it("passes a message and its description through to the sink", () => {
    notify.success("Scaled web to 3", "took 2s");
    expect(sink.success).toHaveBeenCalledWith("Scaled web to 3", "took 2s");

    notify.error("Scale failed", "forbidden");
    expect(sink.error).toHaveBeenCalledWith("Scale failed", "forbidden");

    notify.info("Nothing to do");
    expect(sink.info).toHaveBeenCalledWith("Nothing to do", undefined);
  });

  it("passes the action callbacks through untouched", () => {
    const onView = vi.fn();
    notify.updateAvailable("0.3.0", onView);
    expect(sink.updateAvailable).toHaveBeenCalledWith("0.3.0", onView);

    const onSignIn = vi.fn();
    notify.clusterSignIn("Sign in", "prod needs auth", onSignIn);
    expect(sink.clusterSignIn).toHaveBeenCalledWith("Sign in", "prod needs auth", onSignIn);
  });

  it("is silent, not broken, when no UI has installed a sink", () => {
    // A worker or CLI has no screen. It should get nothing, not a crash.
    restore();
    expect(() => {
      notify.success("no one is listening");
      notify.error("still no one");
      notify.updateAvailable("0.3.0", () => {});
    }).not.toThrow();
    restore = setNotifier(sink);
  });

  it("restores the previous sink, so installs do not leak", () => {
    const second = spySink();
    const undo = setNotifier(second);
    notify.info("to the second");
    expect(second.info).toHaveBeenCalled();

    undo();
    notify.info("back to the first");
    expect(sink.info).toHaveBeenCalledWith("back to the first", undefined);
  });
});
