import { beforeEach, describe, expect, it, vi } from "vitest";

const { invokeCommand, isTauri } = vi.hoisted(() => ({
  invokeCommand: vi.fn(),
  isTauri: vi.fn(),
}));
vi.mock("../transport/transport", () => ({ invokeCommand }));
vi.mock("../transport/platform", () => ({ isTauri }));

import { normalizeNativeComponentLink, openNativeComponentLink } from "./nativeComponentLink";

const opened = vi.fn();
beforeEach(() => {
  vi.clearAllMocks();
  isTauri.mockReturnValue(true);
  invokeCommand.mockResolvedValue(null);
  Object.defineProperty(window, "open", { value: opened, configurable: true });
});

describe("native component links", () => {
  it("normalizes explicit HTTP(S) URLs without opening them", () => {
    expect(normalizeNativeComponentLink("HTTPS://EXAMPLE.COM:443/docs?q=a%20b#section"))
      .toBe("https://example.com/docs?q=a%20b#section");
    expect(normalizeNativeComponentLink("http://localhost:8080/"))
      .toBe("http://localhost:8080/");
    expect(opened).not.toHaveBeenCalled();
    expect(invokeCommand).not.toHaveBeenCalled();
  });

  it.each([
    "", "example.com", "/relative", "//example.com", "https:///example.com",
    "javascript:alert(1)", "data:text/html,hello", "file:///etc/passwd",
    "https%3A%2F%2Fexample.com", "https://", "https://example.com:invalid",
    "https://user:password@example.com", "https://user@example.com", "https://@example.com",
    " https://example.com", "https://example.com/hello world", "https://example.com/\npath",
    "https://example.com/\u0000", "https://example.com/\u007f", "https://example.com/\u202eabc",
    "https://example.com/\u2066abc", "https://example.com\\@evil.example/",
  ])("renders unsafe input as text and refuses navigation: %j", async (value) => {
    expect(normalizeNativeComponentLink(value)).toBeNull();
    for (const desktop of [true, false]) {
      isTauri.mockReturnValue(desktop);
      await expect(openNativeComponentLink(value)).rejects.toThrow(/http/i);
    }
    expect(opened).not.toHaveBeenCalled();
    expect(invokeCommand).not.toHaveBeenCalled();
  });

  it("uses the native opener for an explicit desktop click", async () => {
    await openNativeComponentLink("HTTPS://EXAMPLE.COM/docs");
    expect(invokeCommand).toHaveBeenCalledWith("open_external", { url: "https://example.com/docs" });
    expect(opened).not.toHaveBeenCalled();
  });

  it("opens web links without granting access to the originating window", async () => {
    isTauri.mockReturnValue(false);
    opened.mockReturnValue(null);
    await expect(openNativeComponentLink("https://example.com/docs")).resolves.toBeUndefined();
    expect(opened).toHaveBeenCalledWith("https://example.com/docs", "_blank", "noopener,noreferrer");
    expect(invokeCommand).not.toHaveBeenCalled();
  });

  it("reports native opener failures so the caller can offer another click", async () => {
    invokeCommand.mockRejectedValue(new Error("no default browser"));
    await expect(openNativeComponentLink("https://example.com")).rejects.toThrow("no default browser");
  });
});
