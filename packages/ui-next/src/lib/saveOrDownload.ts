import { isTauri, saveTextFile } from "@srelens/core";

/**
 * Save `content` to `filename`: through the native save dialog in the desktop
 * shell, and as a browser download in web mode.
 *
 * Both halves are needed, and neither works in the other's place — a Tauri
 * webview does not prompt on `<a download>`, and a browser has no
 * `save_text_file` command to invoke. Classic reached the same conclusion
 * (`apps/desktop/src/components/LogsView.tsx`); this is that decision written
 * where the new design's screens can use it, not a second policy.
 */
export async function saveOrDownload(filename: string, content: string): Promise<void> {
  if (isTauri()) {
    await saveTextFile(filename, content);
    return;
  }
  const url = URL.createObjectURL(new Blob([content], { type: "text/plain" }));
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = filename;
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
  URL.revokeObjectURL(url);
}
