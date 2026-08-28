import React from "react";
import { createRoot } from "react-dom/client";
import { applyPersistedTimeout } from "@srelens/core";
import { isTauri } from "@srelens/core/platform";
import { initializeSettingsStorage } from "@srelens/core";
// The service layer says what to notify; this decides how. Installed before
// render so a toast raised during startup is not dropped on the floor.
import { installToastNotifier } from "./ui/notifier";
import srelensMark from "./assets/srelens-mark.svg";
import {
  PORTED_SCREENS,
  applyNextDesignChrome,
  applyNextDesignTheme,
  drawsOwnChrome,
  loadDesign,
  saveHandoff,
  switchDesign,
  toggleNextDesignTheme,
} from "./design";

installToastNotifier();

// Re-apply the persisted request timeout to the backend, which resets to its
// default each launch. Fire-and-forget: it resolves well before the user picks
// a context and triggers the first capability call. Tauri-only: on web this
// would race an unauthenticated 401 before the login gate resolves.
const container = document.getElementById("root");
if (!container) throw new Error("Root element #root not found");

// The container is passed in rather than captured: `bootstrap` is a hoisted
// function declaration, so TypeScript cannot rely on the null check above
// having run before a call and keeps the type as HTMLElement | null inside.
async function bootstrap(root: HTMLElement): Promise<void> {
  await initializeSettingsStorage();
  if (isTauri()) void applyPersistedTimeout();
  // Both the stylesheet AND the tree are imported dynamically. Only the
  // stylesheet is not enough: ui/index.ts imports ui/styles.css, so a
  // statically imported AppGate drags that into the entry chunk, which
  // index.html then links unconditionally — and the classic design's CSS would
  // load underneath the new one. Verified against a real build.
  if (loadDesign() === "next") {
    // Before the stylesheet, so the first paint is already the right mode. Its
    // stop function is HELD, not discarded: this call cannot yet know whether
    // the reader has named one of the new design's five themes — that record
    // lives on the chunk below — so the OS-appearance listener it arms for a
    // reader on "system" is provisional, and gets swapped for a guarded one as
    // soon as the record is readable. (#373 review)
    const stopDerivedThemeFollower = applyNextDesignTheme();
    // The overlay titlebar goes on before anything renders, so the window is
    // never seen with doubled chrome. A rejection inside is swallowed there:
    // an undressed window beats a blank one.
    await applyNextDesignChrome();
    // Started together, awaited together: the stylesheet and the tree are
    // independent downloads, and awaiting one before requesting the other
    // serialised them. index.html links no stylesheet, so the window stays
    // blank until both land — that wait is the whole startup screen.
    const [, { NextApp, applyStoredAppearance, hasChosenTheme }] = await Promise.all([
      import("@srelens/ui-next/styles"),
      import("@srelens/ui-next"),
    ]);
    // The Appearance pane's boot half, and it must run AFTER
    // applyNextDesignTheme above: the theme, accent and density chosen on that
    // pane are the reader's last word, and this is the only thing that puts
    // them back on the root after a reload. It writes only the axes the stored
    // document actually carries, so a reader who has never opened the pane
    // keeps the light/dark the line above derived from classic's preference —
    // see `applyStoredAppearance` for why that distinction is load-bearing.
    // Taken off the chunk already being awaited rather than imported
    // statically, which would put the whole new tree in the entry chunk that a
    // classic boot downloads too, or awaited on its own, which would serialise
    // the stylesheet behind it. Still before `createRoot`, so nothing paints
    // under the wrong appearance.
    //
    // The provisional follower is stood down first. It writes this same
    // `data-theme`, and it knows only "dark" and bare light, so an OS change
    // later in the session rewrote a chosen Midnight to plain dark and deleted
    // a chosen Paper down to light — throwing the reader's choice away until
    // the next launch. (#373 review)
    stopDerivedThemeFollower();
    applyStoredAppearance();
    // Re-armed behind the reader's own record, which is the only thing that can
    // tell a named theme from a derived one: `data-theme="dark"` is both a
    // reading of the OS and the third of the five themes, and a bare root is
    // both "nothing read" and a chosen Light.
    //
    // Not simply dropped, because naming no theme is the ONLY way a reader has
    // of saying "follow the OS" — the pane offers no System entry — so someone
    // who has never opened it must still track their OS while the app is open.
    // The predicate is re-read on every change, so naming a theme later in this
    // session stands this listener down too.
    //
    // Its own stop is discarded deliberately: it is armed for as long as the
    // window lives, and there is no unmount here to hang a teardown on.
    applyNextDesignTheme(hasChosenTheme);
    createRoot(root).render(
      <NextApp
        ported={PORTED_SCREENS.map((s) => s.name)}
        // The lock surface draws srelens's own mark, and ui-next cannot reach
        // this asset: `apps/desktop` depends on that package, so the import
        // would be a cycle across the boundary — the same wall the design
        // toggle hit, answered the same way. The same file classic's landing
        // page and login screen import, so there is one brand asset and vite
        // fingerprints it once.
        brandMarkSrc={srelensMark}
        // The overlay keeps macOS's real traffic lights, so the painted set
        // would double them (found in smoke testing); the picture is only for
        // an Apple browser, which has no window chrome of its own in the page.
        controls={drawsOwnChrome() && !isTauri() ? "macos" : "none"}
        onToggleTheme={toggleNextDesignTheme}
        onExit={async (route, context) => {
          // Written before the switch, so classic knows where to reopen even
          // though the reload throws this document away.
          saveHandoff(route, context);
          const result = await switchDesign("classic");
          return result.ok ? null : result.reason;
        }}
      />,
    );
    return;
  }
  const [, { default: AppGate }, { TooltipProvider }] = await Promise.all([
    import("./styles/globals.css"),
    import("./AppGate"),
    import("./components/ui/tooltip"),
  ]);
  // One tooltip provider for the whole classic window: every tooltip shares
  // its open delay and, once one has shown, its neighbours open at once for a
  // moment (the "sweep across the toolbar" the native `title` never allowed,
  // #376). The delay values are the provider's defaults — see tooltip.tsx for
  // why. The next design is not wrapped: its kit carries its own tooltip.
  createRoot(root).render(
    <TooltipProvider>
      <AppGate />
    </TooltipProvider>,
  );
}

void bootstrap(container);
