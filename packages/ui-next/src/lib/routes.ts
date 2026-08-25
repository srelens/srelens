import type { ComponentType } from "react";
import { K8S_KIND, RESOURCE_LABELS, type ResourceKind } from "@srelens/core";
import { parseDetailRoute } from "./detailRoute";
import { AppLog } from "../screens/AppLog";
import { Events } from "../screens/Events";
import { Forwards } from "../screens/Forwards";
import { Logs, parseLogsRoute } from "../screens/Logs";
import { Overview } from "../screens/Overview";
import { ReleaseNotes } from "../screens/ReleaseNotes";
import { ResourceDetailScreen, Resources } from "../screens/Resources";
import { Toolbox } from "../screens/Toolbox";
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
 * Turn a route into what its tab shows. The cluster name is the real one,
 * passed in by whoever knows it; the mock hard-coded "prod-eu".
 */
export function describe(route: string, clusterName?: string): RouteInfo {
  const sub = clusterName || undefined;
  if (route.startsWith("/resources/")) {
    const [, , rawName, suffix] = route.split("/");
    const name = decodeURIComponent(rawName ?? "");
    // `/resources/<name>/logs|shell|forward` shares the bare
    // `/resources/<name>` prefix — so without this a pod opened three ways
    // ("Open in new tab", "Follow logs", "Open shell") got three tabs with the
    // identical title and kind, indistinguishable in the strip.
    //
    // Only `shell` is still minted (`ResourceMenu.tsx`). Logs and Port forward
    // have real front doors now — `logsRoute` and §A.4's dialog — and the
    // other two shapes survive here only so a tab a previous session
    // persisted can still name itself in the strip.
    if (suffix === "logs") return { route, title: `${name} · logs`, sub, kind: "logs" };
    if (suffix === "shell") return { route, title: `${name} · shell`, sub, kind: "terminal" };
    if (suffix === "forward") return { route, title: `${name} · forward`, sub, kind: "forwards" };
    return { route, title: name, sub, kind: "resource" };
  }
  if (route.startsWith("/edit/")) {
    return { route, title: `Edit ${decodeURIComponent(route.split("/")[2] ?? "")}`, sub, kind: "edit" };
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

export type ScreenComponent = ComponentType<{ route: string }>;

/**
 * The only place that knows which screens exist. Adding a screen is one entry
 * here and nothing else; a route with no entry renders the Placeholder.
 */
const SCREENS: Record<string, ScreenComponent> = Object.assign(Object.create(null), {
  "/applog": AppLog,
  "/notes": ReleaseNotes,
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
  // App-scoped: the managed kubectl, helm and krew are the machine's, and the
  // exec-auth rail is the only part of it that looks at a context at all.
  "/toolbox": Toolbox,
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
