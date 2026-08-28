import { describe, expect, it } from "vitest";
import {
  SHORTCUTS,
  formatChord,
  formatShortcut,
  groupedShortcuts,
  isApplePlatform,
  isTypingTarget,
  matchesShortcut,
  visibleShortcuts,
} from "./shortcuts";

const press = (key: string, mods: Partial<Record<"meta" | "ctrl" | "shift" | "alt", boolean>> = {}) => ({
  key,
  metaKey: !!mods.meta,
  ctrlKey: !!mods.ctrl,
  shiftKey: !!mods.shift,
  altKey: !!mods.alt,
});

describe("the registry itself", () => {
  it("has no duplicate ids", () => {
    const ids = SHORTCUTS.map((s) => s.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("gives every shortcut a describable chord", () => {
    // A chord of nothing but modifiers can never be matched, and would render
    // as a bare "⌘" in the sheet.
    for (const shortcut of SHORTCUTS) {
      expect(shortcut.chords.length).toBeGreaterThan(0);
      for (const chord of shortcut.chords) {
        expect(chord.some((token) => !["Mod", "Shift", "Alt"].includes(token))).toBe(true);
      }
    }
  });
});

describe("matchesShortcut", () => {
  it("accepts Cmd or Ctrl for the same binding", () => {
    expect(matchesShortcut("palette", press("k", { meta: true }))).toBe(true);
    expect(matchesShortcut("palette", press("k", { ctrl: true }))).toBe(true);
  });

  it("is case-insensitive, since Shift changes the reported key", () => {
    expect(matchesShortcut("palette", press("K", { meta: true, shift: true }))).toBe(true);
  });

  it("ignores the key without its modifier", () => {
    // Otherwise typing "k" anywhere would open the palette.
    expect(matchesShortcut("palette", press("k"))).toBe(false);
  });

  it("leaves Alt combos alone", () => {
    // They type characters on several keyboard layouts.
    expect(matchesShortcut("palette", press("k", { meta: true, alt: true }))).toBe(false);
  });

  it("matches `?` even though the layout needs Shift to type it", () => {
    expect(matchesShortcut("cheatsheet", press("?", { shift: true }))).toBe(true);
    expect(matchesShortcut("cheatsheet", press("?"))).toBe(true);
  });

  it("does not treat a modified `?` as the cheat sheet", () => {
    expect(matchesShortcut("cheatsheet", press("?", { meta: true }))).toBe(false);
  });

  it("accepts either chord of a shortcut that has two", () => {
    expect(matchesShortcut("zoom-in", press("+", { meta: true }))).toBe(true);
    expect(matchesShortcut("zoom-in", press("=", { ctrl: true }))).toBe(true);
  });

  it("is false for an id that isn't registered", () => {
    expect(matchesShortcut("no-such-shortcut", press("k", { meta: true }))).toBe(false);
  });
});

describe("isTypingTarget", () => {
  it("recognises the fields a bare-letter shortcut must not interrupt", () => {
    expect(isTypingTarget({ tagName: "INPUT" } as unknown as EventTarget)).toBe(true);
    expect(isTypingTarget({ tagName: "TEXTAREA" } as unknown as EventTarget)).toBe(true);
    expect(isTypingTarget({ tagName: "SELECT" } as unknown as EventTarget)).toBe(true);
  });

  it("recognises the YAML editor, whose input is a contenteditable div", () => {
    expect(
      isTypingTarget({ tagName: "DIV", isContentEditable: true } as unknown as EventTarget),
    ).toBe(true);
  });

  it("recognises anything wearing the textbox role", () => {
    expect(
      isTypingTarget({
        tagName: "DIV",
        getAttribute: (name: string) => (name === "role" ? "textbox" : null),
      } as unknown as EventTarget),
    ).toBe(true);
  });

  it("is false for ordinary content and for no target at all", () => {
    expect(isTypingTarget({ tagName: "DIV", getAttribute: () => null } as unknown as EventTarget)).toBe(
      false,
    );
    expect(isTypingTarget(null)).toBe(false);
  });
});

describe("formatting", () => {
  it("writes Apple chords as run-together symbols", () => {
    expect(formatChord(["Mod", "K"], true)).toBe("⌘K");
    expect(formatChord(["Shift", "Enter"], true)).toBe("⇧↵");
  });

  it("writes everything else with Ctrl and plus signs", () => {
    expect(formatChord(["Mod", "K"], false)).toBe("Ctrl+K");
    expect(formatChord(["Shift", "Enter"], false)).toBe("Shift+Enter");
  });

  it("joins the alternatives of a two-chord shortcut", () => {
    const zoom = SHORTCUTS.find((s) => s.id === "zoom-in")!;
    expect(formatShortcut(zoom, false)).toBe("Ctrl++ or Ctrl+=");
  });

  it("detects Apple platforms from the platform string", () => {
    expect(isApplePlatform("MacIntel")).toBe(true);
    expect(isApplePlatform("iPhone")).toBe(true);
    expect(isApplePlatform("Win32")).toBe(false);
    expect(isApplePlatform("")).toBe(false);
  });
});

describe("what the sheet shows", () => {
  it("hides the desktop-only keys in a browser", () => {
    // The browser already owns Cmd +/-/0 and Cmd+W; listing them as srelens
    // shortcuts would be claiming keys we do not handle there.
    const web = visibleShortcuts(false).map((s) => s.id);
    expect(web).not.toContain("zoom-in");
    expect(web).not.toContain("close-tab");
    expect(web).toContain("palette");
    expect(visibleShortcuts(true).map((s) => s.id)).toContain("zoom-in");
  });

  it("hides Cmd-W off macOS, where nothing implements it", () => {
    // Close-tab comes from the macOS app menu, which is compiled only for
    // macOS — there is no key handler for it in the web layer.
    expect(visibleShortcuts(true, false).map((s) => s.id)).not.toContain("close-tab");
    expect(visibleShortcuts(true, true).map((s) => s.id)).toContain("close-tab");
    // Everything else survives the filter.
    expect(visibleShortcuts(true, false).map((s) => s.id)).toContain("zoom-in");
  });

  it("groups in the order the groups first appear", () => {
    const groups = groupedShortcuts(true).map(([name]) => name);
    expect(groups[0]).toBe("Global");
    expect(new Set(groups).size).toBe(groups.length);
  });

  it("keeps every visible shortcut in exactly one group", () => {
    const grouped = groupedShortcuts(true).flatMap(([, shortcuts]) => shortcuts);
    expect(grouped).toHaveLength(visibleShortcuts(true).length);
  });
});
