import { useEffect } from "react";
import type { ITheme, Terminal } from "@xterm/xterm";
import { terminalFor } from "./sessions";

/**
 * The design tokens the emulator is dressed from.
 *
 * xterm paints its own canvas and knows nothing about this app's stylesheet,
 * so unlike every other surface here it cannot simply inherit. Left alone it
 * draws its own dark palette in a white pane, in a font that matches no other
 * monospace surface in the app — which is what a terminal that reads as
 * foreign looks like.
 *
 * **The token NAMES are here; the values are not.** They are read off the
 * document at {@link terminalDress}, so a theme change, a palette change or a
 * token edit carries into the transcript without this file knowing any colour.
 * That is the same rule `toneColor` follows for everything drawn in CSS; this
 * is the one place that has to resolve a token to a value rather than hand the
 * browser a `var()`, because a canvas cannot take one.
 *
 * The ANSI half maps only the four colours the token set has an honest answer
 * for — a program's `red` means "bad" and so does `--sev`. `cyan` is left to
 * xterm's own palette: there is no token that means cyan, and pointing it at
 * `--info` would be inventing a meaning for someone else's output. The black
 * and white ends come from the ink ramp rather than literal black and white,
 * which is what keeps a program's `white` legible on a light theme — the one
 * place a terminal's defaults are actively wrong here.
 */
const TERMINAL_TOKENS: Readonly<Partial<Record<keyof ITheme, string>>> = {
  // §14 draws the transcript as a SUNK surface, not the pane's own. Named by
  // its ground token so a see-through window clears it with the rest of the
  // sheet — a terminal left solid is a slab in the middle of the glass.
  background: "--ground-sunk",
  foreground: "--ink-soft",
  // §14: "a pulsing accent block cursor sits on the last line".
  cursor: "--accent",
  cursorAccent: "--surface-sunk",
  selectionBackground: "--accent-wash",
  black: "--ink",
  red: "--sev",
  green: "--ok",
  yellow: "--warn",
  blue: "--info",
  magenta: "--accent",
  white: "--ink-muted",
  brightBlack: "--ink-faint",
  brightRed: "--sev",
  brightGreen: "--ok",
  brightYellow: "--warn",
  brightBlue: "--info",
  brightMagenta: "--accent",
  brightWhite: "--ink",
};

/** The token the app's monospace surfaces already share. */
const FONT_TOKEN = "--font-mono";

/**
 * The size the app's monospace reading surfaces are set in — the log body,
 * the YAML editor's 12px — named by its token rather than copied (#942).
 * Left unset, xterm draws at its own 15px, visibly larger than every other
 * line of machine text in the window.
 */
const FONT_SIZE_TOKEN = "--text-label";

/** What `--text-label` is at the default root size, for a document without a stylesheet. */
export const TERMINAL_FONT_SIZE = 12;

/**
 * A CSS length as pixels, for a canvas that cannot take `rem`. Only the two
 * units the token set uses; anything else is not a size this can read.
 *
 * The interface scale (#237) is the webview's native zoom, which does not
 * change a computed CSS pixel — so a size read once in pixels still follows
 * it, as every other surface does.
 */
export function cssPixels(value: string, rootPx: number): number | undefined {
  const match = /^(-?[\d.]+)(px|rem)$/.exec(value.trim());
  if (!match) return undefined;
  const n = Number(match[1]);
  if (!Number.isFinite(n) || n <= 0) return undefined;
  return match[2] === "rem" ? n * rootPx : n;
}

/**
 * Resolve {@link TERMINAL_TOKENS} against the document root.
 *
 * The root rather than the pane, deliberately: `:root` is where the token
 * block is declared and `data-theme` is where the mode is written
 * (`applyNextDesignTheme` in `apps/desktop/src/design.ts`), so this reads the
 * same element both of them speak about.
 *
 * A token that resolves to nothing is left out rather than written as an empty
 * string — xterm treats `""` as a colour and throws parsing it, and a document
 * with no stylesheet attached (a unit test, a first paint) is a real state.
 * Left out, xterm keeps its own default for that one key.
 */
export function terminalDress(root: Element): { theme: ITheme; fontFamily?: string; fontSize: number } {
  const style = getComputedStyle(root);
  const read = (token: string) => style.getPropertyValue(token).trim();
  const theme: Record<string, string> = {};
  for (const [key, token] of Object.entries(TERMINAL_TOKENS)) {
    const value = read(token);
    // A cleared ground resolves to the keyword, and xterm parses colours
    // itself: it takes `rgba()` and throws on `transparent`.
    if (value) theme[key] = value === "transparent" ? "rgba(0, 0, 0, 0)" : value;
  }
  const fontFamily = read(FONT_TOKEN);
  const rootPx = cssPixels(style.fontSize, 16) ?? 16;
  const fontSize = cssPixels(read(FONT_SIZE_TOKEN), rootPx) ?? TERMINAL_FONT_SIZE;
  return { theme, fontFamily: fontFamily || undefined, fontSize };
}

/**
 * Keep the session on screen wearing the app's tokens — on every surface a
 * session is drawn on, the Terminals tab and a page's terminal panel alike,
 * which is why this is `TerminalView`'s and not one screen's (#942).
 *
 * Re-run when the theme attribute changes, because that is a theme switch:
 * the tokens under it now resolve to different values and the emulator has
 * already read the old ones. Nothing else observes `data-theme` — the rest of
 * the app is CSS and follows on its own — which is exactly why the one canvas
 * in the app has to.
 */
/**
 * Dress one emulator from the document's tokens now. `TerminalView` calls
 * this before it fits the emulator to its pane, because the font size decides
 * how many columns fit: dressed after the fit, the shell would be told a
 * width measured in xterm's own larger cells.
 */
export function dressTerminal(term: Terminal, root: Element = document.documentElement): void {
  const { theme, fontFamily, fontSize } = terminalDress(root);
  term.options.theme = theme;
  if (fontFamily) term.options.fontFamily = fontFamily;
  // Only when it differs: a size change re-measures every cell.
  if (term.options.fontSize !== fontSize) term.options.fontSize = fontSize;
}

export function useTerminalDress(sessionId: number | null): void {
  useEffect(() => {
    if (sessionId === null) return undefined;
    const root = document.documentElement;
    const dress = () => {
      const term = terminalFor(sessionId);
      if (term) dressTerminal(term, root);
    };
    dress();
    const observer = new MutationObserver(dress);
    observer.observe(root, { attributes: true, attributeFilter: ["data-theme", "data-opacity"] });
    return () => observer.disconnect();
  }, [sessionId]);
}
