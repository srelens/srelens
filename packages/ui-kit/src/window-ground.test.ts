import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const strip = (css: string) => css.replace(/\/\*[\s\S]*?\*\//g, "");
const kit = strip(readFileSync(join(__dirname, "styles/kit.css"), "utf8"));
const tokens = strip(readFileSync(join(__dirname, "styles/tokens.css"), "utf8"));

/** The declarations of the first rule whose selector list names `selector` exactly. */
function rule(css: string, selector: string): string {
  for (const [, selectors, body] of css.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    if (selectors.split(",").some((s) => s.trim() === selector)) return body;
  }
  throw new Error(`no rule for ${selector}`);
}

// The see-through window (#853) is a stylesheet contract, and jsdom attaches
// no stylesheet: a region that went back to a solid token would still render
// in every component test, and show up only as a slab in someone's window.
describe("the window's ground", () => {
  it("is what the page paints with", () => {
    // Its own rule, not the `html, body, #root` one that only sets a height.
    expect(kit.match(/\n\s*body \{([^}]*)\}/)![1]).toContain("background: var(--ground);");
  });

  it.each([
    [".titlebar", "--ground-deep"],
    [".tabstrip", "--ground-deep"],
    [".statusbar", "--ground-deep"],
    [".panes", "--ground-surface"],
    [".side-rail", "--ground-surface"],
    [".toolbar", "--ground-surface"],
    [".card", "--ground-surface"],
    [".console-dock", "--ground-surface"],
    ['.tab[data-active="true"]', "--ground-surface"],
    [".pane-head", "--ground-sunk"],
  ])("paints %s with %s, so it clears with the rest of the sheet", (selector, token) => {
    expect(rule(kit, selector)).toContain(`background: var(${token});`);
  });

  it.each([
    [".popover", "--surface-raised"],
    [".ctx-menu", "--surface-raised"],
    [".subhead-caps", "--surface-sunk"],
  ])("leaves %s solid, because it has to hide what is under it", (selector, token) => {
    expect(rule(kit, selector)).toContain(`background: var(${token});`);
  });

  it("covers scrolling rows with a sticky table head that stays solid", () => {
    expect(rule(kit, ".tbl thead th")).toContain("background: var(--ground-cover);");
    // Solid at rest, and solid when see-through: never a clear value.
    expect(rule(tokens, ":root")).toBeDefined();
    for (const [, selectors, body] of tokens.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
      const cover = body.match(/--ground-cover:\s*([^;]+);/)?.[1];
      if (cover === undefined) continue;
      expect(cover, selectors.trim()).not.toMatch(/transparent|rgba|color-mix/);
    }
  });

  it("is each solid token at rest, so a window nobody made see-through is unchanged", () => {
    const rest = [...tokens.matchAll(/([^{}]+)\{([^{}]*)\}/g)]
      .filter(([, selectors]) => selectors.trim() === ":root")
      .map(([, , body]) => body)
      .join("\n");
    for (const [ground, solid] of [
      ["--ground", "--canvas"],
      ["--ground-canvas", "--canvas"],
      ["--ground-deep", "--canvas-deep"],
      ["--ground-surface", "--surface"],
      ["--ground-sunk", "--surface-sunk"],
      ["--ground-cover", "--surface"],
    ]) {
      expect(rest).toContain(`${ground}: var(${solid});`);
    }
  });

  it("clears every region when see-through, and tints only the page", () => {
    for (const theme of ["dark", "midnight", "glass"]) {
      const body = rule(tokens, `[data-theme="${theme}"][data-opacity]`);
      expect(body, theme).toMatch(/--ground:\s*color-mix\(in srgb, [^ ]+ var\(--window-alpha, 100%\), transparent\);/);
      for (const region of ["canvas", "deep", "surface", "sunk"]) {
        expect(body, `${theme} ${region}`).toContain(`--ground-${region}: transparent;`);
      }
    }
  });

  /**
   * A dialog is a card, and a card clears with the sheet. Over a see-through
   * window the screen behind showed straight through the dialog, its labels
   * printed across the rows underneath. A layer that floats has to hide what
   * is under it.
   */
  it("keeps a dialog's card from clearing with the sheet, on every see-through theme", () => {
    for (const role of ["dialog", "alertdialog"]) {
      for (const theme of ["dark", "midnight"]) {
        expect(rule(kit, `[data-theme="${theme}"][data-opacity] .card[role="${role}"]`), `${theme} ${role}`).toContain(
          "background: var(--surface);",
        );
      }
      const glass = rule(kit, `[data-theme="glass"][data-opacity] .card[role="${role}"]`);
      // Dark glass, frosted, and nearly opaque: a form has to be readable
      // over any desktop.
      expect(glass).toMatch(/backdrop-filter:\s*blur\(/);
      expect(glass).toMatch(/-webkit-backdrop-filter:\s*blur\(/);
      const alpha = Number(glass.match(/background:\s*rgba\([^)]*,\s*([\d.]+)\)/)?.[1]);
      expect(alpha, role).toBeGreaterThanOrEqual(0.9);
      // And less clear than a popover, which is a few lines and not a form.
      const popover = Number(
        rule(kit, '[data-theme="glass"][data-opacity] .popover').match(/background:\s*rgba\([^)]*,\s*([\d.]+)\)/)?.[1],
      );
      expect(alpha).toBeGreaterThan(popover);
    }
  });

  it("leaves a card in the page clear, and a dialog solid at rest: only the floating card is recoated", () => {
    // The in-page card still paints with the ground token.
    expect(rule(kit, ".card")).toContain("background: var(--ground-surface);");
    // No rule recoats a dialog outside the see-through window, where the
    // ground token is already the solid surface.
    expect(kit).not.toMatch(/\[data-theme="[a-z]+"\] \.card\[role=/);
  });

  it("makes Glass's floating layers dark glass only while the window is see-through", () => {
    const body = rule(kit, '[data-theme="glass"][data-opacity] .popover');
    expect(body).toMatch(/backdrop-filter:\s*blur\(/);
    expect(body).toMatch(/-webkit-backdrop-filter:\s*blur\(/);
    expect(rule(kit, '[data-theme="glass"][data-opacity] .ctx-menu')).toBe(body);
    // No rule frosts them at rest, where Glass is a solid theme.
    expect(kit).not.toMatch(/\[data-theme="glass"\] \.(popover|ctx-menu)[^{]*\{[^}]*backdrop-filter/);
  });
});
