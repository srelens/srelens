// @vitest-environment node
// (reads files from disk; jsdom buys nothing here)
import { describe, expect, it } from "vitest";
import { readFileSync, readdirSync } from "node:fs";
import { join, relative, sep } from "node:path";

/**
 * Tauri v2 gates every window command behind an ACL entry in
 * `capabilities/default.json`. Nothing checks the two against each other:
 * `getCurrentWindow().destroy()` type-checks, ships, and then rejects at
 * runtime on a build that never granted `core:window:allow-destroy`.
 *
 * That is #425. `core:default` brings in `core:window:default`, which is the
 * READ-ONLY half of the window API — every getter, no mutator. So the close
 * this app intercepts and re-issues was refused by the ACL, nothing ever closed
 * the window, and the macOS red traffic light did nothing; only Cmd+Q, which
 * quits the process without reaching the webview's close-requested handler,
 * could shut the app down. Cmd+W on the last tab was refused the same way.
 *
 * A rejected promise was the only symptom and both call sites swallowed it, so
 * the app failed silently and identically on every platform. Pinned here
 * because the mismatch is invisible to the compiler and to every test that
 * mocks the window away — which is all of them.
 */
const root = join(__dirname, "..");
const capabilities = JSON.parse(
  readFileSync(join(root, "src-tauri/capabilities/default.json"), "utf8"),
) as { permissions: string[] };

/**
 * Methods on the window handle that are not commands: they subscribe through
 * the event system, which `core:default` already covers, so they need no
 * `core:window:allow-*` of their own.
 */
const NOT_COMMANDS = new Set(["onCloseRequested", "onResized", "onMoved", "onFocusChanged", "listen", "once"]);

/** `setTitleBarStyle` -> `set-title-bar-style`, the shape ACL keys take. */
function kebab(method: string): string {
  return method.replace(/([a-z0-9])([A-Z])/g, "$1-$2").toLowerCase();
}

/** Every `.ts`/`.tsx` under `dir`, minus tests and type declarations. */
function sources(dir: string): string[] {
  const out: string[] = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) {
      if (entry.name !== "node_modules") out.push(...sources(path));
    } else if (/[.]tsx?$/.test(entry.name) && !/[.]test[.]|[.]d[.]ts$/.test(entry.name)) {
      out.push(path);
    }
  }
  return out;
}

/**
 * The window commands the frontend actually calls, keyed by file.
 *
 * Deliberately a scan for the two shapes this repo uses —
 * `getCurrentWindow().m()`, and `const win = getCurrentWindow()` followed by
 * `win.m()` — rather than a parse. A third shape would go unseen here instead
 * of failing, which is the safe direction for a test whose job is to cover the
 * shapes that exist.
 */
function windowCalls(): Map<string, Set<string>> {
  const HANDLE = "getCurrentWindow[(][)]";
  const found = new Map<string, Set<string>>();
  for (const file of sources(join(root, "src"))) {
    const text = readFileSync(file, "utf8");
    if (!text.includes("getCurrentWindow()")) continue;
    const handles = [HANDLE];
    const bound = new RegExp("(?:const|let) +([A-Za-z_][A-Za-z0-9_]*) *= *" + HANDLE, "g");
    for (const [, name] of text.matchAll(bound)) handles.push(name);

    const calls = new RegExp("(?:" + handles.join("|") + ")[.]([A-Za-z]+) *[(]", "g");
    for (const [, method] of text.matchAll(calls)) {
      if (NOT_COMMANDS.has(method)) continue;
      const key = relative(root, file).split(sep).join("/");
      if (!found.has(key)) found.set(key, new Set());
      found.get(key)!.add(method);
    }
  }
  return found;
}

describe("Tauri window capabilities", () => {
  it("grants close and destroy, which every window close goes through (#425)", () => {
    // Named outright rather than only derived below: these two are what the
    // bug was, so a refactor that stops calling them should have to delete
    // this on purpose.
    expect(capabilities.permissions).toContain("core:window:allow-close");
    expect(capabilities.permissions).toContain("core:window:allow-destroy");
  });

  it("grants every window command the frontend calls", () => {
    const calls = windowCalls();
    expect(calls.size, "no getCurrentWindow() call found at all — has this scan gone stale?").toBeGreaterThan(0);

    const missing: string[] = [];
    for (const [file, methods] of calls) {
      for (const method of methods) {
        const permission = "core:window:allow-" + kebab(method);
        if (!capabilities.permissions.includes(permission)) {
          missing.push(file + " calls " + method + "(), which needs " + permission);
        }
      }
    }
    expect(missing, "capabilities/default.json is missing window grants: " + missing.join("; ")).toEqual([]);
  });
});
