/**
 * A log stream's connection state, turned into a word and a tone — the ONE
 * place that does this, so a screen never hand-pairs one of these four states
 * with a colour itself.
 *
 * Core has no other vocabulary for this: `k8sStatus.ts` and `k8sHealth.ts`
 * cover Kubernetes *resources* — a Pod's phase, a Node's readiness — and a
 * stream's connection is not one. `startLogStream` (`./logsStream`) only ever
 * reports `LogStatus` ("live" | "reconnecting") once a connection exists;
 * "connecting" (before the first status arrives) and "error" (the stream
 * gave up) are states only the caller that opened the stream can observe, so
 * this module's own union names all four rather than importing a narrower
 * one and widening it ad hoc.
 *
 * Shaped after `StatusVerdict` in `k8sStatus.ts`: the word and the tone are
 * decided together, by name, from a small set of named constants — not
 * assembled at each call site — so a copy-paste cannot pair "Following" with
 * the warning tone. The `switch` below is exhaustive on
 * {@link LogConnectionStatus} with no `default` fallthrough baked into the
 * return type, so a fifth state added to the union without a matching case
 * is a compile error here, not a runtime `undefined`.
 */
import type { HealthKind } from "./k8sHealth";
import type { LogStatus } from "./logsStream";

/** The whole state space a log stream's connection can be in. */
export type LogConnectionStatus = "connecting" | LogStatus | "error";

/** A connection state's word, and the tone core says it should carry. */
export interface LogConnectionVerdict {
  readonly label: string;
  readonly health: HealthKind;
}

const CONNECTING: LogConnectionVerdict = { label: "Connecting", health: "info" };
/** "Following", not "Live": this is the word a reader watches scroll. */
const LIVE: LogConnectionVerdict = { label: "Following", health: "success" };
const RECONNECTING: LogConnectionVerdict = { label: "Reconnecting", health: "warning" };
/** Not "Error": the connection failed, not any one line in the buffer. */
const STREAM_ERROR: LogConnectionVerdict = { label: "Stream stopped", health: "danger" };

/** {@link LogConnectionStatus} → its word and tone. See the module doc. */
export function logConnectionStatus(status: LogConnectionStatus): LogConnectionVerdict {
  switch (status) {
    case "connecting":
      return CONNECTING;
    case "live":
      return LIVE;
    case "reconnecting":
      return RECONNECTING;
    case "error":
      return STREAM_ERROR;
    default: {
      // Exhaustiveness guard: a status literal added to the union without a
      // case above fails to compile here, rather than falling through to a
      // verdict nobody chose for it.
      const exhaustive: never = status;
      return exhaustive;
    }
  }
}
