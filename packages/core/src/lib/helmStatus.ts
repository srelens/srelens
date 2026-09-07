/**
 * A Helm release's status word and tone.
 *
 * Follows `packages/core/src/lib/k8sStatus.ts`'s shape, but for a narrower
 * reason: a Kubernetes resource's status is srelens's own reading of raw
 * fields, so that file invents words ("Degraded", "Not scheduled") and pairs
 * them with a tone it decides. A Helm release's status is not srelens's to
 * invent — `info.status` in the release Secret is Helm's own vocabulary, and
 * `crates/kube/src/helm.rs` passes it straight through with no enum in
 * between. This module never renames it; it only tones it.
 *
 * Helm's documented statuses are `deployed`, `failed`, `pending-install`,
 * `pending-upgrade`, `pending-rollback`, `superseded`, `uninstalling`,
 * `uninstalled`. That set is Helm's to extend, not srelens's to close: a
 * future Helm release could add a ninth word to that Secret tomorrow, and
 * this module must not pretend to recognise one it does not.
 */
import type { HealthKind } from "./k8sHealth";

/** A release's status word and its tone. */
export interface HelmVerdict {
  /** Exactly the string Helm gave — never renamed, never paraphrased. */
  word: string;
  /** Tone for the word. */
  health: HealthKind;
}

/**
 * Tone for each of Helm's eight documented statuses.
 *
 * `deployed` is the only state this reads as healthy, and `failed` the only
 * one it reads as broken. Everything else is a state in between, toned by
 * what it actually means rather than by a blanket "anything but deployed is
 * bad" rule:
 *
 * - The three `pending-*` states and `uninstalling` are a mutation in
 *   progress — not yet a success, not yet a failure. Reading them as
 *   `danger` would paint a normal `helm upgrade` mid-flight as broken.
 * - `superseded` means an older revision was replaced by a newer one — the
 *   ordinary shape of a release with history, not a problem with this one.
 * - `uninstalled` is a release that is simply gone, not a release that
 *   failed to be gone.
 *
 * A status not in this table is one this build has never heard of, and is
 * toned the same as those two: neutral. Not healthy — it might be a failure
 * this table has no name for — and not broken — it might be perfectly fine.
 * The same rule that leaves a missing metric reading "no reading" rather
 * than `0%`.
 */
const HELM_HEALTH: Record<string, HealthKind> = {
  deployed: "success",
  failed: "danger",
  "pending-install": "warning",
  "pending-upgrade": "warning",
  "pending-rollback": "warning",
  uninstalling: "warning",
  superseded: "neutral",
  uninstalled: "neutral",
};

/**
 * A release's status word and tone. `status` is returned verbatim as
 * `word` — this function never invents or paraphrases Helm's word — and
 * `health` comes from {@link HELM_HEALTH}; anything not in that table,
 * including the empty string, reads neutral rather than throwing.
 */
export function helmStatus(status: string): HelmVerdict {
  return { word: status, health: HELM_HEALTH[status] ?? "neutral" };
}
