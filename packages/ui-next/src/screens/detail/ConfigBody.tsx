import { asRecord, plural, str, type K8sObject } from "@srelens/core";
import { EmptyState } from "@srelens/ui-kit";
import { Section } from "./Section";

/**
 * One ConfigMap entry — key and value, both shown outright: ConfigMap data
 * is not sensitive, so `SecretBody`'s reveal affordance doesn't apply here
 * (classic's `ConfigDataEntry` takes a `secret` flag for exactly this
 * distinction). Not a `KV`: a ConfigMap value can run to many lines (a whole
 * config file), and `KV`'s single-line `dt`/`dd` row isn't built for that —
 * a `<pre>`, the same tag classic's `fl-secret-entry__value` used, preserves
 * it.
 */
function ConfigEntry({ name, value }: { name: string; value: string }) {
  return (
    <div className="flex flex-col gap-1">
      <span className="font-mono text-[0.8125rem] font-medium">{name}</span>
      <pre className="whitespace-pre-wrap break-all font-mono text-[0.8125rem] text-muted">{value}</pre>
    </div>
  );
}

/**
 * A ConfigMap's Details pane: its `data` keys and values — classic's
 * `ConfigBody`, minus the inline edit/save affordance (`ConfigDataEditor`):
 * this pane is read-only. `ServiceBody`'s Ports table and `PodBody`'s
 * Containers pane once made the same call about classic's inline port-forward
 * button; both offer it now, into §A.4's dialog — editing a ConfigMap in place
 * is a different question and still unanswered. `binaryData` is not read
 * here either — classic's own `ConfigBody` never read it.
 */
export function ConfigDetailsBody({ object }: { object: K8sObject }) {
  const data = asRecord(object.data) as Record<string, string>;
  const keys = Object.keys(data);
  return (
    // Remembered as `Data`, not as `Data (3 keys)`: the heading counts what
    // is in the object, and a memory keyed on it would be lost the first
    // time someone added a key.
    //
    // Open on a first visit, because it is the only titled block a ConfigMap's
    // pane has: everything else on it is the unheaded lead fact list, so a
    // shut Data block is a pane that says a ConfigMap exists and nothing about
    // what is in it. It still folds, and a reader who folds it gets that back.
    <Section id="Data" title={`Data (${plural(keys.length, "key")})`} defaultOpen>
      {keys.length === 0 ? (
        <EmptyState title="No data" />
      ) : (
        <div className="flex flex-col gap-4">
          {keys.map((key) => (
            <ConfigEntry key={key} name={key} value={str(data[key])} />
          ))}
        </div>
      )}
    </Section>
  );
}
