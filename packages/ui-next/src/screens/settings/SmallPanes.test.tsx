import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const HERE = __dirname;

/**
 * The module `ShortcutsPane` is required to read its glyphs from, as text —
 * not trusted, the way `AppearancePane.test.tsx` reads `tokens.css` rather
 * than trusting the pane's own claims about it.
 */
const SHORTCUTS_SOURCE = readFileSync(join(HERE, "../../lib/shortcuts.ts"), "utf8");

/**
 * Every `WindowAction["type"]` the module actually declares, pulled out of
 * its own union rather than retyped by hand here — so a new binding added to
 * `WindowAction` fails this file until the pane is taught to render it,
 * instead of the pane silently omitting it forever.
 */
function declaredActionTypes(): string[] {
  const union = SHORTCUTS_SOURCE.match(/export type WindowAction =([\s\S]*?);\n\ntype Binding/);
  if (!union) throw new Error("could not find the WindowAction union in lib/shortcuts.ts");
  const types = Array.from(union[1].matchAll(/type:\s*"([a-z-]+)"/g)).map((m) => m[1]);
  // `select-tab`'s member carries a second field after its own `;`
  // (`{ type: "select-tab"; index: number }`), so a naive "stop at the first
  // semicolon" cut would silently drop console, the lock and every zoom action
  // — this asserts the fixture itself still has all eleven before trusting it.
  if (types.length !== 11) {
    throw new Error(`expected 11 window action types in the union, read ${types.length}`);
  }
  return types;
}

/**
 * The pane's `hint` is mocked through a double that defaults to the REAL
 * implementation and lets one test substitute a different chord for one
 * action type. This is what lets a test change "a binding" without touching
 * `lib/shortcuts.ts` on disk, per the brief: "a `shortcuts.ts` double, or
 * drive `hint` with a different action set."
 */
const shortcutsDouble = vi.hoisted(() => ({
  // Typed as the real `hint`, whose first parameter is a `WindowAction` and
  // not a `string`. A widened signature here broke `tsc --noEmit` while the
  // suite stayed green, because a double that accepts more than the module
  // does is not assignable to it.
  hint: vi.fn<typeof import("../../lib/shortcuts").hint>(),
}));
// The module's own signature, not a widened copy of it: `hint` takes a
// `WindowAction`, and a `(type: string, …)` annotation here does not accept it
// — which is why this line broke `tsc --noEmit` while the suite stayed green.
let realHint: typeof import("../../lib/shortcuts").hint;
vi.mock("../../lib/shortcuts", async (orig) => {
  const actual = await orig<typeof import("../../lib/shortcuts")>();
  return { ...actual, hint: shortcutsDouble.hint };
});

const tabs = vi.hoisted(() => ({ openTab: vi.fn() }));
vi.mock("../../lib/tabsStore", () => ({ openTab: tabs.openTab }));

import { AccessibilityPane, ShortcutsPane, ClustersPane } from "./SmallPanes";

beforeEach(async () => {
  vi.clearAllMocks();
  const actual = await vi.importActual<typeof import("../../lib/shortcuts")>("../../lib/shortcuts");
  realHint = actual.hint;
  shortcutsDouble.hint.mockImplementation(realHint);
});

describe("ShortcutsPane", () => {
  it("renders a binding for every window action the app knows", () => {
    render(<ShortcutsPane />);
    const rows = screen.getAllByTestId("shortcut-row");
    expect(rows.length).toBeGreaterThan(0);
    // Every rendered binding comes from `hint`, not from this file.
    for (const row of rows) expect(row.textContent).not.toMatch(/TODO|\?\?/);
  });

  /**
   * The set of rows itself must not be a private list either: it has to be
   * exactly the action types `WindowAction` declares, no more and no fewer.
   * A hardcoded row list that happened to be the same LENGTH as the real one
   * is exactly the kind of test that passes for the wrong reason — this
   * compares the actual `type` each row carries, not just a count.
   */
  it("renders exactly the window actions the module declares, not a private list", () => {
    render(<ShortcutsPane />);
    const rendered = screen
      .getAllByTestId("shortcut-row")
      .map((row) => row.getAttribute("data-action-type"))
      .filter((v): v is string => v !== null)
      .sort();
    expect(rendered).toEqual(declaredActionTypes().slice().sort());
  });

  /**
   * The pinning test. A hardcoded shortcut table would show `⌘T` for New tab
   * whether or not `lib/shortcuts.ts` still binds `Mod+T` to it — this proves
   * the row is driven by a LIVE call to `hint`, by making that call return
   * something no real binding would ever render and checking the row shows
   * exactly that, with every other row untouched.
   */
  it("follows a changed binding instead of a memorised one", () => {
    const MUTANT_GLYPH = "⌘Q-mutant";
    shortcutsDouble.hint.mockImplementation((type, apple) =>
      type === "new-tab" ? MUTANT_GLYPH : realHint(type, apple),
    );
    render(<ShortcutsPane />);
    const rows = screen.getAllByTestId("shortcut-row");
    const newTabRow = rows.find((r) => r.getAttribute("data-action-type") === "new-tab");
    const closeTabRow = rows.find((r) => r.getAttribute("data-action-type") === "close-tab");
    expect(newTabRow?.textContent).toContain(MUTANT_GLYPH);
    // Nothing else moved: this is a per-row read, not a snapshot taken once
    // and reused, and not a global find/replace over the whole pane. Checked
    // against both platform renderings of the real chord, since this test
    // does not control `isApplePlatform` and either could be what rendered.
    expect(closeTabRow?.textContent).not.toContain(MUTANT_GLYPH);
    expect(closeTabRow?.textContent).toMatch(/⌘W|Ctrl\+W/);
  });

  it("draws a label beside its chord, not one packed against the other", () => {
    render(<ShortcutsPane />);
    const row = screen.getAllByTestId("shortcut-row")[0];
    const label = row.querySelector("span");
    const keys = screen.getAllByTestId("shortcut-keys")[0];
    // The label is the flexible side (`min-w-0 flex-1`) and the chord the
    // fixed one (`shrink-0`) — the specific pairing `min-width: auto` has
    // broken eight times in this migration, and jsdom renders neither
    // failure, so this checks the class lists rather than a layout.
    expect(label?.className).toContain("min-w-0");
    expect(label?.className).toContain("flex-1");
    expect(keys.className).toContain("shrink-0");
  });
});

describe("AccessibilityPane", () => {
  it("names Reduce motion and what it stops, from §23's own hint", () => {
    render(<AccessibilityPane />);
    const note = screen.getByTestId("reduce-motion-note");
    expect(note.textContent).toMatch(/reduce motion/i);
    expect(note.textContent).toMatch(/live pulse/i);
    expect(note.textContent).toMatch(/topology graph/i);
    expect(note.textContent).toMatch(/every transition/i);
  });

  /**
   * The check the brief asks for: no reduce-motion preference exists
   * anywhere in this codebase (no `data-motion` writer, no settings key), so
   * this pane must not draw a switch that would persist nothing. A `role`
   * check rather than a text search — a switch could exist under different
   * copy and this must catch it regardless of label.
   */
  it("draws no switch for reduce motion, and says why", () => {
    render(<AccessibilityPane />);
    expect(screen.queryAllByRole("switch")).toHaveLength(0);
    const note = screen.getByTestId("reduce-motion-note");
    expect(note.textContent).toMatch(/no preference|keeps no preference/i);
    expect(note.textContent).toMatch(/system/i);
  });

  /**
   * The 7:1 in this sentence is a numeric property of `ui-kit`'s stylesheet, and
   * for most of this branch NOTHING computed it — this test pinned the number
   * while the claim was false: `--ink-faint` on `--surface-sunk` came to
   * 6.56:1 in the `contrast` theme. The token was darkened and the floor is now
   * computed over every pair the theme defines, in
   * `packages/ui-kit/src/tokens-only.test.ts` ("the high-contrast theme"). This
   * assertion is only worth keeping because that one exists; if the guard is
   * ever removed, the sentence goes with it.
   */
  it("carries §23's contrast paragraph, which ui-kit's own guard makes true", () => {
    render(<AccessibilityPane />);
    expect(screen.getByText(/raises every text pair above 7:1/i)).toBeTruthy();
    expect(screen.getByText(/dashed stroke as well as colour/i)).toBeTruthy();
  });

  /**
   * §23 says "the console announces agent replies through a live region", and
   * as of step 9 that is TRUE: the dock has a real agent behind it, and
   * `ui-kit`'s `ConsoleDock` declares the one ARIA log region for it, which
   * `shell/Console.tsx` drives through `live` — polite for the thread, off for
   * the palette and the suggestions list, whose whole body re-renders on every
   * keystroke.
   *
   * This test used to assert the OPPOSITE, because the console was a stub with
   * nothing to announce. Step 9 made the pane's sentence false and the guard
   * missed it: the guard scanned `Console.tsx` for `aria-live`, and the region
   * had moved into `ConsoleDock`, so an absence-scan could pass while the
   * console announced. An absence is the wrong thing to pin — it goes stale
   * silently the moment the feature arrives. So this pins the PRESENCE of what
   * the copy claims, in the file that actually carries it.
   */
  it("claims the console announces replies, and that claim is true", () => {
    render(<AccessibilityPane />);
    const note = screen.getByTestId("live-region-note").textContent ?? "";
    expect(note).toMatch(/live region/i);
    // The pane must not still be telling the reader the agent is absent.
    expect(note).not.toMatch(/announces nothing yet|not in the new design/i);
    expect(note).toMatch(/read as it arrives/i);
    expect(note).toMatch(/goes quiet while you type/i);
    // The source of the claim, so it cannot pass on copy alone: the region the
    // sentence promises exists, and the console drives it.
    const dock = readFileSync(join(HERE, "../../../../ui-kit/src/ConsoleDock.tsx"), "utf8");
    expect(dock).toMatch(/role="log"/);
    expect(dock).toMatch(/aria-live=\{live \? "polite" : "off"\}/);
    const console_ = readFileSync(join(HERE, "../../shell/Console.tsx"), "utf8");
    expect(console_).toMatch(/live=\{dockLive\}/);
  });

  it("carries §23's other screen-reader paragraph", () => {
    render(<AccessibilityPane />);
    expect(screen.getByText(/traps escape/i)).toBeTruthy();
  });
});

describe("ClustersPane", () => {
  it("sends a reader after cluster sources to the screen that owns them", async () => {
    const user = userEvent.setup();
    render(<ClustersPane />);
    await user.click(screen.getByRole("button", { name: /connections/i }));
    expect(tabs.openTab).toHaveBeenCalledWith("/connections");
  });

  it("does not repeat what the connections screen already does", () => {
    render(<ClustersPane />);
    expect(screen.queryByRole("table")).toBeNull();
    expect(screen.queryByRole("button", { name: /add a kubeconfig/i })).toBeNull();
  });

  it("names Connections rather than duplicating a context list", () => {
    render(<ClustersPane />);
    expect(screen.getAllByText(/connections/i).length).toBeGreaterThan(0);
    // No per-cluster rows: a list of context names is the duplication this
    // pane exists specifically not to draw.
    expect(screen.queryAllByRole("listitem")).toHaveLength(0);
  });
});
