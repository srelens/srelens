import { afterEach, describe, expect, it } from "vitest";
import { isTauri } from "./platform";

describe("isTauri", () => {
  afterEach(() => {
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  });
  it("is false in a plain browser", () => {
    expect(isTauri()).toBe(false);
  });
  it("is true when the Tauri host marker is present", () => {
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    expect(isTauri()).toBe(true);
  });
});
