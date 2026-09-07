import type { CSSProperties, ReactNode } from "react";
import { filled } from "./slot";
import { toneColor, type Tone } from "./tone";

export interface TitlebarProps {
  /**
   * Which platform's window controls to draw a picture of.
   *
   * A prop, never a detection. Whether the app draws its own chrome at all is
   * the shell's decision — it depends on how the window was created, which the
   * design system cannot see and must not guess. Only macOS is drawn, because
   * only macOS is in the design; a Windows or Linux set would be an invention
   * rather than a port.
   */
  controls?: "macos" | "none";
  /** The leading column, after the window controls — a workspace switcher. */
  leading?: ReactNode;
  /** The centre column: what this window is. Doubles as the drag handle. */
  title?: ReactNode;
  /** The trailing column — zoom, theme, whatever else acts on the window. */
  actions?: ReactNode;
  /** Names the bar for assistive technology. */
  label?: string;
}

/**
 * The bar across the top of the window: a picture of the platform's window
 * controls, whatever names the window, and the controls that act on it.
 *
 * The visual only, and deliberately so. The mock's version read the theme, the
 * zoom, the active workspace and the hotbar out of four stores and drew its own
 * buttons from them; none of that can come across, because a design system that
 * knows what a workspace is has stopped being one. What is left is the design's
 * three-column chrome and three slots to fill it with, so the same bar can name
 * a window in the app and a frame in the gallery.
 *
 * The traffic lights are decoration, marked as such. They are a picture of the
 * macOS window controls, not the controls: closing, minimising and zooming
 * belong to the window, which this component has no handle on, so drawing them
 * as buttons would put three keyboard-reachable, screen-reader-announced
 * controls on the page that do nothing when pressed. Whether they should be
 * drawn at all is the caller's — hence `controls`, which the shell answers from
 * how it created the window rather than from what platform it is running on.
 *
 * The drag region is the other thing a titlebar has that a toolbar does not.
 * `.titlebar` makes the whole bar draggable, which would swallow the click on
 * anything pressable standing in it; the design got away with that because
 * every control it put up here wore `.icon-btn`, which opts back out. A slot
 * takes whatever the caller passes, so each one opts out here instead. (#320)
 */
export function Titlebar({ controls = "none", leading, title, actions, label = "Titlebar" }: TitlebarProps) {
  const macos = controls === "macos";
  return (
    // `data-tauri-drag-region` is what makes these regions actually draggable
    // under Tauri: its injected script starts a native drag when the mousedown
    // *target* carries the attribute. `-webkit-app-region`, which the
    // stylesheet also sets, is honoured by WebView2 and ignored by macOS's
    // WKWebView — without the attribute the overlay titlebar drew beautifully
    // and moved nothing. Children without the attribute stay interactive, so
    // the slots opt out by being elements of their own.
    <header className="titlebar" aria-label={label} data-tauri-drag-region>
      <div className="flex items-center gap-2 pl-3.5" data-tauri-drag-region>
        {macos && (
          <span data-window-controls aria-hidden="true" className="flex items-center gap-2">
            {MACOS_LIGHTS.map((tone) => (
              // Tones rather than the mock's #ff5f57/#febc2e/#28c840: those
              // three do not move when the theme does, and the kit's own lint
              // forbids a component naming a colour. They are close enough to
              // the platform's that the row still reads as what it is.
              <span key={tone} data-light className="light" style={{ background: toneColor(tone) }} />
            ))}
          </span>
        )}
        {macos && filled(leading) && <span data-chrome-rule className="mx-1.5 h-3.5 w-px" style={RULE} />}
        {filled(leading) && (
          <span data-slot="leading" className="inline-flex items-center gap-2" style={NO_DRAG}>
            {leading}
          </span>
        )}
      </div>

      {/* Bare on purpose: this column is the part of the bar left for dragging
          the window, so nothing pressable goes in it. */}
      <div data-drag-region data-tauri-drag-region className="path flex items-center gap-2 justify-self-center">
        {title}
      </div>

      <div className="flex items-center justify-end gap-0.5 pr-2">
        {filled(actions) && (
          <span data-slot="actions" className="inline-flex items-center gap-0.5" style={NO_DRAG}>
            {actions}
          </span>
        )}
      </div>
    </header>
  );
}

/** Close, minimise, zoom — in the order macOS draws them. */
const MACOS_LIGHTS: Tone[] = ["sev", "warn", "ok"];

const RULE: CSSProperties = { background: "var(--rule)" };

/**
 * Cast because `-webkit-app-region` is not in React's style types: it is a
 * webview property rather than a web one, and only the shell's window ever
 * honours it. In a browser it is inert.
 */
const NO_DRAG = { WebkitAppRegion: "no-drag" } as CSSProperties;
