/**
 * Opening an address in the reader's own browser.
 *
 * **`window.open` and `<a target="_blank">` do nothing at all on the desktop,
 * and do it silently.** Tauri does not patch `window.open`, and wry's
 * `WKUIDelegate` returns nil for a new window unless the app installs a
 * new-window handler — srelens installs none. A link written the obvious way
 * therefore ships looking live and is dead on click (#348); classic's
 * `PortForwardsView` has that exact defect today.
 *
 * So the desktop goes through Rust, the same shape `saveTextFile` uses for the
 * same reason (`<a download>` does not save in a WebView either): the
 * `open_external` command hands the URL to `tauri-plugin-opener`, which is
 * already a dependency and already registered. Calling the plugin from a Rust
 * command needs no JS package and no capability permission — capabilities gate
 * JS→plugin calls, not Rust→plugin ones.
 *
 * In web mode the page genuinely is in a browser, `window.open` genuinely
 * works, and that is what it uses.
 */

import { invokeCommand } from "../transport/transport";
import { isTauri } from "../transport/platform";

/**
 * An absolute http(s) URL — scheme, `//`, and an authority with something in
 * it. Anchored, and the authority may not be empty, so `http://` on its own is
 * not a URL either.
 */
const ABSOLUTE_HTTP = /^https?:\/\/[^/?#\s]+/i;

/**
 * Anything a URL cannot contain: space, tab, newline, and the control
 * characters on either side of printable ASCII. Written as the printable
 * set NEGATED, so the pattern itself holds no control character.
 */
const NOT_IN_A_URL = /[^\u0021-\u007E\u00A0-\uFFFF]/;

/**
 * An address as something a browser can be sent to.
 *
 * `forwardAddress` answers in two shapes — a bare `host:port` authority on the
 * desktop and a full `http(s)://…/pf/<id>/` URL in web mode — because the
 * places that PRINT it want the short form. A browser wants a scheme, and a
 * bare `localhost:9090` handed to `window.open` resolves against the current
 * page rather than opening the tunnel.
 *
 * Deliberately separate from {@link openExternal} rather than folded into it:
 * an opener that quietly repaired whatever it was handed would be a general
 * "open any string" path, and the one thing this must never become is a way
 * for a value that arrived from a cluster to reach the OS.
 */
export function browsable(address: string): string {
  return /^https?:\/\//i.test(address) ? address : `http://${address}`;
}

/**
 * Open `url` in the reader's default browser.
 *
 * **Only ever pass an address srelens itself built** — a live forward's, via
 * `forwardAddress` and {@link browsable}. Never a string that arrived from a
 * cluster.
 *
 * Rejects anything that is not an absolute http(s) URL, on BOTH platforms, so
 * the refusal is the same wherever it is read: the Rust command applies the
 * same gate again, because that side is the one holding the handle to the OS.
 * Rejections are thrown for the caller to word through `describeError`.
 */
export async function openExternal(url: string): Promise<void> {
  if (!ABSOLUTE_HTTP.test(url) || NOT_IN_A_URL.test(url)) {
    throw new Error("srelens only opens http and https addresses.");
  }
  if (isTauri()) {
    await invokeCommand<null>("open_external", { url });
    return;
  }
  // The return value is NOT checked: with `noopener` the spec has
  // `window.open` return null even when the tab opened, so a null check here
  // would report every successful open as a failure.
  window.open(url, "_blank", "noopener,noreferrer");
}
