import { invokeCommand, on } from "../transport/transport";
import { isTauri } from "../transport/platform";

/** A live port-forward: a local port piped to a Pod or Service. */
export interface ActiveForward {
  id: number;
  context: string;
  namespace: string;
  /** "Pod" or "Service". */
  kind: string;
  name: string;
  remotePort: number;
  localPort: number;
}

export interface ForwardRequest {
  context: string;
  namespace: string;
  kind: string;
  name: string;
  remotePort: number;
  /** Preferred local port; omitted/0 lets the OS pick a free one. */
  localPort?: number;
}

// Module-level store so active forwards survive component remounts and are
// shared between the per-resource "Forward" action and the status-bar list.
let forwards: ActiveForward[] = [];
const listeners = new Set<() => void>();
const closers = new Map<number, () => void>();

function emit() {
  for (const l of listeners) l();
}

/** Subscribe to store changes (for `useSyncExternalStore`). */
export function subscribeForwards(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** Current active forwards (stable reference until the next change). */
export function getForwards(): ActiveForward[] {
  return forwards;
}

/** Start a port-forward and track it; auto-removes if the backend loop ends. */
export async function startPortForward(req: ForwardRequest): Promise<ActiveForward> {
  const info = await invokeCommand<{ id: number; localPort: number }>("start_port_forward", {
    context: req.context,
    namespace: req.namespace,
    kind: req.kind,
    name: req.name,
    remotePort: req.remotePort,
    localPort: req.localPort ?? null,
  });
  const fwd: ActiveForward = { ...req, id: info.id, localPort: info.localPort };
  forwards = [...forwards, fwd];
  closers.set(
    info.id,
    on(`forward:closed:${info.id}`, () => removeForward(info.id)),
  );
  emit();
  return fwd;
}

/** Stop a forward and drop it from the store. */
export async function stopPortForward(id: number): Promise<void> {
  await invokeCommand("stop_port_forward", { id });
  removeForward(id);
}

/** Where a live port-forward is reachable from the current UI: the bound
 *  localhost port on desktop, or the same-origin `/pf/<id>/` reverse proxy on
 *  web (the container's loopback port isn't reachable from the browser). */
export function forwardUrl(info: { id: number; localPort: number }): string {
  return isTauri() ? `http://localhost:${info.localPort}` : `/pf/${info.id}/`;
}

/** The human-readable, copy-pasteable address of a live forward: the bound
 *  localhost port on desktop, or the absolute same-origin `/pf/<id>/` proxy URL
 *  on web (the container's loopback port isn't reachable from the browser). */
export function forwardAddress(info: { id: number; localPort: number }): string {
  return isTauri()
    ? `localhost:${info.localPort}`
    : `${window.location.origin}/pf/${info.id}/`;
}

function removeForward(id: number) {
  closers.get(id)?.();
  closers.delete(id);
  const next = forwards.filter((f) => f.id !== id);
  if (next.length !== forwards.length) {
    forwards = next;
    emit();
  }
}
