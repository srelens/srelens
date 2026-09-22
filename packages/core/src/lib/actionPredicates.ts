/**
 * Declared predicates (#550), evaluated on this side of the wire.
 *
 * The host owns enforcement: an action's `preconditions` are checked against
 * the fresh GET `crates/kube`'s action primitives perform, and nothing here
 * can let a write past that. What this evaluates is `availableWhen` — whether
 * a control is offered at all — so a person is not shown a button whose
 * refusal is already known from the resource in front of them.
 *
 * It mirrors `crates/capability/src/predicate.rs`, case for case: a surface
 * that answered differently would hide a control the host would accept, or
 * offer one it refuses. The grammar is the same bounded JSONPath subset, and
 * anything outside it fails closed here as it does there — the host refuses
 * such a predicate at install, so seeing one means something is wrong, and an
 * unreadable condition is not a met condition.
 */

/** Most predicates one declared list may hold; `MAX_PREDICATES` in Rust. */
export const MAX_PREDICATES = 8;

const MAX_PATH_CHARS = 256;
const MAX_PATH_SEGMENTS = 8;
const MAX_REASON_CHARS = 200;

/** One declared check about a resource. Exactly one operator per predicate. */
export interface ActionPredicate {
  /** `.spec.suspend`, `.metadata.annotations['acme.io/pinned']`, `.status.conditions[0].status`. */
  jsonPath: string;
  equals?: string | number | boolean;
  notEquals?: string | number | boolean;
  present?: true;
  absent?: true;
  /** Shown when the predicate does not hold. Untrusted: escape before drawing. */
  reason: string;
}

type Segment = { key: string } | { index: number };

/** The inside of one `[...]`: an index, or a quoted key. */
function bracket(inner: string): Segment | undefined {
  if (/^(0|[1-9][0-9]*)$/.test(inner)) return { index: Number(inner) };
  for (const quote of ["'", '"']) {
    if (inner.length >= 2 && inner.startsWith(quote) && inner.endsWith(quote)) {
      const key = inner.slice(1, -1);
      return key && !key.includes(quote) ? { key } : undefined;
    }
  }
  return undefined;
}

/** One path's segments, or `undefined` when it is not a path we evaluate. */
function segments(path: string): Segment[] | undefined {
  if (typeof path !== "string" || path.length > MAX_PATH_CHARS) return undefined;
  let rest = path.startsWith("$") ? path.slice(1) : path;
  if (!rest.startsWith(".") && !rest.startsWith("[")) return undefined;
  const parsed: Segment[] = [];
  while (rest) {
    if (rest.startsWith("[")) {
      const close = rest.indexOf("]");
      if (close < 0) return undefined;
      const segment = bracket(rest.slice(1, close));
      if (!segment) return undefined;
      parsed.push(segment);
      rest = rest.slice(close + 1);
    } else if (rest.startsWith(".")) {
      const after = rest.slice(1);
      const end = after.search(/[.[]/);
      const key = end < 0 ? after : after.slice(0, end);
      if (!key || !/^[A-Za-z0-9_-]+$/.test(key)) return undefined;
      parsed.push({ key });
      rest = end < 0 ? "" : after.slice(end);
    } else return undefined;
  }
  return parsed.length > MAX_PATH_SEGMENTS ? undefined : parsed;
}

/**
 * The value at a predicate's path, or `undefined` when the resource holds
 * none — including when the path is one this subset does not evaluate.
 */
export function resolvePath(resource: unknown, path: string): unknown {
  const parsed = segments(path);
  if (!parsed) return undefined;
  let node: unknown = resource;
  for (const segment of parsed) {
    if (node === null || typeof node !== "object") return undefined;
    if ("index" in segment) {
      if (!Array.isArray(node)) return undefined;
      node = node[segment.index];
    } else {
      if (Array.isArray(node)) return undefined;
      node = (node as Record<string, unknown>)[segment.key];
    }
    if (node === undefined) return undefined;
  }
  return node;
}

/** A comparand is a literal; an object or a list is a shape match, not a value. */
const isLiteral = (value: unknown) =>
  (typeof value === "string" && value.length <= MAX_REASON_CHARS) ||
  typeof value === "number" ||
  typeof value === "boolean";

/** Every rule a declared predicate must satisfy, as the host states them. */
function wellFormed(predicate: ActionPredicate): boolean {
  if (!predicate || typeof predicate !== "object") return false;
  if (!segments(predicate.jsonPath)) return false;
  const declared = [
    predicate.equals !== undefined,
    predicate.notEquals !== undefined,
    predicate.present !== undefined,
    predicate.absent !== undefined,
  ].filter(Boolean);
  if (declared.length !== 1) return false;
  if (predicate.present !== undefined && predicate.present !== true) return false;
  if (predicate.absent !== undefined && predicate.absent !== true) return false;
  if (predicate.equals !== undefined && !isLiteral(predicate.equals)) return false;
  if (predicate.notEquals !== undefined && !isLiteral(predicate.notEquals)) return false;
  const reason = typeof predicate.reason === "string" ? predicate.reason.trim() : "";
  return reason.length > 0 && predicate.reason.length <= MAX_REASON_CHARS;
}

/** Whether one predicate holds for `resource`. A malformed one never does. */
export function predicateHolds(predicate: ActionPredicate, resource: unknown): boolean {
  if (!wellFormed(predicate)) return false;
  // A null is how the API server spells a field nobody set; the two are one
  // answer, as they are in the host.
  const found = resolvePath(resource, predicate.jsonPath) ?? undefined;
  if (predicate.present) return found !== undefined;
  if (predicate.absent) return found === undefined;
  if (predicate.equals !== undefined) return found === predicate.equals;
  return found !== predicate.notEquals;
}

/**
 * The first predicate that does not hold for `resource`, or `undefined` when
 * every one does.
 *
 * Per resource on purpose: a detail view asks it once, and a list view maps it
 * over the rows it already holds to say how much of a selection an action
 * applies to (#553). The `reason` it returns is a manifest's text — draw it
 * through `plainText` rather than raw.
 */
export function unmetPredicate(
  predicates: ActionPredicate[] | undefined,
  resource: unknown,
): ActionPredicate | undefined {
  if (!predicates?.length) return undefined;
  // A list longer than the host accepts is not one the host installed, so
  // none of it is trusted to say the action applies.
  if (predicates.length > MAX_PREDICATES) return predicates[0];
  return predicates.find((predicate) => !predicateHolds(predicate, resource));
}
