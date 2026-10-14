import { createContext, useContext } from "react";

/**
 * The id of the tab a screen is mounted in, provided by `Window` around each
 * tab's body.
 *
 * A screen cannot find its own tab by asking which one is active: `Window`
 * mounts every tab's body and only hides the inactive ones, so "the active
 * tab" is somebody else's tab for every screen but one. State that belongs to
 * a tab — the namespace selection is the reason this exists — is read and
 * written through this id. `null` outside any tab (the dock, a test rendering
 * a screen on its own), where the active tab is the right answer.
 */
export const TabScope = createContext<string | null>(null);

export function useTabScope(): string | null {
  return useContext(TabScope);
}
