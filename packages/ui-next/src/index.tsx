import { useState, useSyncExternalStore } from "react";
import { Gallery } from "@srelens/ui-kit/gallery";
import { Window } from "./shell/Window";
import { ConsoleProvider } from "./console";

export { ConsoleProvider, useConsole, type ConsoleValue } from "./console";
/**
 * The boot half of the Appearance pane, re-exported for the host, and the one
 * question about the stored record that the host has to be able to ask.
 *
 * `apps/desktop/src/main.tsx` calls `applyStoredAppearance` beside its own
 * `applyNextDesignTheme()`, and passes `hasChosenTheme` INTO that function so
 * the OS-appearance listener it arms stands down once the reader has named a
 * theme — see `lib/appearance.ts` for why only the stored record can tell a
 * named theme from a derived one.
 *
 * Both have to come from the module the host already imports dynamically: a
 * static `@srelens/ui-next` import from the entry would drag this whole tree
 * into the entry chunk that a classic-design boot also downloads, and a second
 * dynamic `import()` before the `Promise.all` would serialise the two downloads
 * the comment there keeps parallel.
 */
export { applyStoredAppearance, hasChosenTheme } from "./screens/settings/AppearancePane";

function subscribeToHash(onChange: () => void): () => void {
  window.addEventListener("hashchange", onChange);
  return () => window.removeEventListener("hashchange", onChange);
}

/**
 * The current location hash, as state rather than as a render-time read.
 *
 * Reading `window.location.hash` while rendering subscribes to nothing, so the
 * browser fires `hashchange` and React never hears about it: navigating to
 * #gallery left the placeholder up, and navigating away left the gallery up,
 * until a reload. (#317 review)
 */
function useHash(): string {
  return useSyncExternalStore(
    subscribeToHash,
    () => window.location.hash,
    // No hash on a server render; the client picks it up on hydration.
    () => "",
  );
}

/**
 * The new design's root: the window, and nothing else.
 *
 * The screens arrive one at a time; until a route has one, its tab renders the
 * Placeholder — so the design is navigable from the first PR rather than being
 * a single "nothing here yet" page. This package having its own tree and its
 * own stylesheet is what proves the two designs never share a document.
 *
 * The component gallery lives here too, at #gallery: a developer surface rather
 * than a screen, so it is a hash and not a route. The way *in* is on the
 * Placeholder, because that is the screen every un-ported route renders.
 *
 * `onExit` is the way back to the classic design, and it is now reached from
 * two places rather than one. It shipped when `Settings` did not exist in this
 * tree, so without it someone who opted in had no route out of the app except
 * editing localStorage — which is why the Placeholder's "Open in classic" is
 * wired to it. `screens/Settings.tsx` exists now, and its Appearance pane's
 * `Design` panel is the way out a reader will actually look for; both go
 * through this one callback, so there is one exit and not two.
 */
export function NextApp({
  onExit,
  ported = [],
  onToggleTheme = () => {},
  controls = "none",
  brandMarkSrc,
}: {
  onExit: (route: string, context?: string) => Promise<string | null> | string | null;
  /** Display names of the screens that exist in the new design. */
  ported?: string[];
  onToggleTheme?: () => void;
  controls?: "macos" | "none";
  /**
   * A URL for srelens's own brand mark, drawn on the lock surface.
   *
   * Injected rather than imported, for the reason `ported` and
   * `onSwitchToClassic` are: the asset lives in `apps/desktop/src/assets`, this
   * package depends on `@srelens/core` and `@srelens/ui-kit` and nothing else,
   * and `apps/desktop` depends on THIS package — so an import the other way is
   * a cycle across a package boundary. A literal `/srelens-mark.svg` would
   * have been a host path hardcoded into a package that must not know the host,
   * and wrong for the kit's gallery or any other consumer.
   *
   * Optional, and its absence is drawn rather than crashed: `Mark` falls
   * through to initials when it has no image or the image will not load, which
   * is what its own comment calls "a state, not an error to report".
   */
  brandMarkSrc?: string;
}) {
  const [error, setError] = useState<string | null>(null);

  async function leave(route: string, context?: string) {
    // Rendered here rather than raised as a toast: the toast host lives in the
    // classic tree, so a failure on the way out would have been invisible and
    // this button would have looked inert. (#314 review)
    setError((await onExit(route, context)) ?? null);
  }

  // A hash rather than a route: this tree has no router yet, and the gallery is
  // a developer surface rather than a screen.
  const gallery = useHash() === "#gallery";

  return (
    // A flex column rather than the Window and the alert as siblings: `body` is
    // `overflow: hidden` and `#root` is `height: 100%`, so an alert next to an
    // `h-full` Window starts at the bottom edge and is clipped — in the DOM and
    // the a11y tree, and off screen. That is the silent failure #314 closed, so
    // the Window gets the room that is left and the alert keeps its own.
    <div className="flex h-full min-h-0 flex-col">
      {/*
        Hidden rather than unmounted while the gallery is up — the same
        keep-mounted principle as `TabSurface`. Returning the Gallery *instead*
        of the Window unmounted it, and coming back re-booted it: with session
        restore off, boot found nothing saved and wrote a fresh default state,
        so every tab of the session was gone; with it on, only what the 300ms
        debounce had already flushed came back. The `hidden` attribute also
        takes the whole column out of the a11y tree, so the gallery is what a
        screen reader — and `queryByRole` — sees.
      */}
      <div className="min-h-0 flex-1" hidden={gallery}>
        {/* The provider wraps the window, not the gallery: the gallery has no
            console, and the dock is the window's own bottom edge. */}
        <ConsoleProvider onToggleTheme={onToggleTheme}>
          <Window
            ported={ported}
            active={!gallery}
            controls={controls}
            brandMarkSrc={brandMarkSrc}
            onToggleTheme={onToggleTheme}
            onOpenInClassic={leave}
            onOpenGallery={() => {
              window.location.hash = "#gallery";
            }}
          />
        </ConsoleProvider>
      </div>
      {gallery && (
        <div className="min-h-0 flex-1">
          <Gallery />
        </div>
      )}
      {error && (
        <p role="alert" className="shrink-0 px-3 py-2 text-[0.75rem] text-[var(--sev)]">
          Could not switch design. {error}
        </p>
      )}
    </div>
  );
}
