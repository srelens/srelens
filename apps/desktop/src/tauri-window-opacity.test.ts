// @vitest-environment node
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const TAURI = join(__dirname, "../src-tauri");
const json = (file: string) => JSON.parse(readFileSync(join(TAURI, file), "utf8"));
const text = (file: string) => readFileSync(join(TAURI, file), "utf8");

// The Appearance pane's window opacity is a stylesheet over a native window,
// and none of the native half is reachable from a browser test: a window that
// is not created transparent simply shows an opaque backdrop behind the
// translucent theme, with nothing failing anywhere.
describe("the see-through window on macOS", () => {
  const base = json("tauri.conf.json");
  const mac = json("tauri.macos.conf.json");

  it("creates every configured macOS window transparent", () => {
    expect(mac.app.windows.length).toBeGreaterThan(0);
    for (const window of mac.app.windows) expect(window.transparent).toBe(true);
  });

  it("leaves the other platforms' windows opaque", () => {
    // Transparent windows off macOS depend on the compositor; the setting is
    // not offered there (`supportsWindowOpacity`), so the window stays as it was.
    for (const window of base.app.windows) expect(window.transparent).toBeUndefined();
  });

  it("changes nothing else about the window", () => {
    // Tauri merges a platform file as a JSON Merge Patch, which replaces an
    // array whole — so the macOS file has to restate every window in full, and
    // a setting added to one file alone would silently not apply on macOS.
    const opaque = mac.app.windows.map(({ transparent: _transparent, ...rest }: Record<string, unknown>) => rest);
    expect(opaque).toEqual(base.app.windows);
  });

  it("turns on the private API a transparent macOS window needs, in both places", () => {
    // tauri-build refuses to build when these two disagree.
    expect(base.app.macOSPrivateApi).toBe(true);
    expect(text("Cargo.toml")).toMatch(/^tauri = \{[^}]*"macos-private-api"[^}]*\}/m);
  });

  it("lets the page ask for the blur behind the window", () => {
    // A command of the app's own, so it needs registering and no permission.
    expect(text("src/lib.rs")).toMatch(/window_blur::set_window_blur,/);
    // The window API's effects are not what blurs it, so nothing grants them.
    expect(json("capabilities/default.json").permissions).not.toContain("core:window:allow-set-effects");
  });

  it("opens wearing the theme's own canvas, light or dark, until the stylesheet lands", () => {
    // A transparent window with an unpainted page is a hole in the desktop.
    // The coat is two literals in index.html, because nothing else has loaded
    // yet; this is what stops them drifting from the tokens they stand in for.
    const html = readFileSync(join(__dirname, "../index.html"), "utf8");
    const tokens = readFileSync(join(__dirname, "../../../packages/ui-kit/src/styles/tokens.css"), "utf8");
    const canvas = (block: string) => block.match(/--canvas:\s*(#[0-9a-fA-F]{6})/)![1];
    const light = canvas(tokens.match(/\n:root \{([^}]*)\}/)![1]);
    const dark = canvas(tokens.match(/\n\[data-theme="dark"\] \{([^}]*)\}/)![1]);
    const style = html.match(/<style>([\s\S]*?)<\/style>/)![1];
    expect(style).toMatch(new RegExp(`html\\s*\\{\\s*background:\\s*${light};`));
    expect(style).toMatch(
      new RegExp(`@media \\(prefers-color-scheme: dark\\)\\s*\\{\\s*html\\s*\\{\\s*background:\\s*${dark};`),
    );
    // And the see-through themes take it off again, on the root itself.
    for (const theme of ["dark", "midnight", "glass"]) {
      const rule = [...tokens.matchAll(/([^{}]+)\{([^{}]*)\}/g)].find(([, selectors]) =>
        selectors.includes(`[data-theme="${theme}"][data-opacity]`),
      );
      expect(rule?.[2], theme).toMatch(/\n\s*background:\s*transparent;/);
    }
  });

  it("builds the windows opened at runtime the same way", () => {
    // `open_context_window` and the recreated `main` are built in Rust and
    // never read the config; both go through one helper so neither can be
    // left opaque.
    expect(text("src/window.rs")).toMatch(/see_through\(\s*WebviewWindowBuilder::new/);
    expect(text("src/deep_link.rs")).toMatch(/see_through\(\s*WebviewWindowBuilder::new/);
  });
});
