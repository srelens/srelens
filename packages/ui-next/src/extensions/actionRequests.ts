import type { ExtensionResourceSelection } from "@srelens/core";

/**
 * A palette command asking an app resource view to open ITS confirmation for
 * one declared action (#544).
 *
 * The palette never runs the action. It names the resource and the action, and
 * the app resource inspector — which already owns the one host confirmation
 * (#552) for that resource, with the UID and resourceVersion it read — opens
 * the same review its own button opens. That keeps there being exactly one
 * confirmation for the write, in the tab that shows the resource.
 *
 * Held as well as announced: the palette may have to open the resource's tab
 * first, and a view that mounts after the request takes it on mount. A request
 * nobody takes lapses after a few seconds, so opening that resource much later
 * does not surprise the reader with a review they did not ask for then.
 */
export interface ExtensionActionRequest {
  id: string;
  capability: string;
  context: string;
  namespace: string;
  name: string;
  action: string;
}

const LAPSE_MS = 10_000;
let held: { request: ExtensionActionRequest; at: number } | null = null;
const listeners = new Set<() => void>();

export function requestExtensionAction(request: ExtensionActionRequest) {
  held = { request, at: Date.now() };
  for (const listener of [...listeners]) listener();
}

/** The action requested for this view's resource, once; `null` when there is none. */
export function takeExtensionAction(selection: ExtensionResourceSelection): string | null {
  if (!held) return null;
  if (Date.now() - held.at > LAPSE_MS) {
    held = null;
    return null;
  }
  const { request } = held;
  const same = request.id === selection.id && request.capability === selection.capability
    && request.context === selection.context && request.namespace === selection.namespace
    && request.name === selection.name;
  if (!same) return null;
  held = null;
  return request.action;
}

export function onExtensionActionRequested(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}
