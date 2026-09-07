/**
 * Node taints, as the Nodes list and the Node detail page read them.
 *
 * The renderers live in two designs — `apps/desktop` (classic) and
 * `packages/ui-next` — so every decision that could drift between them is
 * made once here: the order the taints read in, the text of a taint, the
 * per-effect tally, the badge's accessible name, and the number a Taints
 * column sorts on. Each design supplies only its own Badge and its own
 * tooltip host. (#426)
 */

/** One entry of a Node's `spec.taints`, mirroring `crates/kube`'s `NodeTaint`. */
export interface NodeTaint {
  key: string;
  /** Empty for a valueless taint — a real value, not a missing one. */
  value: string;
  /** `NoSchedule`, `PreferNoSchedule` or `NoExecute`. */
  effect: string;
  /** RFC 3339; Kubernetes sets it only for `NoExecute` taints. */
  timeAdded?: string;
}

/**
 * Most disruptive first. `NoExecute` evicts pods that are already running,
 * `NoSchedule` only turns new ones away, and `PreferNoSchedule` is a hint —
 * so a tooltip that reads top-down reads worst-first, which is the order an
 * operator triaging "why won't this pod land" needs. An unrecognised effect
 * (a future one, or a malformed object) sorts last rather than being dropped:
 * a taint nobody can see is worse than a taint in the wrong place.
 */
const EFFECT_RANK: Record<string, number> = {
  NoExecute: 0,
  NoSchedule: 1,
  PreferNoSchedule: 2,
};

const rank = (effect: string): number => EFFECT_RANK[effect] ?? 3;

/** `key=value:effect` — `kubectl`'s own notation, valueless taints included. */
export function formatTaint(taint: NodeTaint): string {
  return `${taint.key}=${taint.value}:${taint.effect}`;
}

/**
 * The taints in tooltip order: severity first, then the API server's order
 * within an effect (a stable sort), so two nodes with the same taints always
 * read the same way.
 */
export function orderTaints(taints: readonly NodeTaint[]): NodeTaint[] {
  return [...taints].sort((a, b) => rank(a.effect) - rank(b.effect));
}

/** One `key=value:effect` per line, worst effect first. */
export function taintTooltip(taints: readonly NodeTaint[]): string {
  return orderTaints(taints).map(formatTaint).join("\n");
}

/**
 * What to show beside a taint on a detail page. Kubernetes stamps `timeAdded`
 * only on `NoExecute` taints, so most rows genuinely have no time — and the
 * text has to be a real one rather than an em dash, because the classic
 * design's `KV` drops a row whose value is "—" and the taint would disappear
 * from the page entirely. Shared so both designs say the same words.
 */
export function taintTimeAddedText(taint: NodeTaint): string {
  return taint.timeAdded ? `added ${taint.timeAdded}` : "time not recorded";
}

export interface TaintTally {
  noSchedule: number;
  preferNoSchedule: number;
  noExecute: number;
}

export function taintTally(taints: readonly NodeTaint[]): TaintTally {
  const tally: TaintTally = { noSchedule: 0, preferNoSchedule: 0, noExecute: 0 };
  for (const taint of taints) {
    if (taint.effect === "NoSchedule") tally.noSchedule += 1;
    else if (taint.effect === "PreferNoSchedule") tally.preferNoSchedule += 1;
    else if (taint.effect === "NoExecute") tally.noExecute += 1;
  }
  return tally;
}

/** The Taints column's cell: `NoSchedule / PreferNoSchedule / NoExecute`. */
export function taintTallyText(taints: readonly NodeTaint[]): string {
  const { noSchedule, preferNoSchedule, noExecute } = taintTally(taints);
  return `${noSchedule} / ${preferNoSchedule} / ${noExecute}`;
}

/** The header hint that says what the three numbers in a cell mean. */
export const TAINT_COLUMN_HINT = "NoSchedule / PreferNoSchedule / NoExecute";

/**
 * The badge's accessible name. "· 2" is a visual shorthand a screen reader
 * would read as punctuation, so the count is spelled out — and singular for
 * one, because "1 taints" is the kind of detail that makes a reader distrust
 * the rest of the screen.
 */
export function taintBadgeLabel(count: number): string {
  return count === 1 ? "1 taint" : `${count} taints`;
}

/** The badge's visible text: the count rides on the existing word. */
export function taintBadgeText(count: number): string {
  return `Tainted · ${count}`;
}

/**
 * What a Taints column sorts on: the count, so the most-constrained nodes come
 * to the top, and a node with no taints sorts as 0 rather than falling out of
 * the order. Ties break on the more disruptive effect, so among nodes with two
 * taints each the one that is evicting pods sorts above the one that is not.
 */
export function taintSortValue(taints: readonly NodeTaint[] | undefined): number {
  if (!taints || taints.length === 0) return 0;
  const { preferNoSchedule, noSchedule, noExecute } = taintTally(taints);
  // The count dominates; the effect mix is a fraction below 1, so it can only
  // ever order nodes that already have the same number of taints.
  const severity = (noExecute * 4 + noSchedule * 2 + preferNoSchedule) / (taints.length * 5);
  return taints.length + severity;
}

const isRecord = (v: unknown): v is Record<string, unknown> =>
  typeof v === "object" && v !== null && !Array.isArray(v);

const text = (v: unknown): string => (typeof v === "string" ? v : "");

/**
 * Read `spec.taints` off a live object for the detail page. Unlike the list,
 * this keeps *every* taint — including the one Kubernetes adds when a node is
 * cordoned, which the list leaves to its SchedulingDisabled badge. The detail
 * page is the drill-down, and `kubectl describe node` shows it there too.
 *
 * An entry that is not an object, or carries no `key`, is dropped: a row with
 * nothing to identify it is noise, not a taint.
 */
export function parseTaints(spec: unknown): NodeTaint[] {
  if (!isRecord(spec) || !Array.isArray(spec.taints)) return [];
  const taints: NodeTaint[] = [];
  for (const entry of spec.taints) {
    if (!isRecord(entry)) continue;
    const key = text(entry.key);
    if (!key) continue;
    const timeAdded = text(entry.timeAdded);
    taints.push({
      key,
      value: text(entry.value),
      effect: text(entry.effect),
      ...(timeAdded ? { timeAdded } : {}),
    });
  }
  return taints;
}
