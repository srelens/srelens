import { useState, useSyncExternalStore } from "react";
import { Gallery } from "@srelens/ui-kit/gallery";

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
 * The new design's root.
 *
 * A placeholder for now. This package exists so the design switch has a real
 * second tree to load, with its own stylesheet — which is what proves the two
 * designs never share a document. The shell and screens arrive in later steps.
 *
 * It carries its own way back to the classic design on purpose: Settings does
 * not exist here yet, so without this button someone who opts in would have no
 * route out of the app except editing localStorage.
 */
export function NextApp({ onExit }: { onExit: () => Promise<string | null> | string | null }) {
  const [error, setError] = useState<string | null>(null);

  async function leave() {
    // Rendered here rather than raised as a toast: the toast host lives in the
    // classic tree, so a failure on the way out would have been invisible and
    // this button would have looked inert. (#314 review)
    setError((await onExit()) ?? null);
  }

  // A hash rather than a route: this tree has no router yet, and the gallery is
  // a developer surface rather than a screen.
  if (useHash() === "#gallery") {
    return <Gallery />;
  }

  return (
    <main className="next-placeholder">
      <h1>The new design</h1>
      <p>
        Nothing is built here yet. You are seeing this because the new design is
        switched on in Settings — the screens are still being written.
      </p>
      <button type="button" onClick={() => void leave()}>
        Back to the classic design
      </button>
      {error && <p role="alert">Could not switch design. {error}</p>}
    </main>
  );
}
