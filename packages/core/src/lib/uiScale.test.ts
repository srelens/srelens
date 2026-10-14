import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const transportMocks = vi.hoisted(() => ({ setWebviewZoom: vi.fn(async () => {}) }));
vi.mock("../transport/transport", () => transportMocks);

import { settingsStorage } from "./settingsStorage";

import {
  UI_SCALE,
  applyUiScale,
  clampUiScale,
  getUiScale,
  setUiScale,
  stepUiScale,
  uiScaleShortcut,
} from "./uiScale";

beforeEach(() => transportMocks.setWebviewZoom.mockClear());
afterEach(() => localStorage.clear());

describe("clampUiScale", () => {
  it("clamps to the supported range and rounds", () => {
    expect(clampUiScale(79)).toBe(UI_SCALE.MIN);
    expect(clampUiScale(151)).toBe(UI_SCALE.MAX);
    expect(clampUiScale(112.4)).toBe(112);
  });

  it("falls back to the default for junk", () => {
    expect(clampUiScale("huge")).toBe(UI_SCALE.DEFAULT);
    expect(clampUiScale(NaN)).toBe(UI_SCALE.DEFAULT);
    expect(clampUiScale(undefined)).toBe(UI_SCALE.DEFAULT);
  });
});

describe("persistence", () => {
  it("round-trips through backend settings, clamped", () => {
    expect(setUiScale(120)).toBe(120);
    expect(getUiScale()).toBe(120);
    expect(setUiScale(9000)).toBe(UI_SCALE.MAX);
    expect(getUiScale()).toBe(UI_SCALE.MAX);
  });

  it("preserves classic stored percentages without conversion", () => {
    settingsStorage.setItem("srelens.uiScale", "110");
    expect(getUiScale()).toBe(110);
  });

  it("keeps each design's saved scale independent", () => {
    setUiScale(120);
    expect(getUiScale("next")).toBe(100);
    setUiScale(140, "next");
    expect(getUiScale()).toBe(120);
    expect(getUiScale("next")).toBe(140);
    setUiScale(90);
    expect(getUiScale("next")).toBe(140);
  });

  it("stores displayed percentages for the new design without compounding", () => {
    setUiScale(100, "next");
    expect(settingsStorage.getItem("srelens.next.uiScale")).toBe("100");
    setUiScale(getUiScale("next"), "next");
    expect(getUiScale("next")).toBe(100);
  });

  it("defaults when unset or corrupted", () => {
    expect(getUiScale()).toBe(UI_SCALE.DEFAULT);
    localStorage.setItem("srelens.uiScale", "not json{");
    expect(getUiScale()).toBe(UI_SCALE.DEFAULT);
  });
});

describe("applyUiScale", () => {
  it("zooms the webview by the clamped percentage as a factor", () => {
    applyUiScale(120);
    expect(transportMocks.setWebviewZoom).toHaveBeenCalledWith(1.2);
    applyUiScale(100);
    expect(transportMocks.setWebviewZoom).toHaveBeenLastCalledWith(1);
    // Out-of-range input is clamped before it reaches the webview.
    applyUiScale(400);
    expect(transportMocks.setWebviewZoom).toHaveBeenLastCalledWith(1.5);
  });

  it("applies the larger baseline only when the new design requests it", () => {
    applyUiScale(100, "next");
    expect(transportMocks.setWebviewZoom).toHaveBeenLastCalledWith(1.1);
    applyUiScale(120, "next");
    expect(transportMocks.setWebviewZoom).toHaveBeenLastCalledWith(1.32);
    applyUiScale(100);
    expect(transportMocks.setWebviewZoom).toHaveBeenLastCalledWith(1);
  });

  it("swallows a zoom rejection so a keystroke never throws", () => {
    transportMocks.setWebviewZoom.mockRejectedValueOnce(new Error("not permitted"));
    expect(() => applyUiScale(110)).not.toThrow();
  });
});

describe("uiScaleShortcut", () => {
  const key = (key: string, mods: Partial<{ metaKey: boolean; ctrlKey: boolean; altKey: boolean }> = {}) => ({
    key,
    metaKey: false,
    ctrlKey: false,
    altKey: false,
    ...mods,
  });

  it("maps the browser-zoom vocabulary with either modifier", () => {
    expect(uiScaleShortcut(key("+", { metaKey: true }))).toBe("in");
    expect(uiScaleShortcut(key("=", { ctrlKey: true }))).toBe("in");
    expect(uiScaleShortcut(key("-", { metaKey: true }))).toBe("out");
    expect(uiScaleShortcut(key("_", { ctrlKey: true }))).toBe("out");
    expect(uiScaleShortcut(key("0", { metaKey: true }))).toBe("reset");
  });

  it("ignores unmodified keys and Alt combos", () => {
    expect(uiScaleShortcut(key("+"))).toBeNull();
    expect(uiScaleShortcut(key("=", { metaKey: true, altKey: true }))).toBeNull();
    expect(uiScaleShortcut(key("k", { metaKey: true }))).toBeNull();
  });
});

describe("stepUiScale", () => {
  it("steps by 10 within bounds and resets to 100", () => {
    expect(stepUiScale(100, "in")).toBe(110);
    expect(stepUiScale(UI_SCALE.MAX, "in")).toBe(UI_SCALE.MAX);
    expect(stepUiScale(UI_SCALE.MIN, "out")).toBe(UI_SCALE.MIN);
    expect(stepUiScale(130, "reset")).toBe(UI_SCALE.DEFAULT);
  });
});
