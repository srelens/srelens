import type { ComponentType } from "react";
import { K8S_KIND, RESOURCE_LABELS, type ResourceKind } from "@srelens/core";
import { parseDetailRoute, parseEditRoute } from "./detailRoute";
import { Agent } from "../screens/Agent";
import { AppLog } from "../screens/AppLog";
import { Connect } from "../screens/Connect";
import { Connections } from "../screens/Connections";
import { Events } from "../screens/Events";
import { Forwards } from "../screens/Forwards";
import { Helm } from "../screens/Helm";
import { Logs, parseLogsRoute } from "../screens/Logs";
import { Overview } from "../screens/Overview";
import { ReleaseNotes } from "../screens/ReleaseNotes";
import { ResourceDetailScreen, Resources } from "../screens/Resources";
import { Settings } from "../screens/Settings";
import { Terminals } from "../screens/Terminals";
import { Toolbox } from "../screens/Toolbox";
import { Topology } from "../screens/Topology";
import { Workloads } from "../screens/Workloads";

/**
 * What a tab is about, for the strip's icon and for the context menu. The
 * mock's vocabulary, minus `components`, which is the gallery and not a tab.
 */
export type TabKind =
  | "control" | "incidents" | "agent" | "workloads" | "resource"
  | "logs" | "terminal" | "forwards" | "helm" | "toolbox" | "settings" | "connect"
  | "topology" | "connections" | "edit" | "events" | "applog" | "notes";

export interface RouteInfo {
  route: string;
  title: string;
  /** The cluster, for cluster-scoped routes. Absent for app-scoped ones. */
  sub?: string;
  kind: TabKind;
  pinned?: boolean;
}

/**
 * A `/k/<slug>` route is a built-in list when the slug is one of core's kinds.
 * `overview` is excluded: core lists it as a kind for the classic sidebar, but
 * it has no Kubernetes kind behind it (`K8S_KIND.overview === ""`) and lives at
 * `/overview` here.
 */
export function isBuiltInKind(slug: string): slug is ResourceKind {
  return slug !== "overview" && Object.prototype.hasOwnProperty.call(K8S_KIND, slug);
}

/**
 * Routes whose tab carries no cluster in its sub.
 *
 * Null-prototype, like every lookup below it: a route is a string that can
 * arrive from a persisted session or a resource name, and on a plain object
 * literal `APP_SCOPED["constructor"]` is `Object` — truthy, so the tab came
 * back with no title at all.
 */
const APP_SCOPED: Record<string, Omit<RouteInfo, "route" | "sub">> =
  Object.assign(Object.create(null), {
    "/applog": { title: "Application log", kind: "applog" },
    "/notes": { title: "Release notes", kind: "notes" },
    "/settings": { title: "Settings", kind: "settings" },
    "/connections": { title: "Connections", kind: "connections" },
    "/connect": { title: "Connect a cluster", kind: "connect" },
    "/toolbox": { title: "Toolbox", kind: "toolbox" },
  });

/** Routes whose tab names the cluster it is looking at. */
const CLUSTER_SCOPED: Record<string, Omit<RouteInfo, "route" | "sub">> =
  Object.assign(Object.create(null), {
    "/": { title: "Control room", kind: "control", pinned: true },
    "/incidents": { title: "Incidents", kind: "incidents" },
    "/agent": { title: "Agent", kind: "agent" },
    "/resources": { title: "Workloads", kind: "workloads" },
    "/logs": { title: "Logs", kind: "logs" },
    "/terminals": { title: "Shell", kind: "terminal" },
    "/forwards": { title: "Port forwards", kind: "forwards" },
    "/helm": { title: "Helm", kind: "helm" },
    "/topology": { title: "Topology", kind: "topology" },
    "/new": { title: "New resource", kind: "edit" },
    "/events": { title: "Events", kind: "events" },
    "/overview": { title: "Cluster overview", kind: "control" },
  });

/**
 * One route segment, decoded — or left exactly as it arrived when it will not
 * decode.
 *
 * `decodeURIComponent` THROWS a `URIError` on a malformed escape (`%zz`, a
 * truncated multi-byte sequence), and {@link describe} runs DURING RENDER —
 * `shell/Placeholder` calls it — over routes restored from storage, which
 * `parseTab` accepts as any string at all. So one corrupted or legacy entry is
 * not a bad tab: it is the whole new-design window failing to boot, on every
 * launch, with nothing on screen to say why.
 *
 * `parseLogsRoute` and `parseDetailRoute` answer the same hazard with `null`,
 * which is their contract for a route they cannot make a subject of.
 * `describe` has no such answer — its contract is that a tab is ALWAYS named —
 * so it needs a fallback rather than a refusal, and the raw segment is it: it
 * is what storage actually holds, it is what the last-resort branch at the end
 * of `describe` already titles from, and it keeps two undecodable tabs
 * distinguishable in the strip where an invented word would collapse them.
 * `hostOf` in `screens/connections/clusterText.ts` makes the same call for the
 * same reason — an unparseable server prints verbatim.
 */
function decodedSegment(raw: string): string {
  try {
    return decodeURIComponent(raw);
  } catch {
    return raw;
  }
}

/**
 * Turn a route into what its tab shows. The cluster name is the real one,
 * passed in by whoever knows it; the mock hard-coded "prod-eu".
 */
export function describe(route: string, clusterName?: string): RouteInfo {
  const sub = clusterName || undefined;
  if (route.startsWith("/resources/")) {
    const [, , rawName, suffix] = route.split("/");
    const name = decodedSegment(rawName ?? "");
    // `/resources/<name>/logs|shell|forward` shares the bare
    // `/resources/<name>` prefix — so without this a pod opened three ways
    // ("Open in new tab", "Follow logs", "Open shell") got three tabs with the
    // identical title and kind, indistinguishable in the strip.
    //
    // None of the three is minted any more. Logs and Port forward have real
    // front doors — `logsRoute` and §A.4's dialog — and `Open shell` starts a
    // session and opens `/terminals` on it, so all three shapes survive here
    // only so a tab a previous session persisted can still name itself in the
    // strip.
    if (suffix === "logs") return { route, title: `${name} · logs`, sub, kind: "logs" };
    if (suffix === "shell") return { route, title: `${name} · shell`, sub, kind: "terminal" };
    if (suffix === "forward") return { route, title: `${name} · forward`, sub, kind: "forwards" };
    return { route, title: name, sub, kind: "resource" };
  }
  // `/edit/<kind>/<namespace>/<name>` — five segments, parsed rather than
  // prefix-split for the reason `parseEditRoute` gives: a decoded name can
  // contain a `/`, so only the segment count tells the shape.
  //
  // The NAMESPACE is in the title, not in `sub`. `openTab` dedupes by route, so
  // the route is what stops `default/api` and `staging/api` collapsing onto one
  // tab — but the strip then has to say which is which, and `sub` cannot be
  // where that goes: it is the CLUSTER on every other tab in the strip, so a
  // namespace sitting there would read as one. This is the one place a resource
  // tab's title carries more than the bare name, and it carries it because two
  // of these tabs are open at once far more often than two detail tabs are.
  const edit = parseEditRoute(route);
  if (edit) {
    const where = edit.namespace === null ? edit.name : `${edit.namespace}/${edit.name}`;
    return { route, title: `Edit ${where}`, sub, kind: "edit" };
  }
  // The legacy one-segment `/edit/<name>`. Nothing mints it any more; the shape
  // survives here only so a tab a previous session persisted can still name
  // itself in the strip, exactly as `/resources/<name>/logs|shell|forward` does
  // above.
  if (route.startsWith("/edit/")) {
    return { route, title: `Edit ${decodedSegment(route.split("/")[2] ?? "")}`, sub, kind: "edit" };
  }
  // `/logs/<kind>/<namespace>/<name>` — five segments, matched by none of the
  // exact tables below, so without this it fell to the last-resort branch and
  // titled itself after the raw path under the control-room icon. The subject
  // IS the tab's identity here (`openTab` dedupes by route), so a strip with
  // two streams open has to say which is which. Parsed rather than
  // prefix-matched, for the reason `parseLogsRoute` gives: a decoded name can
  // contain a `/`, so only the segment count tells the shape.
  const logs = parseLogsRoute(route);
  if (logs) return { route, title: `${logs.name} · logs`, sub, kind: "logs" };
  // A detail route (`/k/<kind>/<namespace>/<name>`) shares the `/k/` prefix
  // with a LIST route (`/k/<slug>`) — this must run before the list branch
  // below, or a detail route would fall into it and title itself after the
  // raw "<kind>/<namespace>/<name>" slug instead of the resource's own name.
  const detail = parseDetailRoute(route);
  if (detail) return { route, title: detail.name, sub, kind: "resource" };
  if (route.startsWith("/k/")) {
    const slug = route.slice(3);
    const title = isBuiltInKind(slug) ? RESOURCE_LABELS[slug] : slug;
    return { route, title, sub, kind: "workloads" };
  }
  const app = APP_SCOPED[route];
  if (app) return { route, ...app };
  const cluster = CLUSTER_SCOPED[route];
  if (cluster) return { route, ...cluster, sub };
  return { route, title: route.replace(/^\//, "") || "Untitled", sub, kind: "control" };
}

/**
 * What every routed screen is handed.
 *
 * `route` is the screen's own. `ported` and `onSwitchToClassic` are the HOST's,
 * injected from the root — and they are here rather than imported because
 * ui-next **cannot** import them. `onLocked` is neither: it is this package's
 * own, `shell/LockGate`'s `lockWorkspace` handed down by `Window`, and it is
 * declared beside the other two because it travels the same path. Three fields
 * now, two of them the host's; the sentence said "the other two are the HOST's"
 * when there were only two, and kept saying it once there were three. This package depends on `@srelens/core` and `@srelens/ui-kit`;
 * `apps/desktop` depends on this package, so reaching `apps/desktop/src/design`
 * from here is a genuine cycle across a package boundary with no alias to
 * shortcut it.
 *
 * The path already exists, and this widening only extends it by one hop:
 * `main.tsx` passes `PORTED_SCREENS.map((s) => s.name)` and a closure over
 * `switchDesign("classic")` into `NextApp`, which passes them to `Window`,
 * which passes them to `Body` — where `Placeholder` has consumed exactly these
 * two since #305. `Body` now hands the same pair to the screen it renders
 * instead of only to the Placeholder, so there is one injection path in this
 * package and not two.
 *
 * **Required, not optional, on purpose** — all three. `Settings` renders the Appearance
 * pane, whose `Design` panel is the only way back to the classic design; an
 * optional prop with a default would have let `Body` drop it and left that
 * panel listing nothing behind an inert button. Required means the typecheck
 * fails at the call site instead. Screens that want nothing but their route
 * still declare `{ route: string }` and are still assignable here — a wider
 * props type is not a demand on the fourteen entries that ignore it.
 *
 * `ported` and `onSwitchToClassic` are step-11 scaffolding and leave with it:
 * when migration step 11 deletes `apps/desktop/src/design.ts` and
 * `shell/Placeholder.tsx`, those two fields and `Body`'s forwarding of them go
 * too. `onLocked` stays — the lock surface is not scaffolding — so this becomes
 * `{ route, onLocked }` rather than `{ route }`.
 */
export interface RoutedScreenProps {
  route: string;
  /** Display names of the screens that exist in the new design. */
  ported: readonly string[];
  /** Leave the new design, from the route the reader is on. */
  onSwitchToClassic: () => void;
  /**
   * Raise the lock surface over the whole window.
   *
   * The third field, and the only one that is NOT step-11 scaffolding: this one
   * stays. `Settings`'s Security pane is the one screen that calls it, after
   * `vaultLock()` has resolved, and what it needs raised is `shell/LockGate` —
   * which is mounted above the tab strip and the cluster rail, because since
   * PR #365 anything mounted inside a tab covers only that tab. A lock that did
   * that would leave the rail and every other tab live over a sealed vault,
   * which is worse than no lock because the window would look sealed.
   *
   * **Zero-argument, synchronous, non-throwing, fire-and-forget, idempotent.**
   * A caller must be able to invoke it from a click handler and move on: no
   * promise to await, nothing to catch, and `Lock now` may be double-clicked.
   * It reads no vault state before covering — every await between the vault
   * being sealed and the window being covered is a window of live UI over a
   * sealed vault.
   *
   * **Required, for the reason the two above it are.** An optional handler with
   * a default is how a screen ends up drawing a lock button behind nothing;
   * required means the typecheck fails at the call site instead.
   */
  onLocked: () => void;
}

export type ScreenComponent = ComponentType<RoutedScreenProps>;

/**
 * The only place that knows which screens exist. Adding a screen is one entry
 * here and nothing else; a route with no entry renders the Placeholder.
 */
const SCREENS: Record<string, ScreenComponent> = Object.assign(Object.create(null), {
  "/applog": AppLog,
  "/notes": ReleaseNotes,
  // The full view of the one agent run this window holds — the console dock
  // (`shell/Console.tsx`) is a second, compact renderer over the SAME store,
  // never a second conversation.
  "/agent": Agent,
  "/resources": Workloads,
  "/events": Events,
  "/overview": Overview,
  // The bare route. Its deeper `/logs/<kind>/<namespace>/<name>` shape reaches
  // the same screen through `parseLogsRoute` in `screenFor` — one screen, two
  // shapes, because telling them apart is the screen's own job.
  "/logs": Logs,
  // Cluster-scoped in the strip, but the screen lists every tunnel this
  // process holds — the store behind it is module-level and does not partition
  // by context, and a forward outlives the tab that started it.
  "/forwards": Forwards,
  // Cluster-scoped in the strip for the same reason as `/forwards` and with
  // the same caveat: the store behind it is module-level, a session outlives
  // the tab that opened it, and the rail lists every one this process holds.
  // The route is reached from the strip, from the status bar's live-session
  // count, and from the resource row menu's `Open shell` — which starts the
  // session first and opens this tab on it, so without this entry that action
  // left a live PTY running behind a Placeholder.
  "/terminals": Terminals,
  // Cluster-scoped, and the last of the three screens whose work outlives the
  // tab: an upgrade or a rollback started here keeps running in `helmOps`
  // after its dialog closes, and the status strip counts it from there. The
  // sidebar's Helm node points at this route and nothing else does.
  "/helm": Helm,
  // App-scoped: the managed kubectl, helm and krew are the machine's, and the
  // exec-auth rail is the only part of it that looks at a context at all.
  // Cluster-scoped: how traffic reaches a workload in ONE namespace, which is
  // why the screen carries a namespace picker rather than reading the tab.
  // Four lanes, all of them joins the cluster can prove — see
  // crates/kube/src/topology.rs for the three the design has that it cannot.
  "/topology": Topology,
  "/toolbox": Toolbox,
  // App-scoped, and about every cluster at once rather than one: which contexts
  // srelens can see, the file each was read from, and what the last probe said.
  // Reached from the cluster rail's `Connection details`, which has pointed
  // here since the rail was built — without this entry that menu item opened a
  // correctly titled tab onto the Placeholder.
  "/connections": Connections,
  // The first-run door, and the only screen a reader can be on with no cluster
  // connected at all. Reached from `/connections` twice over — the
  // `Add connection` control and the empty state's own — and both of those were
  // built and tested against a route that rendered the Placeholder, which is
  // the seam the note in `Connections.tsx` left for this entry.
  "/connect": Connect,
  // App-scoped, and the one screen here that is about srelens itself: the six
  // panes behind §23's 196px nav rail. Reached from the titlebar's gear, which
  // has opened this route since the chrome was built — so without this entry
  // that button opened a correctly titled tab onto the Placeholder, and the
  // Appearance pane's design toggle, the Security pane's `Lock now` and the
  // MCP token were all unreachable in the design that owns them.
  "/settings": Settings,
});

/**
 * Routes matched by prefix rather than by name, in order. Kept beside the
 * exact table rather than folded into it: a prefix table is a different kind of
 * claim — "everything under here" — and reading it as a list makes the reach of
 * each entry obvious. One screen answers all 34 built-in kinds and every CRD a
 * cluster has, so enumerating them here would be a second list to keep in step
 * with the sidebar's.
 */
const PREFIXED: ReadonlyArray<[string, ScreenComponent]> = [["/k/", Resources]];

export function screenFor(route: string): ScreenComponent | null {
  // `hasOwnProperty.call` as well as the null prototype: the table is the one
  // thing standing between an arbitrary route string and something rendered as
  // a component, and it costs nothing to say so twice.
  if (Object.prototype.hasOwnProperty.call(SCREENS, route)) return SCREENS[route];
  // A detail route (`/k/<kind>/<namespace>/<name>`, five segments) shares its
  // `/k/` prefix with a LIST route (`/k/<slug>`, three) — the more specific
  // match must win here, ahead of the prefix loop below, or a detail route
  // would render the LIST screen (`Resources`) as if the resource's own name
  // were just another kind slug. Matched by parse rather than by adding a
  // second `/k/` entry to `PREFIXED`, which cannot tell the two apart at all.
  if (parseDetailRoute(route)) return ResourceDetailScreen;
  // A logs subject route, for the same reason and by the same means: matched
  // by parse rather than by a `/logs/` prefix entry, so `/logs/` on its own —
  // and anything else under the prefix that is not a whole subject — stays a
  // Placeholder instead of rendering a stream with no subject in it.
  if (parseLogsRoute(route)) return Logs;
  // `/k/events` shares its `/k/` prefix with every other built-in list route
  // below, and `events` IS in core's `K8S_KIND` (so `isBuiltInKind` is true) —
  // but `descriptors.ts`'s `TYPED` table has no `events` entry, so the prefix
  // loop's `Resources` would fall to the generic Name/Namespace/Age
  // descriptor, silently losing Type, Reason, Object, Message and Count. This
  // must be checked ahead of that loop, or the sidebar's Events node — which
  // already points at `/events`, not `/k/events` — would still leave `/k/events`
  // reachable only by typing it or restoring an old session, as a trap.
  if (route === "/k/events") return Events;
  for (const [prefix, screen] of PREFIXED) {
    // A bare prefix names no resource; `/k/` is not a route.
    if (route.startsWith(prefix) && route.length > prefix.length) return screen;
  }
  return null;
}
