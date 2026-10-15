import {
  ageFromTimestamp,
  asArray,
  asRecord,
  formatResourceManifest,
  resolvePath,
  type Condition,
  type K8sObject,
  type PrinterColumn,
} from "@srelens/core";
import { CodeEditor } from "@srelens/ui-kit";
import { identityFacts } from "./GenericBody";
import { Section } from "./Section";
import { ConditionsSection } from "./sections";
import type { DetailFact } from "./facts";

/** A printer column's value as the fact list reads it; `undefined` drops the fact. */
function printerText(type: string, value: unknown): string | undefined {
  if (value === undefined || value === null || value === "") return undefined;
  if (typeof value === "string") return type === "date" ? ageFromTimestamp(value) : value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  return JSON.stringify(value);
}

/**
 * A custom resource's lead facts (#919): what its own operator chose to show
 * in `kubectl get` — the CRD's printer columns — then its phase and a lagging
 * generation where the columns do not already say so, then the identity
 * facts every kind ends on.
 *
 * Evaluated here with core's `resolvePath`, the bounded JSONPath subset the
 * host's printer columns also read, over the object already fetched — so no
 * second read. A path outside that subset (an escaped-dot key) resolves to
 * nothing and its fact is left out rather than shown wrong; the list's table,
 * evaluated by the host, still shows it.
 */
export function customResourceFacts(object: K8sObject, printerColumns: PrinterColumn[] = []): DetailFact[] {
  const facts: DetailFact[] = [];
  for (const column of printerColumns) {
    const text = printerText(column.type, resolvePath(object, column.jsonPath));
    if (text !== undefined) facts.push({ label: column.name, value: text });
  }
  const status = asRecord(object.status);
  if (
    typeof status.phase === "string" &&
    status.phase &&
    !printerColumns.some((column) => column.jsonPath === ".status.phase")
  ) {
    facts.push({ label: "Phase", value: status.phase });
  }
  // The controller has not caught up with the latest spec. Only said when it
  // is true: "5 of 5" on every healthy resource would be noise.
  const generation = asRecord(object.metadata).generation;
  const observed = status.observedGeneration;
  if (typeof generation === "number" && typeof observed === "number" && observed < generation) {
    facts.push({ label: "Observed generation", value: `${observed} of ${generation}` });
  }
  return [...facts, ...identityFacts({ kind: object.kind ?? "", object })];
}

/**
 * The Status pane's blocks for a custom resource: its conditions, then its
 * whole `.status` — the only generic way to show what an operator reports
 * outside conditions (an Argo CD Application's sync and health, a release's
 * history). Read-only; Edit in the action bar is where a change is made.
 *
 * Titled "Raw status" rather than "Status" because the pane it sits in is
 * already called that. Like every titled block it opens shut on a first visit.
 */
export function CustomResourceBody({ object, name }: { object: K8sObject; name: string }) {
  const status = asRecord(object.status);
  const conditions = asArray(status.conditions) as unknown as Condition[];
  return (
    <>
      <ConditionsSection conditions={conditions} />
      {Object.keys(status).length > 0 && (
        <Section title="Raw status">
          <CodeEditor
            value={formatResourceManifest(status)}
            readOnly
            language="yaml"
            copy
            ariaLabel={`${name} status`}
          />
        </Section>
      )}
    </>
  );
}
