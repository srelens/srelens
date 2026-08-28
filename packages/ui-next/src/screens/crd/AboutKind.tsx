import type { ReactNode } from "react";
import type { CrdRef } from "@srelens/core";
import { CopyCommand, KVList, Section } from "@srelens/ui-kit";

export interface AboutKindProps {
  /** The CustomResourceDefinition this list's rows come from. */
  crd: CrdRef;
  /** The cluster context name, for the command a reader can take away. */
  context: string;
  /**
   * How many objects of this kind the list is showing, or `undefined` when
   * there is no count yet. Optional for the same reason the version fields
   * are: a row nothing is known for is left out, never drawn as a zero.
   */
  objects?: number;
}

/**
 * The rail beside a custom resource's list: what this kind IS, for a reader
 * who has just met it.
 *
 * A built-in kind needs no such thing — anyone opening a Pods list already
 * knows what a Pod is, and the sidebar's own grouping says the rest. A custom
 * resource is the opposite: the slug in the route is the only name the reader
 * has been given, it is a plural DNS name rather than a kind, and nothing on
 * the screen says which API group it belongs to, whether its objects live in a
 * namespace, or which of its versions the cluster actually stores. All of that
 * is on the CRD the list was already built from, so the rail costs no round
 * trip of its own.
 *
 * **Every value here is read off the CRD, none of it is assumed.** The design's
 * §12 hard-codes `Scope: Namespaced`, `Served versions: v1, v1beta1` and
 * `Storage version: v1`, and never draws a cluster-scoped kind at all though
 * its own tree lists `ClusterIssuers` and `PriorityClasses`. Those are a mock's
 * fixtures, not a rule — a rail that tells a reader a cluster-scoped kind is
 * namespaced is worse than one that says nothing.
 *
 * **A ROW WITH NOTHING BEHIND IT IS LEFT OUT, NOT DRAWN EMPTY OR ZEROED.**
 * `versions` and `storageVersion` are both optional on `CrdRef` (an older
 * backend, or a ref hand-built in a test, arrives without them), and `objects`
 * is absent until the list has answered. A key with an empty value invites the
 * reader to conclude the CRD serves no versions, which is not a thing a CRD
 * can do; `Objects 0` on a kind with forty of them is worse still, because
 * zero is a number a reader will believe. An absent row asks a question. A
 * wrong one answers it.
 *
 * The title comes from `crd.kind`. The design derives it from the slug by
 * upper-casing the first letter, which renders `servicemonitors` as
 * `Servicemonitors` — a string that names nothing and that kubectl will not
 * accept. `crd.kind` is `ServiceMonitor`.
 *
 * It renders its two `Section`s and nothing around them: `SideRail` drops what
 * it is handed straight into one box, and `.section + .section` is what rules
 * between them. A wrapper here would break that adjacency and the rail would
 * read as one undivided block.
 */
export function AboutKind({ crd, context, objects }: AboutKindProps) {
  const rows: Array<[key: string, value: ReactNode]> = [
    ["Kind", crd.kind],
    // Read, not assumed — see the note above.
    ["Scope", crd.namespaced ? "Namespaced" : "Cluster"],
  ];
  if (crd.versions?.length) rows.push(["Served versions", crd.versions.join(", ")]);
  if (crd.storageVersion) rows.push(["Storage version", crd.storageVersion]);
  if (objects !== undefined) rows.push(["Objects", String(objects)]);

  return (
    <>
      <Section title="Definition">
        <KVList rows={rows} />
      </Section>
      <Section title="Fetch it yourself">
        {/* `CopyCommand`, not `KubectlPreview`: that one prints "Equivalent
            kubectl:" ahead of the line, which belongs beside an action the app
            is about to perform and says the wrong thing under a heading that
            hands the reader something to run themselves. The kit's own note
            carries the rest of the reasoning, the wrapping included.

            `crd.name` rather than `crd.plural`: the fully qualified
            `<plural>.<group>` is what kubectl resolves unambiguously when two
            operators have installed a kind of the same short name. */}
        <CopyCommand command={`kubectl --context ${context} get ${crd.name} -A -o wide`} />
      </Section>
    </>
  );
}
