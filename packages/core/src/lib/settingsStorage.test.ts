import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invokeCapability = vi.hoisted(() => vi.fn());

vi.mock("../transport/transport", () => ({ invokeCapability }));

function tauriWindow(): void {
  Object.defineProperty(window, "__TAURI_INTERNALS__", {
    configurable: true,
    value: {},
  });
}

describe("desktop settings storage", () => {
  beforeEach(() => {
    vi.resetModules();
    invokeCapability.mockReset();
    localStorage.clear();
    tauriWindow();
  });

  afterEach(() => {
    Reflect.deleteProperty(window, "__TAURI_INTERNALS__");
  });

  it("imports known localStorage values once and clears them after the file write", async () => {
    localStorage.setItem("srelens.uiScale", "120");
    localStorage.setItem("freelens.contextOrder", JSON.stringify(["prod", "dev"]));
    localStorage.setItem("fl-theme", "light");
    localStorage.setItem("unrelated", "keep me");
    invokeCapability
      .mockResolvedValueOnce({
        schemaVersion: 1,
        localStorageMigrated: false,
        values: {},
      })
      .mockResolvedValueOnce({ saved: true });

    const { initializeSettingsStorage, settingsStorage } = await import("./settingsStorage");
    await initializeSettingsStorage();

    expect(invokeCapability).toHaveBeenNthCalledWith(1, "settings.get", {});
    expect(invokeCapability).toHaveBeenNthCalledWith(2, "settings.set", {
      values: {
        "srelens.uiScale": 120,
        "srelens.contextOrder": ["prod", "dev"],
        "fl-theme-v2": { name: "slate", mode: "light" },
      },
      localStorageMigrated: true,
    });
    expect(settingsStorage.getItem("srelens.uiScale")).toBe("120");
    expect(settingsStorage.getItem("srelens.contextOrder")).toBe('["prod","dev"]');
    expect(localStorage.getItem("srelens.uiScale")).toBeNull();
    expect(localStorage.getItem("freelens.contextOrder")).toBeNull();
    expect(localStorage.getItem("fl-theme")).toBeNull();
    expect(localStorage.getItem("unrelated")).toBe("keep me");
  });

  it("uses the file as source of truth after migration", async () => {
    localStorage.setItem("srelens.uiScale", "90");
    invokeCapability.mockResolvedValueOnce({
      schemaVersion: 1,
      localStorageMigrated: true,
      values: { "srelens.uiScale": 130 },
    });

    const { initializeSettingsStorage, settingsStorage } = await import("./settingsStorage");
    await initializeSettingsStorage();

    expect(settingsStorage.getItem("srelens.uiScale")).toBe("130");
    expect(invokeCapability).toHaveBeenCalledTimes(1);
  });

  it("updates the synchronous mirror immediately and writes through in order", async () => {
    invokeCapability.mockResolvedValue({
      schemaVersion: 1,
      localStorageMigrated: true,
      values: {},
    });
    const { initializeSettingsStorage, settingsStorage, flushSettingsWrites } = await import(
      "./settingsStorage"
    );
    await initializeSettingsStorage();

    settingsStorage.setItem("srelens.uiScale", "140");
    expect(settingsStorage.getItem("srelens.uiScale")).toBe("140");
    settingsStorage.removeItem("srelens.uiScale");
    expect(settingsStorage.getItem("srelens.uiScale")).toBeNull();
    await flushSettingsWrites();

    expect(invokeCapability).toHaveBeenNthCalledWith(2, "settings.set", {
      values: { "srelens.uiScale": 140 },
    });
    expect(invokeCapability).toHaveBeenNthCalledWith(3, "settings.set", {
      remove: ["srelens.uiScale"],
    });
  });

  it("retains localStorage when migration cannot be committed", async () => {
    const error = vi.spyOn(console, "error").mockImplementation(() => {});
    localStorage.setItem("srelens.uiScale", "125");
    invokeCapability
      .mockResolvedValueOnce({
        schemaVersion: 1,
        localStorageMigrated: false,
        values: {},
      })
      .mockRejectedValueOnce(new Error("disk full"));

    const { initializeSettingsStorage, settingsStorage } = await import("./settingsStorage");
    await initializeSettingsStorage();

    expect(localStorage.getItem("srelens.uiScale")).toBe("125");
    expect(settingsStorage.getItem("srelens.uiScale")).toBe("125");
    expect(error).toHaveBeenCalled();
    error.mockRestore();
  });

  it("keeps the file backend when localStorage itself is unavailable", async () => {
    // A WebView with localStorage disabled throws from getItem. That must not
    // discard the file document we just loaded — falling back to the very
    // storage that is unavailable would leave nothing able to persist.
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const getItem = vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("localStorage is disabled");
    });
    invokeCapability
      .mockResolvedValueOnce({
        schemaVersion: 1,
        localStorageMigrated: false,
        values: { "srelens.uiScale": 130 },
      })
      .mockResolvedValue(undefined);

    const { initializeSettingsStorage, settingsStorage, flushSettingsWrites } = await import(
      "./settingsStorage"
    );
    await initializeSettingsStorage();

    // Served from the file mirror, not from the throwing localStorage.
    expect(settingsStorage.getItem("srelens.uiScale")).toBe("130");

    // Crucially, migration must NOT be marked done: the scan never ran, so
    // committing the flag would retire the one-time import and strand the
    // legacy preferences forever once storage came back.
    expect(invokeCapability).not.toHaveBeenCalledWith(
      "settings.set",
      expect.objectContaining({ localStorageMigrated: true }),
    );

    // And writes still reach the file store.
    settingsStorage.setItem("srelens.defaultNamespace", '"kube-system"');
    await flushSettingsWrites();
    expect(invokeCapability).toHaveBeenCalledWith("settings.set", {
      values: { "srelens.defaultNamespace": "kube-system" },
    });

    getItem.mockRestore();
    warn.mockRestore();
  });

  it("retries migration on a later launch after a transient storage failure", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const getItem = vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("storage temporarily disabled");
    });
    invokeCapability.mockResolvedValueOnce({
      schemaVersion: 1,
      localStorageMigrated: false,
      values: {},
    });

    const first = await import("./settingsStorage");
    await first.initializeSettingsStorage();
    getItem.mockRestore();

    // Second launch: storage is back and the flag was never set, so the real
    // values are imported instead of being lost.
    vi.resetModules();
    invokeCapability.mockReset();
    localStorage.setItem("srelens.uiScale", "140");
    invokeCapability
      .mockResolvedValueOnce({ schemaVersion: 1, localStorageMigrated: false, values: {} })
      .mockResolvedValue(undefined);

    const second = await import("./settingsStorage");
    await second.initializeSettingsStorage();

    expect(invokeCapability).toHaveBeenCalledWith("settings.set", {
      values: { "srelens.uiScale": 140 },
      localStorageMigrated: true,
    });
    expect(second.settingsStorage.getItem("srelens.uiScale")).toBe("140");
    warn.mockRestore();
  });

  it("keeps the storage-shaped localStorage fallback in web mode", async () => {
    Reflect.deleteProperty(window, "__TAURI_INTERNALS__");
    vi.resetModules();
    const { initializeSettingsStorage, settingsStorage } = await import("./settingsStorage");

    await initializeSettingsStorage();
    settingsStorage.setItem("theme", "dark");
    expect(settingsStorage.getItem("theme")).toBe("dark");
    settingsStorage.removeItem("theme");
    expect(settingsStorage.getItem("theme")).toBeNull();
    expect(invokeCapability).not.toHaveBeenCalled();
  });
});
