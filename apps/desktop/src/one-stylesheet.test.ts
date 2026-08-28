// @vitest-environment node
// (reads the entry module's source from disk; jsdom buys nothing here)
import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { join } from "node:path";

/**
 * The two designs cannot share a document: both import Tailwind, use different
 * dark-mode conventions (`.dark` versus `[data-theme=dark]`), and each writes
 * global `body` and `*` rules.
 *
 * The isolation rests on one property — neither design's stylesheet nor its
 * root is imported statically from the entry — so Vite keeps each in its own
 * chunk and `index.html` links no stylesheet at all.
 *
 * A static import would be a one-word change with no symptom in `pnpm dev`,
 * because dev-mode Vite injects styles differently, and a visibly broken new
 * design in the built app. So it is asserted on the source, where it is cheap.
 *
 * Established by building the real app: a statically imported AppGate pulls
 * ui/index.ts's `ui/styles.css` into the entry chunk, and that chunk is linked
 * unconditionally.
 */
const main = readFileSync(join(__dirname, "main.tsx"), "utf8");

describe("the app entry", () => {
  it("imports no stylesheet statically", () => {
    const statics = [...main.matchAll(/^import\s+["']([^"']+\.css)["']/gm)].map((m) => m[1]);
    expect(statics, `static stylesheet imports in main.tsx: ${statics.join(", ")}`).toEqual([]);
  });

  it("imports neither design's root statically", () => {
    // AppGate drags the classic stylesheet in with it; NextApp drags the new
    // one. Either as a top-level import defeats the split.
    expect(main).not.toMatch(/^import\s+AppGate\b/m);
    expect(main).not.toMatch(/^import\s+\{[^}]*\bNextApp\b[^}]*\}/m);
  });

  it("loads both designs dynamically, and only one per boot", () => {
    // Written as `import(...)` inside Promise.all rather than `await import`,
    // so each design's stylesheet and tree download together instead of one
    // after the other.
    expect(main).toMatch(/import\(["']@srelens\/ui-next\/styles["']\)/);
    expect(main).toMatch(/import\(["']@srelens\/ui-next["']\)/);
    expect(main).toMatch(/import\(["']\.\/styles\/globals\.css["']\)/);
    expect(main).toMatch(/import\(["']\.\/AppGate["']\)/);
    // The next-design branch must return, or the classic tree would render on
    // top of it and pull its stylesheet into the same document.
    const branch = main.slice(main.indexOf('loadDesign() === "next"'));
    expect(branch.slice(0, branch.indexOf('import("./styles/globals.css")'))).toMatch(/\breturn;/);
  });

  it("does not serialise a design's stylesheet behind its tree", () => {
    // Both are needed before the first paint and neither depends on the other,
    // so awaiting one before requesting the second only lengthens the blank
    // window. (#314 review)
    const serial = /await import\([^)]*\);\s*(?:const[^;]*=\s*)?await import\(/.test(main);
    expect(serial, "main.tsx awaits one import before starting the next").toBe(false);
  });
});
