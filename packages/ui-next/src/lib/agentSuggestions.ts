/**
 * What the console dock says it is asking about, derived from the active route
 * — pure: no React, no store, no I/O.
 *
 * It also held §F's five sets of canned suggestions. Those were removed with
 * the surface that drew them ("remove question not needed, make the dock
 * clean"), and the sets went with it rather than sitting here unread.
 *
 * Design: `docs/superpowers/specs/mock-full-design.md` §F (the agent dock).
 */
import { parseDetailRoute } from "./detailRoute";
import { describe } from "./routes";

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
