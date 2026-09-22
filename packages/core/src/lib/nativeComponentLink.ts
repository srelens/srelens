import { isTauri } from "../transport/platform";
import { invokeCommand } from "../transport/transport";

/** Only absolute HTTP(S) links are navigable in host-rendered component data. */
export function normalizeNativeComponentLink(value: string): string | null {
  // Reject browser repairs and invisible direction changes before URL parses it.
  if (!/^https?:\/\/[^/?#]+/i.test(value) || /[\s\u0000-\u001f\u007f-\u009f\u061c\u200e\u200f\u202a-\u202e\u2066-\u2069\\]/u.test(value)) {
    return null;
  }
  const authority = value.slice(value.indexOf("://") + 3).split(/[/?#]/, 1)[0];
  if (authority.includes("@")) return null;
  try {
    const url = new URL(value);
    if (!url.hostname || url.username || url.password || !["http:", "https:"].includes(url.protocol)) {
      return null;
    }
    return url.href;
  } catch {
    return null;
  }
}

/** Call only from an explicit user click; validation itself never navigates. */
export async function openNativeComponentLink(value: string): Promise<void> {
  const url = normalizeNativeComponentLink(value);
  if (!url) throw new Error("srelens only opens absolute http and https links without credentials.");
  if (isTauri()) {
    await invokeCommand<null>("open_external", { url });
    return;
  }
  window.open(url, "_blank", "noopener,noreferrer");
}
