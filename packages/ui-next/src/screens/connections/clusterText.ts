import type { ClusterContext } from "@srelens/core";
import type { BadgeTone } from "@srelens/ui-kit";
import type { Probe, ProbeState } from "../../lib/probe";

/**
 * The words the surfaces that draw a cluster have to agree on: §6's table
 * cells, the Sources rail's rows, and §24's first-run card.
 *
 * All of them started private in a screen. `joined`, `viaOf` and
 * `latencyLabel` came out of `ClusterTable.tsx`; {@link STATUS} came out of
 * `ClusterTable.tsx` and `Connect.tsx` at once, where the same three pairs had
 * been written twice. They live here because those screens render the same
 * facts about the same clusters — a local cluster's `Via`, its round trip, a
 * line assembled out of parts that may be missing, and what the last probe
 * said — and a second copy of any of them is how two panes six inches apart
 * start disagreeing about one reading. The latency formatter in particular:
 * its whole job is to never print `0 ms`, and a second formatter written
 * beside it is exactly how that rule gets lost.
 */

/**
 * The one separator on this screen, and the only place parts are joined.
 *
 * Every caller has parts that may be missing — a cluster with no region, a
 * local cluster with no detected provider, a file with no contexts to count in
 * use — so the filter is here rather than at each site. `· ·` and a line that
 * begins or ends with a separator tell a reader nothing and look broken, and
 * both are what an unconditional `parts.join(" · ")` produces.
 *
 * An empty string means there was nothing to say; callers render NOTHING for
 * it rather than an empty line.
 */
export function joined(parts: readonly (string | undefined)[]): string {
  return parts
    .map((part) => part?.trim() ?? "")
    .filter((part) => part.length > 0)
    .join(" · ");
}

/**
 * The host a cluster answers on, for a local cluster's `Via`.
 *
 * `new URL` rather than a regexp, and the raw string when it will not parse: a
 * `server` srelens cannot read is still what the kubeconfig says, and printing
 * it verbatim is more use to a reader than an invented "unknown". Unix-socket
 * and non-URL servers arrive here too.
 */
function hostOf(server: string): string {
  try {
    return new URL(server).host || server;
  } catch {
    return server;
  }
}

/**
 * §6's `Via`: what the cluster is actually reached THROUGH.
 *
 * For a kubeconfig context that is the file it was declared in — the column's
 * whole reason for existing, and the field decision 1 added. For a local
 * cluster the file is beside the point (kind writes one for you); what
 * identifies it is the tool that made it and the endpoint it listens on.
 */
export function viaOf(context: ClusterContext): string {
  return context.isLocal ? joined([context.provider, hostOf(context.server)]) : context.sourceFile;
}

/**
 * The round trip, or nothing.
 *
 * **Gated on the STATE as well as on the number.** `probe.ts` documents
 * `latencyMs` as absent unless the state is `reachable`, so the two gates
 * agree today — but this is what a reader trusts, and `0 ms` on a cluster
 * that never answered reads as "instant", which is the exact opposite of the
 * truth (spec decision 4). One gate would leave that one drift away; two make
 * it structural.
 *
 * A reading under half a millisecond is a real reading of a cluster on this
 * laptop, and it is NOT discarded — it is drawn as `<1 ms`, because rounding it
 * to `0 ms` would put on screen the one string this may never show.
 */
export function latencyLabel(probe: Probe): string | null {
  if (probe.state !== "reachable") return null;
  const ms = probe.latencyMs;
  if (typeof ms !== "number" || !Number.isFinite(ms) || ms < 0) return null;
  return ms < 0.5 ? "<1 ms" : `${Math.round(ms)} ms`;
}

/**
 * The three words a probe can put on a row, and the tone each is worth.
 *
 * **No cluster is ever `healthy` or `degraded`** (spec decision 3). §6's mock
 * tones `healthy`→ok and `degraded`→sev, and the spec refuses both:
 * `connectCluster` reports whether the API server answered, and calling that
 * answer a health verdict claims a check that never ran. What is drawn is the
 * reading itself.
 *
 * `unread` is the absence, NAMED as an absence. Not a third status word
 * ("pending", "idle") — those read as things the cluster is, and this is a
 * thing srelens has not done yet.
 *
 * **One table, and this is the only copy of it.** It was private in
 * `ClusterTable.tsx` and written again in `Connect.tsx`, because promoting it
 * would have meant editing a file under review in a parallel task — which is
 * what the note there said, along with where the table belonged. This project
 * has already taken ten hand-paired label/tone tables out of the screens that
 * held them; that pair was the eleventh. It sits beside {@link latencyLabel},
 * which the first-run card already imported for exactly this reason.
 */
export const STATUS: Record<ProbeState, { word: string; tone: BadgeTone }> = {
  reachable: { word: "reachable", tone: "ok" },
  unreachable: { word: "unreachable", tone: "sev" },
  unread: { word: "no reading", tone: "muted" },
};
