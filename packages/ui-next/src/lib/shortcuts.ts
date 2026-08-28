import { formatChord, isTypingTarget, type KeyEventLike, type KeyToken } from "@srelens/core";

export type WindowAction =
  | { type: "close-tab" }
  | { type: "new-tab" }
  | { type: "reopen-tab" }
  | { type: "prev-tab" }
  | { type: "next-tab" }
  | { type: "select-tab"; index: number }
  | { type: "console" }
  | { type: "lock" }
  | { type: "zoom-in" }
  | { type: "zoom-out" }
  | { type: "zoom-reset" };

type Binding = { chord: KeyToken[]; action: WindowAction; whileTyping?: boolean };

/**
 * The window's accelerators, bound once in `Window`. Core's matcher treats
 * `Mod` as Meta-or-Control, which is right for a palette and wrong here: on a
 * Mac, Ctrl+W belongs to the terminal it was typed into, and the mock's
 * `metaKey || ctrlKey` closed the document tab instead. So the modifier is
 * strict per platform and only the hint formatting is shared with core.
 */
const BINDINGS: Binding[] = [
  { chord: ["Mod", "W"], action: { type: "close-tab" } },
  { chord: ["Mod", "T"], action: { type: "new-tab" } },
  { chord: ["Mod", "Shift", "T"], action: { type: "reopen-tab" } },
  { chord: ["Mod", "["], action: { type: "prev-tab" } },
  { chord: ["Mod", "]"], action: { type: "next-tab" } },
  ...Array.from({ length: 9 }, (_, i) => ({
    chord: ["Mod", String(i + 1)],
    action: { type: "select-tab", index: i } as WindowAction,
  })),
  { chord: ["Mod", "K"], action: { type: "console" }, whileTyping: true },
  // §23 draws this beside `Lock now` and §25 names it. `whileTyping`, unlike
  // every other chord here: a reader who reaches for the lock with the caret
  // in a filter box, a YAML editor or a terminal is asking for the vault to be
  // sealed, and a security chord that quietly stood down because something had
  // focus is the kind of surprise that gets discovered after it mattered. The
  // window's own handler is what turns this into `vault_lock` — see `Window`.
  { chord: ["Mod", "Shift", "L"], action: { type: "lock" }, whileTyping: true },
  { chord: ["Mod", "="], action: { type: "zoom-in" } },
  // `+` is Shift+= on US/UK layouts, so the keydown really carries Shift; a
  // chord without the token would be a row that can never fire.
  { chord: ["Mod", "Shift", "+"], action: { type: "zoom-in" } },
  { chord: ["Mod", "-"], action: { type: "zoom-out" } },
  { chord: ["Mod", "0"], action: { type: "zoom-reset" } },
];

function matches(chord: KeyToken[], e: KeyEventLike, apple: boolean): boolean {
  const mod = apple ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey;
  const wantsShift = chord.includes("Shift");
  const key = chord[chord.length - 1];
  if (chord.includes("Mod") !== mod) return false;
  if (e.altKey) return false;
  if (wantsShift !== e.shiftKey) return false;
  return e.key.toLowerCase() === key.toLowerCase();
}

export function matchWindowKey(
  e: KeyEventLike & { target?: EventTarget | null },
  apple: boolean,
): WindowAction | null {
  const typing = isTypingTarget(e.target ?? null);
  for (const b of BINDINGS) {
    if (typing && !b.whileTyping) continue;
    if (matches(b.chord, e, apple)) return b.action;
  }
  return null;
}

export function hint(type: WindowAction["type"], apple: boolean): string {
  const b = BINDINGS.find((x) => x.action.type === type);
  return b ? formatChord(b.chord, apple) : "";
}
