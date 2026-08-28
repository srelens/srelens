// One list of the app's keyboard shortcuts (#160).
//
// Both the bindings App installs and the cheat sheet `?` opens read from here,
// so a shortcut cannot be added, moved, or removed in one place and stay stale
// in the other — which is the usual fate of a hand-maintained help screen.
//
// Shortcuts owned by a single component (a terminal's find bar, a dialog's
// Escape) are still handled where they live: hoisting them would mean routing
// key events through a global that has no idea which pane is focused. They are
// listed here with the surface they belong to so the sheet is complete.

/** A key combination, in tokens the formatter turns into platform symbols. */
export type KeyToken = "Mod" | "Shift" | "Alt" | string;

export interface Shortcut {
  id: string;
  /** Alternative chords for the same action (e.g. `Mod +` and `Mod =`). */
  chords: KeyToken[][];
  description: string;
  /** Heading it appears under in the cheat sheet. */
  group: string;
  /** Where the key is handled — a global binding, or the focused surface. */
  scope: "global" | "surface";
  /** Desktop-only: in a browser these keys already belong to the browser. */
  desktopOnly?: boolean;
  /** macOS-only: served by the app menu, which only macOS builds install. */
  appleOnly?: boolean;
}

/**
 * Every shortcut the app answers to.
 *
 * Ordered as the cheat sheet reads them, most-used first within each group.
 */
export const SHORTCUTS: readonly Shortcut[] = [
  {
    id: "palette",
    chords: [["Mod", "K"]],
    description: "Open the command palette",
    group: "Global",
    scope: "global",
  },
  {
    id: "cheatsheet",
    chords: [["?"]],
    description: "Show this shortcut list",
    group: "Global",
    scope: "global",
  },
  {
    id: "close-tab",
    chords: [["Mod", "W"]],
    description: "Close the current tab, or the window on the last one",
    group: "Global",
    scope: "global",
    desktopOnly: true,
    // Served by the macOS app menu (`install_macos_menu`, compiled only for
    // macOS), not by a key handler in the web layer. Listing it elsewhere
    // would promise a key nothing answers.
    appleOnly: true,
  },
  {
    id: "zoom-in",
    chords: [
      ["Mod", "+"],
      ["Mod", "="],
    ],
    description: "Make the interface larger",
    group: "Global",
    scope: "global",
    desktopOnly: true,
  },
  {
    id: "zoom-out",
    chords: [["Mod", "-"]],
    description: "Make the interface smaller",
    group: "Global",
    scope: "global",
    desktopOnly: true,
  },
  {
    id: "zoom-reset",
    chords: [["Mod", "0"]],
    description: "Reset the interface size",
    group: "Global",
    scope: "global",
    desktopOnly: true,
  },
  {
    id: "palette-move",
    chords: [["Up"], ["Down"]],
    description: "Move through the results",
    group: "Command palette",
    scope: "surface",
  },
  {
    id: "palette-run",
    chords: [["Enter"]],
    description: "Run the selected command",
    group: "Command palette",
    scope: "surface",
  },
  {
    id: "terminal-find",
    chords: [["Mod", "F"]],
    description: "Search the terminal's scrollback",
    group: "Terminal",
    scope: "surface",
  },
  {
    id: "terminal-find-next",
    chords: [["Enter"], ["Shift", "Enter"]],
    description: "Jump to the next or previous match",
    group: "Terminal",
    scope: "surface",
  },
  {
    id: "editor-find",
    chords: [["Mod", "F"]],
    description: "Find in the YAML editor",
    group: "Editing",
    scope: "surface",
  },
  {
    id: "dismiss",
    chords: [["Esc"]],
    description: "Close the open dialog, drawer, or search bar",
    group: "Everywhere",
    scope: "surface",
  },
];

/** Whether the platform labels the modifier `⌘` (and prints keys as symbols). */
export function isApplePlatform(platform: string = navigator.platform ?? ""): boolean {
  return /mac|iphone|ipad/i.test(platform);
}

const APPLE_SYMBOLS: Record<string, string> = {
  Mod: "⌘",
  Shift: "⇧",
  Alt: "⌥",
  Enter: "↵",
  Esc: "esc",
  Up: "↑",
  Down: "↓",
};
const OTHER_NAMES: Record<string, string> = {
  Mod: "Ctrl",
  Esc: "Esc",
  Up: "↑",
  Down: "↓",
};

/** One chord as the user's platform writes it: `⌘K` on a Mac, `Ctrl+K` elsewhere. */
export function formatChord(chord: readonly KeyToken[], apple: boolean): string {
  const keys = chord.map((token) =>
    apple ? (APPLE_SYMBOLS[token] ?? token) : (OTHER_NAMES[token] ?? token),
  );
  // Apple's convention runs modifiers together; everywhere else joins with `+`.
  return apple ? keys.join("") : keys.join("+");
}

/** The chords of a shortcut, formatted and joined for display. */
export function formatShortcut(shortcut: Shortcut, apple: boolean): string {
  return shortcut.chords.map((chord) => formatChord(chord, apple)).join(" or ");
}

/**
 * The shortcuts to show here: browser builds omit the ones the browser owns,
 * and non-Apple builds omit the ones only the macOS app menu provides. A sheet
 * that lists a key nothing answers is worse than one that omits it.
 */
export function visibleShortcuts(desktop: boolean, apple = true): Shortcut[] {
  return SHORTCUTS.filter((s) => (desktop || !s.desktopOnly) && (apple || !s.appleOnly));
}

/** Shortcuts grouped for display, in the order the groups first appear. */
export function groupedShortcuts(desktop: boolean, apple = true): Array<[string, Shortcut[]]> {
  const groups = new Map<string, Shortcut[]>();
  for (const shortcut of visibleShortcuts(desktop, apple)) {
    const list = groups.get(shortcut.group);
    if (list) list.push(shortcut);
    else groups.set(shortcut.group, [shortcut]);
  }
  return [...groups];
}

/** The parts of a keydown the matcher needs. */
export interface KeyEventLike {
  key: string;
  metaKey: boolean;
  ctrlKey: boolean;
  shiftKey: boolean;
  altKey: boolean;
}

/**
 * Whether `event` is the shortcut with id `id`.
 *
 * `Mod` is Cmd or Ctrl, whichever the platform sends, so one entry covers both.
 * A chord without `Shift` still matches a shifted key press when the character
 * itself requires Shift — `?` is Shift+/ on most layouts, and demanding an
 * unshifted `?` would mean demanding a key nobody has.
 */
export function matchesShortcut(id: string, event: KeyEventLike): boolean {
  const shortcut = SHORTCUTS.find((s) => s.id === id);
  if (!shortcut) return false;
  return shortcut.chords.some((chord) => {
    const wantsMod = chord.includes("Mod");
    const wantsShift = chord.includes("Shift");
    const wantsAlt = chord.includes("Alt");
    const key = chord.find((token) => !["Mod", "Shift", "Alt"].includes(token));
    if (!key) return false;
    if (wantsMod !== (event.metaKey || event.ctrlKey)) return false;
    if (wantsAlt !== event.altKey) return false;
    if (wantsShift && !event.shiftKey) return false;
    return event.key.toLowerCase() === key.toLowerCase();
  });
}

/**
 * Whether a key press is the user typing rather than commanding.
 *
 * Unmodified letter shortcuts have to check this or they fire mid-word: `?` in
 * a search box is a question mark, not a request for help. `isContentEditable`
 * covers the YAML editor, whose input is a div rather than a textarea.
 */
export function isTypingTarget(target: EventTarget | null): boolean {
  const element = target as
    | { tagName?: string; isContentEditable?: boolean; getAttribute?: (name: string) => unknown }
    | null;
  if (!element) return false;
  if (element.isContentEditable) return true;
  const tag = (element.tagName ?? "").toLowerCase();
  if (tag === "input" || tag === "textarea" || tag === "select") return true;
  return element.getAttribute?.("role") === "textbox";
}
