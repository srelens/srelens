/**
 * What the console dock offers to ask, and what it says it is asking about —
 * both derived from the active route, and both pure: no React, no store, no
 * I/O. This is the part of the dock that can be reasoned about without
 * mounting anything.
 *
 * Design: `docs/superpowers/specs/mock-full-design.md` §F (the agent dock).
 */
import { parseDetailRoute } from "./detailRoute";
import { describe } from "./routes";

/** §F's five suggestion sets, verbatim — this is copy, not paraphrase. */
const LOGS_SUGGESTIONS = [
  "Summarise the last 500 lines",
  "Which trace ids failed?",
  "Group these errors by cause",
] as const;

const RESOURCE_SUGGESTIONS = [
  "Why is this workload degraded?",
  "What changed in the last hour?",
  "Compare rev 119 with rev 118",
] as const;

const HELM_SUGGESTIONS = [
  "What did release 119 change?",
  "Roll back checkout to 118",
  "Which releases drift from git?",
] as const;

const INCIDENTS_SUGGESTIONS = [
  "Why is checkout-api returning 5xx?",
  "What changed in prod-eu today?",
  "Is any other service affected?",
] as const;

/**
 * The "anything else" set — a missing reading is absent, never a placeholder,
 * so every route gets a real set of suggestions and this is what an unmatched
 * route falls back to.
 */
const FALLBACK_SUGGESTIONS = [
  "What is unhealthy right now?",
  "Show me pods restarting today",
  "Explain this screen",
] as const;

/**
 * §F's route-aware `Start here` prompts. Matched by route shape, in the order
 * §F lists them, with the fallback set always last.
 *
 * The resource set is matched via `describe(route).kind === "resource"` rather
 * than a `/resources/` prefix or `parseDetailRoute` alone: the mock predates
 * `detailRoute`'s `/k/<kind>/<ns>/<name>` shape, and a real resource tab's
 * route never starts with `/resources/` any more — but a tab a previous
 * session persisted still can (`describe`'s own legacy branch, `routes.ts`
 * around line 133), and it is just as much a resource as the modern shape.
 * `describe` already tells the two `/resources` shapes apart from the
 * WORKLOADS LIST at that exact route (`kind: "workloads"`, not `"resource"`),
 * so reusing its `kind` here catches both resource shapes without also
 * catching the list.
 */
export function suggestionsFor(route: string): readonly string[] {
  if (route.startsWith("/logs")) return LOGS_SUGGESTIONS;
  if (describe(route).kind === "resource") return RESOURCE_SUGGESTIONS;
  if (route.startsWith("/helm")) return HELM_SUGGESTIONS;
  if (route === "/incidents" || route === "/") return INCIDENTS_SUGGESTIONS;
  return FALLBACK_SUGGESTIONS;
}

/**
 * Join a cluster name and a subject with `" / "`, but only when BOTH halves
 * have something in them.
 *
 * Joining unconditionally produces a bare separator on either side — an empty
 * cluster gives `" / helm"`, and a route with no subject (the control room)
 * would give `"prod-eu / "`. On #380 the second shape reached a reader as
 * dialog copy reading *"This still runs against , not prod-eu"* — a missing
 * half surfacing as a literal blank rather than being dropped. Returning
 * whichever half is non-empty (or `""` if neither is) is what keeps that from
 * happening here.
 */
function join(cluster: string, subject: string): string {
  if (cluster && subject) return `${cluster} / ${subject}`;
  return cluster || subject;
}

/**
 * What the dock says it is asking about — the placeholder's `<context>` in
 * §F's `Ask about <context>`.
 *
 * A detail route names its resource directly. Any other route falls back to
 * {@link describe}'s title, lowercased, for the screen — EXCEPT a `pinned`
 * route (today, only `/`, the control room): that screen already represents
 * the whole cluster, so it contributes no subject at all rather than the
 * redundant `"control room"`. That is what leaves `contextLabelFor("/", "prod-eu")`
 * as bare `"prod-eu"` instead of `"prod-eu / control room"` — the same "half
 * can be empty" case {@link join} exists to handle without printing a bare
 * separator.
 */
export function contextLabelFor(route: string, clusterName: string): string {
  const detail = parseDetailRoute(route);
  if (detail) return join(clusterName, detail.name);
  const info = describe(route);
  const subject = info.pinned ? "" : info.title.toLowerCase();
  return join(clusterName, subject);
}
