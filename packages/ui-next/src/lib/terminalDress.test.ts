import { afterEach, describe, expect, it } from "vitest";
import { cssPixels, terminalDress, TERMINAL_FONT_SIZE } from "./terminalDress";

afterEach(() => {
  document.documentElement.removeAttribute("style");
});

describe("cssPixels", () => {
  it("reads pixels and root-relative sizes", () => {
    expect(cssPixels("12px", 16)).toBe(12);
    expect(cssPixels("0.75rem", 16)).toBe(12);
    expect(cssPixels(" 0.8125rem ", 16)).toBe(13);
    expect(cssPixels("0.75rem", 20)).toBe(15);
  });

  it("refuses what it cannot read as a size", () => {
    expect(cssPixels("", 16)).toBeUndefined();
    expect(cssPixels("1em", 16)).toBeUndefined();
    expect(cssPixels("var(--x)", 16)).toBeUndefined();
    expect(cssPixels("0px", 16)).toBeUndefined();
  });
});

describe("terminalDress", () => {
  it("sizes the terminal from the app's monospace token, against the root size (#942)", () => {
    const root = document.documentElement;
    root.style.setProperty("--text-label", "0.75rem");
    root.style.fontSize = "18px";
    expect(terminalDress(root).fontSize).toBe(13.5);
  });

  it("falls back to the app's default size, not xterm's, with no stylesheet", () => {
    expect(terminalDress(document.documentElement).fontSize).toBe(TERMINAL_FONT_SIZE);
  });

  it("matches the token's value in the kit's stylesheet", async () => {
    const { readFileSync } = await import("node:fs");
    const { join } = await import("node:path");
    const css = readFileSync(join(__dirname, "../../../ui-kit/src/styles/tokens.css"), "utf8");
    const value = /--text-label:\s*([^;]+);/.exec(css)?.[1];
    expect(cssPixels(value ?? "", 16)).toBe(TERMINAL_FONT_SIZE);
  });
});
