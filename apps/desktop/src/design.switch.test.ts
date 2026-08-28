import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

const { isTauriMock, setDecorationsMock, notifyErrorMock } = vi.hoisted(() => ({
  isTauriMock: vi.fn(),
  setDecorationsMock: vi.fn(),
  notifyErrorMock: vi.fn(),
}));
vi.mock("@srelens/core", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@srelens/core")>()),
  isTauri: isTauriMock,
  notify: { error: notifyErrorMock, success: vi.fn(), info: vi.fn() },
}));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ setDecorations: setDecorationsMock }),
}));

import { DESIGN_KEY, switchDesign } from "./design";

const reload = vi.fn();
let originalLocation: Location;

beforeEach(() => {
  localStorage.clear();
  reload.mockClear();
  setDecorationsMock.mockReset().mockResolvedValue(undefined);
  notifyErrorMock.mockClear();
  isTauriMock.mockReturnValue(true);
  originalLocation = window.location;
  Object.defineProperty(window, "location", {
    configurable: true,
    value: { ...originalLocation, reload },
  });
});
afterEach(() => {
  Object.defineProperty(window, "location", { configurable: true, value: originalLocation });
});

describe("switchDesign", () => {
  it("saves the choice and reloads", async () => {
    expect((await switchDesign("next")).ok).toBe(true);
    expect(localStorage.getItem(DESIGN_KEY)).toBe("next");
    expect(reload).toHaveBeenCalledTimes(1);
  });

  it("leaves the system chrome alone while the new design has none of its own", async () => {
    // NextApp is a heading, a paragraph and a button. Dropping the decorations
    // now would leave a frameless window with no drag region and no window
    // controls — unmovable, unminimisable, closable only by quitting. This
    // flips when the design's own titlebar lands. (#314 review)
    await switchDesign("next");
    expect(setDecorationsMock).not.toHaveBeenCalled();
    expect(reload).toHaveBeenCalledTimes(1);
  });

  it("does not reload when the choice could not be saved", async () => {
    // Reloading would come back on the old design, since the next boot reads
    // no preference — a switch that silently undoes itself. (#314 review)
    const original = Storage.prototype.setItem;
    Storage.prototype.setItem = () => {
      throw new Error("denied");
    };
    try {
      const result = await switchDesign("next");
      expect(reload).not.toHaveBeenCalled();
      // Reported back rather than toasted: the toast host lives in the classic
      // tree, so a failure while leaving the new design would be invisible.
      expect(result.ok).toBe(false);
      expect(result.ok === false && result.reason).toBeTruthy();
    } finally {
      Storage.prototype.setItem = original;
    }
  });

  it("reloads on web, where there are no decorations to set", async () => {
    isTauriMock.mockReturnValue(false);
    await switchDesign("next");
    expect(setDecorationsMock).not.toHaveBeenCalled();
    expect(reload).toHaveBeenCalledTimes(1);
  });
});
