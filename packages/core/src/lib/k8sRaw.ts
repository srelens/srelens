/**
 * Small raw-object accessors for walking untyped Kubernetes API objects
 * (`Record<string, unknown>`, `unknown[]`, loose field values). These are
 * internal plumbing shared across the `k8s*` modules — `k8sHealth`,
 * `k8sContainer`, and friends — not a general-purpose API the rest of the
 * app should reach for. They are exported only so those sibling modules
 * (and their tests) can import a single copy instead of each keeping one.
 */
export function asRecord(v: unknown): Record<string, unknown> {
  return v && typeof v === "object" ? (v as Record<string, unknown>) : {};
}
export function asArray(v: unknown): unknown[] {
  return Array.isArray(v) ? v : [];
}
export function str(v: unknown): string {
  if (v == null) return "";
  if (typeof v === "string") return v;
  return String(v);
}
export function plural(n: number, one: string, many = `${one}s`): string {
  return `${n} ${n === 1 ? one : many}`;
}
