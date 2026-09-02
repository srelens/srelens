/**
 * A fact read once (typically at mount): still loading, refused, or landed.
 *
 * Kept generic and shared so every screen reading something once doesn't
 * reinvent it, and — the actual reason it exists — so "not read yet" can
 * never collapse onto the same value as "read, and it's empty". A `[]` or
 * `{}` used as the "not read yet" default is indistinguishable from a
 * genuinely empty answer, and a rejected promise left in that same default
 * is indistinguishable from either one: `RunsRail`'s `.catch(() =>
 * setSessions([]))` rendered "No recent runs yet." for a failed
 * `listSessions` exactly as it would for an account with none, and
 * `Composer`'s `listAgents` failure rendered "No agent is available … install
 * one to get started" for a backend it never actually asked. Both are
 * confident false statements standing in for silence.
 *
 * `AgentPane` introduced this shape first, for its own three independent
 * reads; lifted here rather than left there once `RunsRail` and `Composer`
 * needed the identical three states for the same reason.
 */
export type Read<T> = { kind: "loading" } | { kind: "error"; error: unknown } | { kind: "ready"; value: T };

export const LOADING: Read<never> = { kind: "loading" };
