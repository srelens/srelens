import { describe, it, expect, vi, beforeEach, beforeAll, afterAll } from "vitest";

const { isTauriMock, isApplePlatformMock, setTitleBarStyleMock, setTitleMock } = vi.hoisted(() => ({
  isTauriMock: vi.fn(),
  isApplePlatformMock: vi.fn(),
  setTitleBarStyleMock: vi.fn(),
  setTitleMock: vi.fn(),
}));
vi.mock("@srelens/core", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@srelens/core")>()),
  isTauri: () => isTauriMock(),
  isApplePlatform: (platform?: string) => isApplePlatformMock(platform),
}));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ setTitleBarStyle: setTitleBarStyleMock, setTitle: setTitleMock }),
}));

import { applyNextDesignChrome, drawsOwnChrome } from "./design";

beforeEach(() => {
  isTauriMock.mockReset().mockReturnValue(true);
  // Explicit strings go through the real matcher — what those tests pin IS
  // the pass-through. An absent one means "the caller had no answer", which
  // here stands in for jsdom's empty navigator.platform saying Mac.
  isApplePlatformMock.mockReset().mockImplementation((platform?: string) =>
    platform === undefined ? true : /mac|iphone|ipad/i.test(platform),
  );
  setTitleBarStyleMock.mockReset().mockResolvedValue(undefined);
  setTitleMock.mockReset().mockResolvedValue(undefined);
});

describe("drawsOwnChrome", () => {
  it("is true on Apple platforms", () => {
    expect(drawsOwnChrome("MacIntel")).toBe(true);
  });

  it("is false on Windows and Linux, whose traffic lights are not macOS-shaped", () => {
    expect(drawsOwnChrome("Win32")).toBe(false);
    expect(drawsOwnChrome("Linux x86_64")).toBe(false);
  });

  it("asks core when no platform is given, so the runtime decides", () => {
    drawsOwnChrome();
    expect(isApplePlatformMock).toHaveBeenCalled();
  });
});

describe("applyNextDesignChrome", () => {
  it("sets the overlay titlebar exactly when Tauri and Apple", async () => {
    await applyNextDesignChrome();
    expect(setTitleBarStyleMock).toHaveBeenCalledWith("overlay");
  });

  it("clears the native title, which macOS keeps painting over the overlay", async () => {
    // Found on a real machine: with an overlay style the system still draws
    // the window's own title — "srelens" from tauri.conf.json — on top of the
    // webview, landing square on the workspace switcher.
    await applyNextDesignChrome();
    expect(setTitleMock).toHaveBeenCalledWith("");
  });

  it("does nothing off Apple, where the mock's traffic lights would lie", async () => {
    isApplePlatformMock.mockReturnValue(false);
    await applyNextDesignChrome();
    expect(setTitleBarStyleMock).not.toHaveBeenCalled();
    expect(setTitleMock).not.toHaveBeenCalled();
  });

  it("does nothing on web, where there is no window to dress", async () => {
    isTauriMock.mockReturnValue(false);
    await applyNextDesignChrome();
    expect(setTitleBarStyleMock).not.toHaveBeenCalled();
    expect(setTitleMock).not.toHaveBeenCalled();
  });

  it("survives a rejecting window call rather than breaking boot", async () => {
    // A build without `core:window:allow-set-title-bar-style` granted throws
    // here. The overlay is cosmetic; a rejected promise escaping this would
    // have left bootstrap awaiting forever and the window blank.
    setTitleBarStyleMock.mockRejectedValue(new Error("permission not granted"));
    await expect(applyNextDesignChrome()).resolves.toBeUndefined();
  });
});
