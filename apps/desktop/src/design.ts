import { isApplePlatform, isTauri, K8S_KIND, type ResourceKind } from "@srelens/core";
// theme.ts imports only settingsStorage, so this does not drag the classic
// stylesheet into the new design's chunk.
import { applyTheme, getInitialTheme, resolvedThemeMode } from "./ui/theme";

/**
 * Which design the app renders.
 *
 * Read synchronously before React mounts, so it lives in localStorage rather
 * than the settings store, which is async. It describes the person using the
 * app, not the cluster they are looking at, so it is not scoped to a context
 * or a workspace.
 *
 * This module deliberately sits outside `main.tsx`: importing the entry module
 * from a component or a test would run its side effects — installing the
 * notifier and starting the app.
 */
export const DESIGN_KEY = "srelens.design";

export type Design = "classic" | "next";

export function loadDesign(): Design {
  try {
    // Anything unrecognised means classic. A preference written by a future
    // version must never leave someone on a design that does not exist, since
    // a blank window has no way back to Settings.
    return localStorage.getItem(DESIGN_KEY) === "next" ? "next" : "classic";
  } catch {
    // Storage throws in some privacy modes; a preference is not worth failing
    // to boot over.
    return "classic";
  }
}

/** Persist the choice. Returns false if storage refused it. */
export function saveDesign(design: Design): boolean {
  try {
    localStorage.setItem(DESIGN_KEY, design);
    return true;
  } catch {
    // Restricted or private storage. The caller must not reload on this: the
    // next boot would read no preference and come back on the old design.
    return false;
  }
}

/**
 * Apply a design choice.
 *
 * Reloads rather than swapping trees in place: the two designs' stylesheets
 * cannot share a document — both import Tailwind, use different dark-mode
 * conventions and write global rules — and unloading one at runtime is not
 * something the platform offers. A reload on a deliberate, rare action is a
 * fair price for removing a whole class of style-bleed bug.
 */
/** Whether a design switch went through, and why not when it did not. */
export type SwitchResult = { ok: true } | { ok: false; reason: string };

/**
 * Whether the new design draws its own titlebar here.
 *
 * The mock's traffic lights are macOS-shaped, so only Apple gets the overlay
 * for now; Windows and Linux keep the system decorations until the design has
 * an answer for their controls. Optional argument for the callers that already
 * hold a platform string; the runtime answers for itself when they do not.
 */
export function drawsOwnChrome(platform?: string): boolean {
  return isApplePlatform(platform);
}

/**
 * Dress the window for the new design, once per boot of it.
 *
 * The titlebar goes overlay so the design's own Titlebar sits flush under the
 * traffic lights without doubling the chrome. Cosmetic, and explicitly not
 * allowed to block boot: on a build where
 * `core:window:allow-set-title-bar-style` is not granted this throws, and a
 * rejecting promise escaping would have left bootstrap awaiting forever — a
 * blank window instead of an undressed one.
 */
export async function applyNextDesignChrome(): Promise<void> {
  if (!isTauri() || !drawsOwnChrome()) return;
  try {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    const win = getCurrentWindow();
    // An overlay style keeps macOS painting the native title over the
    // webview — "srelens" from tauri.conf.json landed square on the workspace
    // switcher — so the name is cleared and the design's own Titlebar speaks
    // for the window. switchDesign("classic") gives it back.
    await win.setTitle("");
    await win.setTitleBarStyle("overlay");
  } catch {
    // Wrong chrome is a blemish; a failed boot is a broken app.
  }
}

export async function switchDesign(design: Design): Promise<SwitchResult> {
  if (!saveDesign(design)) {
    // The choice could not be persisted, so a reload would come back on the
    // old design — and on desktop it could do so with the chrome already
    // changed. Leave everything alone and report it.
    //
    // Reported to the caller rather than raised as a toast: the toast host
    // lives in the classic tree, so a failure while leaving the new design
    // would have been invisible, and the button would have looked inert.
    return { ok: false, reason: "This device would not let srelens save the preference." };
  }
  if (design === "classic" && isTauri() && drawsOwnChrome()) {
    try {
      // Leaving the new design means handing the system titlebar back: classic
      // renders under the real decorations, and an overlay left behind would
      // double the chrome. The native title comes back with it — it was
      // cleared for the overlay, and classic draws no name of its own. Going
      // the other way dresses nothing here — the next boot's
      // applyNextDesignChrome owns that direction.
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      const win = getCurrentWindow();
      await win.setTitle("srelens");
      await win.setTitleBarStyle("visible");
    } catch {
      // Wrong chrome is a blemish; not switching at all is a broken setting.
    }
  }
  window.location.reload();
  return { ok: true };
}

/**
 * Carry the user's light/dark choice into the new design.
 *
 * The two designs disagree about what `data-theme` means: the classic one puts
 * the palette name there (`slate`) and the mode in `data-theme-mode`, while
 * ui-next's stylesheet reads `data-theme="dark"` as the mode itself. Nothing
 * translates between them, and a reload starts from a bare document — so
 * without this the new design always rendered light, including for the many
 * users on the classic default, which is dark. (#314 review)
 *
 * Light is the absence of the attribute, matching ui-next's `:root` tokens.
 */
/**
 * The new design's reading of the stored preference, as an attribute on the
 * root. Shared by boot, the system-appearance listener and the toggle — three
 * writers of one convention is two too many.
 *
 * Light is the absence of the attribute, matching ui-next's `:root` tokens.
 */
function applyNextThemeAttribute(): void {
  const root = document.documentElement;
  if (resolvedThemeMode(getInitialTheme().mode) === "dark") {
    root.dataset.theme = "dark";
  } else {
    delete root.dataset.theme;
  }
}

export function applyNextDesignTheme(): () => void {
  applyNextThemeAttribute();

  // Someone on "system" changes appearance while the app is open, and the new
  // tree has no equivalent of the classic App's matchMedia effect, so it would
  // sit on a stale palette until the next reload. (#314 review)
  if (getInitialTheme().mode !== "system") return () => {};
  const query = window.matchMedia("(prefers-color-scheme: dark)");
  const apply = () => applyNextThemeAttribute();
  query.addEventListener("change", apply);
  return () => query.removeEventListener("change", apply);
}

/**
 * Flip light/dark for both designs at once.
 *
 * The write goes through classic's `applyTheme`, so one stored preference
 * drives both designs; but that write leaves classic's conventions on the root
 * (`data-theme` = palette name), which ui-next reads as a mode. Re-asserting
 * our own attribute afterwards keeps the two designs' readings from fighting
 * over the same element.
 */
export function toggleNextDesignTheme(): void {
  const current = getInitialTheme();
  const mode = resolvedThemeMode(current.mode);
  applyTheme({ ...current, mode: mode === "dark" ? "light" : "dark" });
  applyNextThemeAttribute();
}

/**
 * Which screens exist in the new design. Classic's Settings shows this beside
 * the toggle so the choice is informed before it is made, and the new design's
 * Placeholder shows it so the user knows what is there. One list, read by both,
 * so they cannot drift. A screen is added here in the PR that ports it.
 */
export const PORTED_SCREENS: ReadonlyArray<{ route: string; name: string }> = [
  { route: "/applog", name: "Application log" },
  { route: "/notes", name: "Release notes" },
  { route: "/resources", name: "Workloads" },
  // Not one screen: every `/k/<slug>` route, around 34 built-in kinds plus
  // every custom resource the cluster has, all sharing one screen keyed off
  // a descriptor — and every `/k/<kind>/<namespace>/<name>` route beneath
  // them, which is the resource detail. "/k" names the family here — it is
  // not itself a route.
  //
  // The detail is one entry with the lists rather than its own, because it
  // shares their prefix and because a reader who has a kind's list has its
  // details too: there is no build in which one is ported and the other is
  // not.
  { route: "/k", name: "Resource lists and details" },
  // Its own screen over the same list engine, not one of the `/k` kinds:
  // `/events` has its own chrome — the by-reason rail and the type filter —
  // and `/k/events` resolves to it too.
  { route: "/events", name: "Events" },
  // The sidebar's first cluster node. Not one of the `/k` lists and not the
  // control room: it is the cluster's own front page — the capacity strip, the
  // nodes table, the unhealthy list and the At a glance rail.
  { route: "/overview", name: "Cluster overview" },
  // A live stream rather than a view of a list: `/logs` tails every container
  // behind a workload, or one pod, and keeps tailing after you switch tabs.
  // Its deeper `/logs/<kind>/<namespace>/<name>` shape is the same screen with
  // a subject in it, so "/logs" names both here.
  { route: "/logs", name: "Logs" },
  // A tunnel outlives the tab that opened it, so this screen is where the ones
  // already running are seen and stopped — not a view of a cluster's state but
  // of this process's own.
  { route: "/forwards", name: "Port forwards" },
  // The only screen here that is about the machine rather than a cluster: the
  // managed kubectl, helm and krew under ~/.srelens/bin, and what the active
  // context's exec-auth needs on PATH.
  { route: "/toolbox", name: "Toolbox" },
];

/**
 * Where classic should land after leaving the new design.
 *
 * The switch reloads the document, so the note rides `sessionStorage`: a
 * handoff is for the one reload that follows, never for a later launch, and
 * session scope makes the difference structural rather than remembered.
 */
export const HANDOFF_KEY = "srelens.design.handoff";

export interface Handoff {
  context: string;
  kind: ResourceKind;
}

/**
 * The classic view a route in the new design stands for.
 *
 * Pure per R-F: it carries `{ context, kind }` and nothing else. A route whose
 * kind classic does not have — or cannot parse — still lands on the cluster's
 * overview, because standing somewhere near where you were beats being dumped
 * at home with no trace of the cluster you were looking at.
 */
export function handoffFor(route: string, context?: string): Handoff | null {
  if (!context) return null;
  const slug = /^\/k\/([^/]+)$/.exec(route)?.[1];
  if (slug && slug !== "overview" && Object.prototype.hasOwnProperty.call(K8S_KIND, slug)) {
    return { context, kind: slug as ResourceKind };
  }
  switch (route) {
    case "/events":
      return { context, kind: "events" };
    case "/forwards":
      return { context, kind: "portforwards" };
    case "/helm":
      return { context, kind: "helmreleases" };
    default:
      // `/`, `/overview`, and every route classic has no answer for.
      return { context, kind: "overview" };
  }
}

/** Note where the new design was, for classic to pick up after the reload. */
export function saveHandoff(route: string, context?: string): void {
  try {
    const handoff = handoffFor(route, context);
    if (handoff) sessionStorage.setItem(HANDOFF_KEY, JSON.stringify(handoff));
  } catch {
    // Session storage throws in some privacy modes; losing the handoff is
    // landing on classic's overview, which is where it lands anyway.
  }
}

/**
 * Consume the handoff. Reading and removing are one action: a caller that
 * reads without clearing would reopen the same view on every launch.
 */
export function takeHandoff(): Handoff | null {
  try {
    const raw = sessionStorage.getItem(HANDOFF_KEY);
    sessionStorage.removeItem(HANDOFF_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<Handoff> | null;
    if (parsed && typeof parsed.context === "string" && typeof parsed.kind === "string") {
      return { context: parsed.context, kind: parsed.kind };
    }
    return null;
  } catch {
    return null;
  }
}
