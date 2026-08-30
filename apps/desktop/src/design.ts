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
 * One side of light/dark, as an attribute on the root. The only writer of that
 * convention here — boot, the system-appearance listener and the toggle all go
 * through it, because three spellings of one convention is two too many.
 *
 * Takes the side rather than reading it, so the toggle can write the side it
 * just chose instead of asking the store, which `applyTheme` persists to only
 * best-effort.
 *
 * Light is the absence of the attribute, matching ui-next's `:root` tokens.
 */
function writeNextThemeAttribute(lightness: "dark" | "light"): void {
  const root = document.documentElement;
  if (lightness === "dark") root.dataset.theme = "dark";
  else delete root.dataset.theme;
}

/** The new design's reading of classic's stored light/dark preference. */
function applyNextThemeAttribute(): void {
  writeNextThemeAttribute(resolvedThemeMode(getInitialTheme().mode));
}

/**
 * The named themes whose token block paints a DARK ground.
 *
 * ui-next offers five themes and this module can only spell two of them, so the
 * light/dark button needs to know which side each of the other three is on
 * before it can flip away from one. That answer belongs to the stylesheet:
 * `packages/ui-kit/src/styles/tokens.css` gives `dark` and `midnight` a canvas
 * darker than their ink and groups the two of them wherever a rule is for the
 * dark grounds — the mark palette, every accent override — while `paper` and
 * `contrast` and the bare `:root` are light grounds.
 *
 * Copied here rather than imported because importing `@srelens/ui-kit` from
 * this module would drag the whole kit into the chunk a CLASSIC boot downloads
 * — the same wall the design toggle and `brandMarkSrc` hit. So the copy is
 * PINNED: `design.theme.test.ts` parses that stylesheet, derives the dark side
 * from the `--canvas`/`--ink` pair each block declares, and fails if this set
 * and the stylesheet ever disagree. A theme added to `tokens.css` fails that
 * test rather than quietly being treated as light here.
 */
export const DARK_NEXT_THEMES: ReadonlySet<string> = new Set(["dark", "midnight"]);

/**
 * The lightness of the theme the reader can actually SEE.
 *
 * Anything this build does not recognise counts as light, because that is what
 * the stylesheet does with it: no `[data-theme]` block matches, so `:root`'s
 * light tokens are what gets painted. A bare root is light for the same reason.
 */
function visibleLightness(): "dark" | "light" {
  const named = document.documentElement.getAttribute("data-theme");
  return named !== null && DARK_NEXT_THEMES.has(named) ? "dark" : "light";
}

/**
 * @param themeChosen Whether the reader has NAMED one of ui-next's five themes,
 * in which case this function keeps its hands off `data-theme` entirely — it
 * neither derives a value nor arms the listener that would.
 *
 * The reading here is derived, from classic's light/dark preference, and it
 * knows only two of those five: `dark`, or the attribute's absence. So for a
 * reader on `system` the listener below was overwriting a chosen theme —
 * Midnight became plain dark, Paper became bare light — at the next OS change,
 * for the rest of the session and with nothing on screen to say why. (#373
 * review)
 *
 * The predicate is a parameter rather than a read, because the answer lives in
 * ui-next's stored appearance record and nowhere else: the document cannot be
 * asked, since `dark` is both a derivation and a named theme and a bare root is
 * both "nothing read yet" and a chosen Light. That record sits on the chunk the
 * new design is loaded from — a static import of it here would put the whole
 * new tree in the entry chunk a classic boot downloads — so `main.tsx` hands it
 * down. It defaults to "nothing named", which is what the first call of the
 * boot, made before that chunk exists, has to assume.
 */
export function applyNextDesignTheme(themeChosen: () => boolean = () => false): () => void {
  if (themeChosen()) return () => {};
  applyNextThemeAttribute();

  // Someone on "system" changes appearance while the app is open, and the new
  // tree has no equivalent of the classic App's matchMedia effect, so it would
  // sit on a stale palette until the next reload. (#314 review)
  if (getInitialTheme().mode !== "system") return () => {};
  const query = window.matchMedia("(prefers-color-scheme: dark)");
  const apply = () => {
    // Asked again on every change, not once at arm time: the Appearance pane is
    // live, so a reader who boots having named nothing and then picks Midnight
    // has to stand this listener down without a reload.
    if (themeChosen()) return;
    applyNextThemeAttribute();
  };
  query.addEventListener("change", apply);
  return () => query.removeEventListener("change", apply);
}

/**
 * Flip light/dark for both designs at once.
 *
 * **The direction comes off the document, not out of classic's store.** It used
 * to come from the stored mode, and a named theme never writes that mode:
 * `pickTheme` lives in `packages/ui-next`, classic's mode store lives here in
 * the package that DEPENDS on it, and a static import upward is a cycle across
 * that boundary. So a reader on the classic default — `dark` — who picks Paper
 * has a light window and a store still saying `dark`, and one click of this
 * button flipped that stale `dark` to `light`: the window stayed light, Paper
 * was deleted, and the reader who asked to go dark got neither the dark they
 * asked for nor the theme they had. (#373 review, round 5)
 *
 * **Which side a theme is on is the stylesheet's answer**, read through
 * {@link DARK_NEXT_THEMES}.
 *
 * **What it flips TO is the plain pair — `dark`, or the bare root.** Paper and
 * High contrast both land on `dark`, Midnight lands on bare light. There is no
 * dark counterpart of Paper and no dark High contrast in `tokens.css` to land
 * on, and nothing anywhere remembers which dark theme this reader would have
 * wanted, so inventing a pairing would be this module guessing at a design
 * decision §23 has not made. Naming one of the five is the Appearance pane's
 * job; this button's job is the lightness, and it says so — it is a sun, not a
 * theme carousel.
 *
 * The write goes through classic's `applyTheme`, so one stored preference
 * drives both designs and they agree about light and dark after the click even
 * when they did not before. But that write leaves classic's conventions on the
 * root (`data-theme` = palette name), which ui-next reads as a mode, so the new
 * design's attribute is asserted afterwards — from the side just chosen rather
 * than re-read through the store, which `applyTheme` persists to only
 * best-effort. Re-reading put the OLD side back on the root for the reader
 * whose device refuses to save preferences.
 *
 * `Chrome` calls ui-next's `rememberTheme()` straight after this, which stores
 * whatever landed on the root — so the record follows the visible flip, and
 * `hasChosenTheme` stands the OS follower down on the side the reader chose. A
 * click is an explicit answer about lightness, which is why recording it is a
 * choice and not the derivation `appearance.ts` warns against.
 */
export function toggleNextDesignTheme(): void {
  const current = getInitialTheme();
  const next = visibleLightness() === "dark" ? "light" : "dark";
  applyTheme({ ...current, mode: next });
  writeNextThemeAttribute(next);
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
  // The full view of the one agent run this window holds — the transcript,
  // the composer, and the 312px rail. The console dock at the bottom of the
  // window is a second, compact renderer over the SAME run, not a screen of
  // its own.
  { route: "/agent", name: "Agent" },
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
  // The session rail and the live shell one of them is. Listed after forwards
  // because it shares their shape — a session outlives the tab that started
  // it, so this screen is where the ones already running are seen and ended.
  { route: "/terminals", name: "Terminals" },
  // The release table, the rendered diff between two revisions, and the four
  // operations. Listed with forwards and terminals because it shares their
  // shape: an upgrade or a rollback outlives the dialog that started it, and
  // the status strip counts the ones still running.
  { route: "/helm", name: "Helm" },
  // The only screen here that is about the machine rather than a cluster: the
  // managed kubectl, helm and krew under ~/.srelens/bin, and what the active
  // context's exec-auth needs on PATH.
  { route: "/toolbox", name: "Toolbox" },
  // Every cluster srelens can see at once, rather than one of them: the file
  // each context was read from, which credential it uses, and what the last
  // reachability probe said. Reached from the cluster rail's
  // `Connection details`.
  { route: "/connections", name: "Connections" },
  // The first-run door, and the only screen here a reader can be on with no
  // cluster connected at all. Listed after connections because that is where
  // both ways in are — the `Add connection` control and the empty state's own.
  { route: "/connect", name: "Connect a cluster" },
  // srelens itself rather than a cluster or the machine: the six panes behind
  // §23's nav rail — Agent & MCP, Security, Appearance, Accessibility,
  // Shortcuts and Clusters. Listed last because it is the newest, and worth
  // naming here in particular: the new design's own Appearance pane is where
  // this toggle lives on that side, so a reader weighing the switch is
  // weighing whether they can find their way back.
  { route: "/settings", name: "Settings" },
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
