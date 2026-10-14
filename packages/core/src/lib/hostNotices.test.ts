import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

const { onMock } = vi.hoisted(() => ({ onMock: vi.fn() }));
vi.mock("../transport/transport", () => ({ on: onMock }));

import { listenForHostNotices } from "./hostNotices";
import { setNotifier, type Notifier } from "./notify";

/** A notifier recording what it was asked to show, by kind. */
function recordingNotifier() {
  const shown: Array<[kind: string, message: string, description?: string]> = [];
  const notifier: Notifier = {
    success: (m, d) => shown.push(["success", m, d]),
    error: (m, d) => shown.push(["error", m, d]),
    info: (m, d) => shown.push(["info", m, d]),
    updateAvailable: () => {},
    clusterSignIn: () => {},
  };
  return { shown, notifier };
}

/**
 * #735: the desktop host reports what finished with no page left to hear it —
 * a helm operation whose window closed or reloaded while it ran — by
 * broadcasting a notice to every open window. Each window shows it as a toast.
 */
describe("listenForHostNotices", () => {
  let restore: () => void = () => {};
  let deliver: (payload: unknown) => void = () => {};
  const off = vi.fn();

  beforeEach(() => {
    onMock.mockReset();
    off.mockReset();
    onMock.mockImplementation((event: string, handler: (payload: unknown) => void) => {
      if (event === "host-notice") deliver = handler;
      return off;
    });
  });
  afterEach(() => restore());

  it("shows a failure as an error and anything else as information", () => {
    const { shown, notifier } = recordingNotifier();
    restore = setNotifier(notifier);
    listenForHostNotices();

    deliver({ level: "error", title: "helm upgrade web failed", detail: "helm exited with code 1" });
    deliver({ level: "info", title: "helm upgrade web finished" });

    expect(shown).toEqual([
      ["error", "helm upgrade web failed", "helm exited with code 1"],
      ["info", "helm upgrade web finished", undefined],
    ]);
  });

  it("shows nothing for a payload that is not a notice", () => {
    const { shown, notifier } = recordingNotifier();
    restore = setNotifier(notifier);
    listenForHostNotices();

    deliver(null);
    deliver("helm finished");
    deliver({ level: "error" });

    expect(shown).toEqual([]);
  });

  it("hands back the subscription's own release", () => {
    expect(listenForHostNotices()).toBe(off);
  });

  // The new design mounts no `notify` sink — a toast sent there is drawn
  // nowhere — so it shows notices itself, and they go to it instead.
  it("hands each notice to the surface it is given, and not to notify", () => {
    const { shown, notifier } = recordingNotifier();
    restore = setNotifier(notifier);
    const surfaced: unknown[] = [];
    listenForHostNotices((notice) => surfaced.push(notice));

    deliver({ level: "error", title: "helm upgrade web failed", detail: "helm exited with code 1" });
    deliver({ title: "no level" });
    deliver(null);

    expect(surfaced).toEqual([
      { level: "error", title: "helm upgrade web failed", detail: "helm exited with code 1" },
      { level: "info", title: "no level", detail: undefined },
    ]);
    expect(shown).toEqual([]);
  });
});
