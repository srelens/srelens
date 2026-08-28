import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

const { isTauriMock, isApplePlatformMock, setTitleBarStyleMock, setTitleMock, notifyErrorMock } = vi.hoisted(() => ({
  isTauriMock: vi.fn(),
  isApplePlatformMock: vi.fn(),
  setTitleBarStyleMock: vi.fn(),
  setTitleMock: vi.fn(),
  notifyErrorMock: vi.fn(),
}));
vi.mock("@srelens/core", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@srelens/core")>()),
  isTauri: () => isTauriMock(),
  isApplePlatform: (platform?: string) => isApplePlatformMock(platform),
  notify: { error: notifyErrorMock, success: vi.fn(), info: vi.fn() },
}));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ setDecorations: vi.fn(), setTitleBarStyle: setTitleBarStyleMock, setTitle: setTitleMock }),
}));

import { DESIGN_KEY, switchDesign } from "./design";

const reload = vi.fn();
let originalLocation: Location;

beforeEach(() => {
  localStorage.clear();
  reload.mockClear();
  setTitleBarStyleMock.mockReset().mockResolvedValue(undefined);
  setTitleMock.mockReset().mockResolvedValue(undefined);
  isApplePlatformMock.mockReset().mockReturnValue(true);
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

  it("leaves the titlebar alone switching to next — its boot dresses the window", async () => {
    // The overlay lands in applyNextDesignChrome after the reload, not here:
    // one writer per direction, and boot is where "which design am I" has
    // already been read.
    await switchDesign("next");
    expect(setTitleBarStyleMock).not.toHaveBeenCalled();
    expect(reload).toHaveBeenCalledTimes(1);
  });

  it("hands the system titlebar back when leaving the new design on Apple", async () => {
    // Classic renders under system decorations; an overlay left behind there
    // would double the chrome — and the native title cleared for the overlay
    // has to come back, since classic draws no name of its own. Same failure
    // policy as any cosmetic step: attempted, never allowed to block.
    await switchDesign("classic");
    expect(setTitleBarStyleMock).toHaveBeenCalledWith("visible");
    expect(setTitleMock).toHaveBeenCalledWith("srelens");
    expect(reload).toHaveBeenCalledTimes(1);
  });

  it("does not touch the titlebar off Apple, whose chrome classic never drew over", async () => {
    isApplePlatformMock.mockReturnValue(false);
    await switchDesign("classic");
    expect(setTitleBarStyleMock).not.toHaveBeenCalled();
    expect(reload).toHaveBeenCalledTimes(1);
  });

  it("still reloads when resetting the titlebar rejects", async () => {
    // A build without the capability granted throws here; wrong chrome is a
    // blemish, a switch that silently undoes itself is a broken setting.
    setTitleBarStyleMock.mockRejectedValue(new Error("permission not granted"));
    const result = await switchDesign("classic");
    expect(result.ok).toBe(true);
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

  it("reloads on web, where there is no window chrome to set", async () => {
    isTauriMock.mockReturnValue(false);
    await switchDesign("classic");
    expect(setTitleBarStyleMock).not.toHaveBeenCalled();
    expect(reload).toHaveBeenCalledTimes(1);
  });
});
