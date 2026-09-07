import { loadRestoreSession, settingsStorage } from "@srelens/core";
import type { TableSort } from "@srelens/ui-kit";
import type { Tab, TabsState, Workspace } from "./tabs";

/**
 * Where the shell's tabs live between launches.
 *
 * Through `settingsStorage`, not `localStorage`: on the desktop that is the
 * backend's settings file, and on the web it is `localStorage` — one code
 * path, the same one classic's session restore uses. Storage is injectable so
 * this can be tested with a Map and no platform at all, the way core's
 * `listContexts` takes its invoker.
 */
export const STORAGE_KEY = "srelens.next.workspaces";
export const STORAGE_VERSION = 1;

export interface Storage {
  getItem(key: string): string | null;
  setItem(key: string, raw: string): void;
  removeItem(key: string): void;
}

const isRecord = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);
const isString = (v: unknown): v is string => typeof v === "string";

const isTableSort = (v: unknown): v is TableSort =>
  isRecord(v) && isString(v.key) && (v.direction === "asc" || v.direction === "desc");

/**
 * A resource list's sort, filter string and active filter column. Follows
 * the tab's own rule: a field this build cannot read is dropped on its own
 * rather than taking the rest of the object with it — a `sort` in a shape
 * this build does not recognise costs `sort`, not the `view` around it, and
 * a `view` that is not an object at all (a stray string, say) costs the
 * `view`, not the tab.
 */
function parseView(v: unknown): Tab["view"] | undefined {
  if (!isRecord(v)) return undefined;
  const view: NonNullable<Tab["view"]> = {};
  if (v.sort === null) view.sort = null;
  else if (isTableSort(v.sort)) view.sort = v.sort;
  if (isString(v.filter)) view.filter = v.filter;
  if (v.filterKey === null) view.filterKey = null;
  else if (isString(v.filterKey)) view.filterKey = v.filterKey;
  return view;
}

function parseTab(v: unknown): Tab | null {
  if (!isRecord(v) || !isString(v.id) || !isString(v.route) || !isString(v.title) || !isString(v.kind)) return null;
  const tab: Tab = { id: v.id, route: v.route, title: v.title, kind: v.kind as Tab["kind"] };
  if (isString(v.sub)) tab.sub = v.sub;
  if (v.preview === true) tab.preview = true;
  if (v.pinned === true) tab.pinned = true;
  const view = parseView(v.view);
  if (view !== undefined) tab.view = view;
  return tab;
}

function parseWorkspace(v: unknown): Workspace | null {
  if (!isRecord(v) || !isString(v.id) || !isString(v.name) || !isString(v.activeId)) return null;
  if (!Array.isArray(v.clusters) || !Array.isArray(v.tabs)) return null;
  const tabs = v.tabs.map(parseTab).filter((t): t is Tab => t !== null);
  const closed = Array.isArray(v.closed) ? v.closed.map(parseTab).filter((t): t is Tab => t !== null) : [];
  const clusters = v.clusters.filter(isString);
  const ws: Workspace = {
    id: v.id,
    name: v.name,
    clusters,
    tabs,
    activeId: v.activeId,
    closed,
  };
  // Only a cluster the workspace actually has: the field is an index into
  // `clusters`, and `reconcile` would drop a dangling one anyway.
  if (isString(v.activeCluster) && clusters.includes(v.activeCluster)) ws.activeCluster = v.activeCluster;
  return ws;
}

/**
 * A stored document, or null for anything this build cannot read.
 *
 * A document at or below `STORAGE_VERSION` must be *migrated* here, never
 * refused: refusing an older one is how a version bump silently discards
 * every existing user's workspaces, and there is nothing to bump to if the
 * only way forward throws the data away. There is one version so far, so
 * there is nothing to migrate yet; the next bump adds the step here, between
 * this check and the parse below.
 *
 * A document from a *future* version is refused whole rather than half-read:
 * applying the fields we recognise and dropping the rest would leave the user
 * with some of their tabs and no idea which. So is one with no numeric
 * version at all, which is not a document this code ever wrote.
 *
 * Unknown fields are dropped; malformed tabs are dropped individually, since
 * one bad tab is not a reason to lose the workspace around it.
 */
export function parseStoredState(raw: string | null): TabsState | null {
  if (!raw) return null;
  let doc: unknown;
  try {
    doc = JSON.parse(raw);
  } catch {
    return null;
  }
  if (!isRecord(doc) || typeof doc.version !== "number" || doc.version > STORAGE_VERSION) return null;
  if (!Array.isArray(doc.workspaces) || !isString(doc.currentId)) return null;
  const workspaces = doc.workspaces.map(parseWorkspace).filter((w): w is Workspace => w !== null);
  return { workspaces, currentId: doc.currentId };
}

/**
 * Both accessors are guarded, the way every sibling helper in core guards its
 * own: `settingsStorage` falls back to raw `localStorage` when the backend
 * file is unavailable, and `localStorage` throws outright in a WebView with
 * storage disabled. An unguarded read rejected the Window's boot, so
 * `setBooted(true)` never ran and the spinner stayed up forever; an unguarded
 * write escaped a `setTimeout` and the `beforeunload` listener, where nothing
 * can catch it.
 */
export function loadTabsState(storage: Storage = settingsStorage, restore: () => boolean = loadRestoreSession): TabsState | null {
  if (!restore()) return null;
  try {
    return parseStoredState(storage.getItem(STORAGE_KEY));
  } catch (error) {
    console.error("could not read the saved workspaces", error);
    return null;
  }
}

export function saveTabsState(state: TabsState, storage: Storage = settingsStorage): void {
  try {
    storage.setItem(STORAGE_KEY, JSON.stringify({ version: STORAGE_VERSION, ...state }));
  } catch (error) {
    // Best-effort, as `settingsStorage` itself is: losing the tabs of a
    // session is a great deal better than losing the session.
    console.error("could not persist the workspaces", error);
  }
}

let pending: { state: TabsState; storage: Storage } | null = null;
let timer: ReturnType<typeof setTimeout> | null = null;

/** Save soon, coalescing a burst of changes into one write of the latest. */
export function scheduleSave(state: TabsState, storage: Storage = settingsStorage, delayMs = 300): void {
  pending = { state, storage };
  if (timer) clearTimeout(timer);
  timer = setTimeout(flushSave, delayMs);
}

/** Write whatever is pending now. Safe to call with nothing pending. */
export function flushSave(): void {
  if (timer) {
    clearTimeout(timer);
    timer = null;
  }
  if (!pending) return;
  const { state, storage } = pending;
  pending = null;
  saveTabsState(state, storage);
}

/** A debounced save must not lose the last change to a window closing. */
export function installFlushOnUnload(target: Window = window): () => void {
  target.addEventListener("beforeunload", flushSave);
  return () => target.removeEventListener("beforeunload", flushSave);
}
