// @vitest-environment node
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { expect, it } from "vitest";

it("lets document-tab HTML drag/drop reach the desktop webview", () => {
  const config = JSON.parse(readFileSync(join(__dirname, "../src-tauri/tauri.conf.json"), "utf8"));
  // Tauri's native file-drop handler consumes drops before the DOM sees them.
  // Browser-only tab tests cannot catch this desktop host configuration.
  expect(config.app.windows.length).toBeGreaterThan(0);
  for (const window of config.app.windows) expect(window.dragDropEnabled).toBe(false);
});
