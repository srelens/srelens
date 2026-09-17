import { asArray, asRecord, str, type K8sObject } from "@srelens/core";
import { KV, Table, type Column } from "@srelens/ui-kit";
import { Section } from "./Section";
import { StringList } from "./sections";

interface IngressPathRow {
  key: string;
  host: string;
  path: string;
  backend: string;
}

const RULE_COLUMNS: Column<IngressPathRow>[] = [
  { key: "host", header: "Host", render: (r) => <span className="font-mono">{r.host}</span> },
  { key: "path", header: "Path", render: (r) => <span className="font-mono">{r.path}</span> },
  {
    key: "backend",
    header: "Backend",
    // Classic's cell is a `ResourceLink` to the Service; here the name:port
    // renders as inert text — see the task report for the full inert-value
    // list. The string itself is still the path → service mapping the reader
    // came for.
    render: (r) => <span className="font-mono">{r.backend}</span>,
  },
];

/**
 * Class and TLS — classic's "Ingress" section, ported fact-for-fact.
 * TLS secrets are a `LinkedResources` list in classic; here each name renders
 * as inert `Secret/name` text, same convention as CronJob's active jobs.
 */
function IngressSection({ object }: { object: K8sObject }) {
  const spec = asRecord(object.spec);
  const tls = asArray(spec.tls).flatMap((t) => asArray(asRecord(t).hosts).map(str));
  const tlsSecrets = asArray(spec.tls)
    .map((t) => str(asRecord(t).secretName))
    .filter(Boolean);

  return (
    <Section title="Ingress">
      <KV k="Class" v={str(spec.ingressClassName)} />
      <KV k="TLS hosts" v={tls.length ? tls.join(", ") : ""} />
      <KV
        k="TLS secrets"
        v={
          tlsSecrets.length > 0 ? (
            <StringList items={tlsSecrets.map((name) => `Secret/${name}`)} />
          ) : (
            ""
          )
        }
      />
    </Section>
  );
}

/**
 * Host / path / backend rows — classic's "Rules" table, shown only when the
 * Ingress declares any (a bare Ingress with only a defaultBackend has none
 * classic would list either). Backend is `name:port`, the same string classic
 * paints into its Service link.
 */
function RulesSection({ object }: { object: K8sObject }) {
  const spec = asRecord(object.spec);
  const rows: IngressPathRow[] = [];
  asArray(spec.rules).forEach((r, ri) => {
    const rr = asRecord(r);
    const host = str(rr.host) || "*";
    asArray(asRecord(rr.http).paths).forEach((p, pi) => {
      const pp = asRecord(p);
      const svc = asRecord(asRecord(pp.backend).service);
      const port = asRecord(svc.port);
      rows.push({
        key: `${ri}-${pi}`,
        host,
        path: str(pp.path) || "/",
        backend: `${str(svc.name)}:${str(port.number) || str(port.name)}`,
      });
    });
  });
  if (rows.length === 0) return null;

  return (
    <Section title="Rules">
      <Table columns={RULE_COLUMNS} data={rows} getRowKey={(r) => r.key} />
    </Section>
  );
}

/**
 * An Ingress's Details pane: Class/TLS and the path → service Rules table, in
 * classic's own order (`IngressBody`). This is what was missing from
 * `DETAILS_BODY`: without it the new design fell through to
 * `GenericBody` alone and never showed how traffic is routed.
 */
export function IngressDetailsBody({ object }: { object: K8sObject }) {
  return (
    <>
      <IngressSection object={object} />
      <RulesSection object={object} />
    </>
  );
}
