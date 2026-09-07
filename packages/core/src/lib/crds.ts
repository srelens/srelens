import { invokeCapability, type Invoker } from "../transport/transport";
import { ageSeconds } from "./age";

/**
 * One column a CRD asks tools to display, from its `additionalPrinterColumns`
 * — the same metadata `kubectl get` renders. Custom resources share no fields
 * beyond name/namespace/age, so this is the only way to show anything useful
 * about them (health, phase, version, replica counts…).
 */
export interface PrinterColumn {
  name: string;
  jsonPath: string;
  type: string;
}

/** A discovered CustomResourceDefinition, enough to list/view its instances. */
export interface CrdRef {
  name: string;
  group: string;
  version: string;
  kind: string;
  plural: string;
  namespaced: boolean;
  /** Optional: older backends and hand-built refs in tests may omit it. */
  printerColumns?: PrinterColumn[];
  /** Served versions, in declaration order. Optional for the same reason `printerColumns` is. */
  versions?: string[];
  /** The version objects are stored as. Optional for the same reason `printerColumns` is. */
  storageVersion?: string;
}

export interface CustomRow {
  name: string;
  namespace: string;
  age: string;
  /** Values for the CRD's printer columns, in declaration order. */
  columns?: string[];
  /** Raw values for columns whose rendered text does not sort correctly. */
  sortKeys?: string[];
}

/**
 * Stable table keys for a CRD's printer columns.
 *
 * These keys persist: `useColumnVisibility` stores hidden columns under them,
 * and the tab keeps sort and filter state by key. A positional key would
 * therefore silently change meaning when an operator upgrade inserts or
 * reorders `additionalPrinterColumns` — a column the user hid or sorted would
 * come back as a different one. Identify a column by what it *is* instead, and
 * add an occurrence suffix only for genuinely duplicate definitions.
 */
export function printerColumnKeys(columns: PrinterColumn[]): string[] {
  const seen = new Map<string, number>();
  return columns.map((column) => {
    const base = `printer:${column.name}:${column.jsonPath}`;
    const occurrence = (seen.get(base) ?? 0) + 1;
    seen.set(base, occurrence);
    return occurrence === 1 ? base : `${base}#${occurrence}`;
  });
}

/**
 * Sort key for a printer column's rendered text, honouring the type the CRD
 * declared. The table's collator handles plain digit strings, but not signed or
 * decimal numbers, and `date` columns have already been rendered as compact
 * ages ("2h", "10d") whose text does not order chronologically.
 *
 * `integer` yields a bigint, since Kubernetes integers are 64-bit and Number
 * cannot represent them all exactly. The table compares bigints relationally.
 */
export function printerSortValue(
  type: string,
  value: string,
  sortKey = "",
  now: number = Date.now(),
): number | string | bigint {
  if (type === "integer" || type === "number") {
    const text = value.trim();
    // Unset or unparseable values group below every real number. Check for
    // empty first: Number("") is 0, which would interleave blank cells with
    // real zeros and negatives.
    if (text === "") return Number.NEGATIVE_INFINITY;
    if (type === "integer") {
      // Kubernetes integers are 64-bit, and Number cannot hold them exactly:
      // 9007199254740993 collapses onto ...992, tying two distinct rows.
      try {
        return BigInt(text);
      } catch {
        return Number.NEGATIVE_INFINITY;
      }
    }
    const parsed = Number(text);
    return Number.isFinite(parsed) ? parsed : Number.NEGATIVE_INFINITY;
  }
  if (type === "date") {
    // Prefer the raw timestamp: two rows inside the same displayed unit both
    // render "1h" and would otherwise tie. Convert it to an age rather than
    // using the epoch value directly, so it sorts in the same direction as
    // `ageSeconds` — larger means older, for both this and the Age column.
    const parsed = sortKey ? Date.parse(sortKey) : NaN;
    if (Number.isFinite(parsed)) return (now - parsed) / 1000;
    return ageSeconds(value);
  }
  return value;
}

/** Discover installed CRDs in a cluster via `k8s.listCRDs`. */
export async function listCrds(
  context: string,
  invoke: Invoker = invokeCapability,
): Promise<{ crds?: CrdRef[]; error?: string }> {
  try {
    const out = await invoke<{ crds: CrdRef[] }>("k8s.listCRDs", { context });
    return { crds: out.crds };
  } catch (e) {
    return { error: String(e) };
  }
}

/** List instances of a custom resource via `k8s.listCustomResource`. */
export async function listCustomResource(
  context: string,
  crd: CrdRef,
  namespace: string | null,
  invoke: Invoker = invokeCapability,
): Promise<{ items?: CustomRow[]; error?: string }> {
  try {
    const out = await invoke<{ items: CustomRow[] }>("k8s.listCustomResource", {
      context,
      group: crd.group,
      version: crd.version,
      plural: crd.plural,
      kind: crd.kind,
      namespaced: crd.namespaced,
      namespace: namespace ?? "",
      printerColumns: crd.printerColumns ?? [],
    });
    return { items: out.items };
  } catch (e) {
    return { error: String(e) };
  }
}
