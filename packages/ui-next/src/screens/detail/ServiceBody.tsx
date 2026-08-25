import { useEffect, useState } from "react";
import {
  asArray,
  asRecord,
  listEndpointSlices,
  serviceExternalAddress,
  str,
  type K8sObject,
} from "@srelens/core";
import { KV, PairList, Table, type Column } from "@srelens/ui-kit";
import { ForwardAction } from "../forwards/ForwardAction";
import { Section } from "./Section";
import { StringList } from "./sections";

/**
 * How the Service is reached — classic's "Connection" section, ported
 * fact-for-fact: Type, Cluster IP, External IP (core's own
 * `serviceExternalAddress`, which already encodes classic's LoadBalancer
 * `<pending>` / ExternalName rules), Session affinity, Selector. Selector is
 * OMITTED when empty rather than shown as classic's `Chips` widget does
 * ("None") — the same convention `WorkloadBody`'s Properties section settled
 * on, kept here too rather than reintroducing classic's "None" text for this
 * body alone.
 */
function ConnectionSection({ object }: { object: K8sObject }) {
  const spec = asRecord(object.spec);
  const selector = asRecord(spec.selector) as Record<string, string>;

  return (
    <Section title="Connection">
      <KV k="Type" v={str(spec.type) || "ClusterIP"} />
      <KV k="Cluster IP" v={str(spec.clusterIP)} mono />
      <KV k="External IP" v={serviceExternalAddress(object) || "—"} mono />
      <KV k="Session affinity" v={str(spec.sessionAffinity)} />
      {/* `breakValues`: `PairList` truncates by default and no longer writes
          the value into a row `title` — that attribute was how a Secret's
          whole applied manifest reached the DOM — so wrapping is the only way
          a long selector key/value can be read at all. */}
      {Object.keys(selector).length > 0 && (
        <KV k="Selector" v={<PairList pairs={Object.entries(selector)} breakValues />} />
      )}
    </Section>
  );
}

interface PortRow {
  key: string;
  name: string;
  port: string;
  /**
   * The Service port ITSELF, as a number — what a forward is pointed at.
   *
   * Separate from `port`, which folds the node port in for display
   * ("80:30080") and cannot be parsed back, and deliberately not `target`:
   * `kubectl port-forward svc/x <local>:<remote>` takes the SERVICE port and
   * resolves the `targetPort` behind it on its own. `null` for a port that is
   * not a number a socket could bind, which is nothing this table can forward.
   */
  forwardPort: number | null;
  target: string;
  protocol: string;
}

/** The highest port a TCP socket can bind. */
const MAX_PORT = 65_535;

const PORT_COLUMNS: Column<PortRow>[] = [
  { key: "name", header: "Name", render: (p) => p.name },
  { key: "port", header: "Port", render: (p) => <span className="font-mono">{p.port}</span> },
  { key: "target", header: "Target", render: (p) => <span className="font-mono">{p.target}</span> },
  { key: "protocol", header: "Protocol", render: (p) => p.protocol },
];

/**
 * The Service's own ports — classic's "Ports" table, shown only when the
 * Service declares any (an ExternalName service, for instance, has none).
 * "Port" folds in the node port the way classic does ("80:30080").
 *
 * **Every row is a way into a forward**, which is the affordance classic
 * offers inline here and ui-next could not, having had no forward dialog when
 * this body was ported. It has one now (§A.4), and this is the obvious place
 * for it: the row is already showing the reader the exact port they want.
 * The row does not START anything — {@link ForwardAction} opens the dialog
 * prefilled and the reader confirms there.
 */
function PortsSection({ object, context }: { object: K8sObject; context: string }) {
  const spec = asRecord(object.spec);
  const namespace = str(object.metadata?.namespace);
  const name = str(object.metadata?.name);
  const ports: PortRow[] = asArray(spec.ports).map((p, i) => {
    const pr = asRecord(p);
    const number = Number(str(pr.port));
    return {
      key: str(pr.name) || `port-${i}`,
      name: str(pr.name) || "—",
      port: str(pr.port) + (pr.nodePort ? `:${str(pr.nodePort)}` : ""),
      forwardPort: Number.isInteger(number) && number > 0 && number <= MAX_PORT ? number : null,
      target: str(pr.targetPort),
      protocol: str(pr.protocol) || "TCP",
    };
  });
  if (ports.length === 0) return null;

  // No cluster, or a Service the API server named neither — there is nothing
  // to point a forward AT, and a control that opened a dialog which could not
  // say what it was forwarding is worse than no control.
  const canForward = Boolean(context && namespace && name);
  const columns: Column<PortRow>[] = canForward
    ? [
        ...PORT_COLUMNS,
        {
          // §13's own unnamed trailing column, for the same reason: the header
          // would name a verb the cells already say.
          key: "forward",
          header: "",
          sortable: false,
          filterable: false,
          align: "end",
          render: (p) =>
            p.forwardPort === null ? null : (
              <ForwardAction
                context={context}
                namespace={namespace}
                kind="Service"
                name={name}
                remotePort={p.forwardPort}
                // Named per row: a table of "Forward" names nothing. The PORT
                // is what tells the rows apart, and the row shows it already —
                // this is not a value hidden in an attribute.
                label={`Forward port ${p.forwardPort}`}
              >
                Forward
              </ForwardAction>
            ),
        },
      ]
    : PORT_COLUMNS;

  return (
    <Section title="Ports">
      <Table columns={columns} data={ports} getRowKey={(p) => p.key} />
    </Section>
  );
}

/**
 * The EndpointSlices backing this Service — classic's "Endpoint Slices",
 * matched by the `kubernetes.io/service-name` label the backend surfaces as
 * `service` on `EndpointSliceSummary`, fetched live via core's
 * `listEndpointSlices`. Classic renders nothing at all while the fetch is in
 * flight or comes back empty (no spinner, no empty-state row) — this section
 * only ever appears once slices are found, and that is kept here too rather
 * than inventing a loading state classic never had. Each slice is
 * `ResourceLink`-navigable in classic; here it renders as inert
 * "EndpointSlice/name" text — see the task report for the full inert-value
 * list.
 */
function EndpointSlicesSection({ context, object }: { context: string; object: K8sObject }) {
  const namespace = str(object.metadata?.namespace);
  const name = str(object.metadata?.name);
  const [names, setNames] = useState<string[]>([]);

  useEffect(() => {
    setNames([]);
    if (!context || !namespace || !name) return;
    let active = true;
    listEndpointSlices(context, namespace).then((out) => {
      if (!active) return;
      const mine = (out.endpointslices ?? []).filter((s) => s.service === name).map((s) => s.name);
      setNames(mine);
    });
    return () => {
      active = false;
    };
  }, [context, namespace, name]);

  if (names.length === 0) return null;
  return (
    <Section title="Endpoint Slices">
      <StringList items={names.map((n) => `EndpointSlice/${n}`)} />
    </Section>
  );
}

/**
 * A Service's Details pane: Connection, Ports and Endpoint Slices, in
 * classic's own order (`ServiceBody`). Related pods and Conditions come from
 * `GenericBody`, not from here — `relatedPodSelector` returns the Service's
 * own `spec.selector` non-empty, so `GenericBody` already renders a "Pods"
 * section for a Service with one; rendering another here would duplicate it,
 * exactly the mistake a workload body made for DaemonSet (see the task
 * report).
 */
export function ServiceDetailsBody({ object, context }: { object: K8sObject; context: string }) {
  return (
    <>
      <ConnectionSection object={object} />
      <PortsSection object={object} context={context} />
      <EndpointSlicesSection context={context} object={object} />
    </>
  );
}
