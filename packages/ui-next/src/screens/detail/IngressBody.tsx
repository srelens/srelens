import { useState } from "react";
import {
  asArray,
  asRecord,
  ingressRuleAddress,
  ingressUsesRegexPaths,
  openExternal,
  str,
  type IngressRuleAddress,
  type K8sObject,
} from "@srelens/core";
import { Button, CopyButton, KV, Table, type Column } from "@srelens/ui-kit";
import { FailureAlert } from "../../lib/errorCopy";
import { Section } from "./Section";
import { StringList } from "./sections";

interface IngressPathRow {
  key: string;
  host: string;
  path: string;
  backend: string;
  /** Where the rule sends a reader; `null` for a rule with no host. */
  address: IngressRuleAddress | null;
}

function ruleColumns(open: (url: string) => void): Column<IngressPathRow>[] {
  return [
    { key: "host", header: "Host", render: (r) => <span className="font-mono">{r.host}</span> },
    { key: "path", header: "Path", render: (r) => <span className="font-mono">{r.path}</span> },
    {
      key: "address",
      header: "URL",
      getValue: (r) => (r.address?.kind === "url" ? r.address.url : (r.address?.host ?? "")),
      render: (r) => <RuleAddress address={r.address} open={open} />,
    },
    {
      key: "backend",
      header: "Backend",
      // Classic's cell is a `ResourceLink` to the Service; here the label
      // renders as inert text — service backends stay `name:port`, resource
      // backends `kind/name`. See the task report for the full inert-value
      // list.
      render: (r) => <span className="font-mono">{r.backend}</span>,
    },
  ];
}

/**
 * A rule's address: a link that opens the system browser, with a copy beside
 * it — or, for a wildcard host, the host as text with the copy alone, since
 * there is no one address to open.
 *
 * A `Button` rather than an anchor, for the reason Forwards' address is one:
 * `<a target="_blank">` opens nothing inside the Tauri WebView (#348). Its
 * accessible name is the address it shows, as a link's would be.
 */
function RuleAddress({ address, open }: { address: IngressRuleAddress | null; open: (url: string) => void }) {
  if (!address) return <span className="text-muted">—</span>;
  const copied = address.kind === "url" ? address.url : address.host;
  return (
    <span className="flex min-w-0 items-center gap-1">
      {address.kind === "url" ? (
        <Button
          variant="ghost"
          size="xs"
          className="-mx-1 min-w-0 max-w-full text-accent"
          onClick={() => open(address.url)}
        >
          <span className="min-w-0 truncate font-mono">{address.url}</span>
        </Button>
      ) : (
        <span className="min-w-0 truncate font-mono">{address.host}</span>
      )}
      <CopyButton text={copied} label={`Copy ${copied}`} iconOnly />
    </span>
  );
}

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
 * Format an Ingress HTTP path backend the way the Rules table shows it.
 * Service backends stay `name:port` (classic's string). Resource backends —
 * mutually exclusive with service in networking.k8s.io/v1 — render as
 * `kind/name` so the cell is not a bare `:`.
 */
function backendLabel(backend: Record<string, unknown>): string {
  const svc = asRecord(backend.service);
  const name = str(svc.name);
  if (name) {
    const port = asRecord(svc.port);
    return `${name}:${str(port.number) || str(port.name)}`;
  }
  const resource = asRecord(backend.resource);
  const kind = str(resource.kind);
  const resourceName = str(resource.name);
  if (kind && resourceName) return `${kind}/${resourceName}`;
  return resourceName || kind;
}

/**
 * Host / path / backend rows — classic's "Rules" table, shown only when the
 * Ingress declares any (a bare Ingress with only a defaultBackend has none
 * classic would list either).
 */
function RulesSection({ object }: { object: K8sObject }) {
  const [failure, setFailure] = useState<{ url: string; error: unknown } | null>(null);
  const spec = asRecord(object.spec);
  const ingress = {
    tlsHosts: asArray(spec.tls).flatMap((t) => asArray(asRecord(t).hosts).map(str)),
    regexPaths: ingressUsesRegexPaths(asRecord(asRecord(object.metadata).annotations)),
  };
  const rows: IngressPathRow[] = [];
  asArray(spec.rules).forEach((r, ri) => {
    const rr = asRecord(r);
    const host = str(rr.host);
    asArray(asRecord(rr.http).paths).forEach((p, pi) => {
      const pp = asRecord(p);
      const path = str(pp.path) || "/";
      rows.push({
        key: `${ri}-${pi}`,
        host: host || "*",
        path,
        backend: backendLabel(asRecord(pp.backend)),
        address: ingressRuleAddress(host, path, ingress),
      });
    });
  });
  if (rows.length === 0) return null;

  // Said here, beside the link that failed, rather than in a toast: a toast
  // reaches nobody in this design (#374).
  async function open(url: string) {
    setFailure(null);
    try {
      await openExternal(url);
    } catch (error) {
      setFailure({ url, error });
    }
  }

  return (
    <Section title="Rules">
      {failure && (
        <FailureAlert title={`Could not open ${failure.url}`} error={failure.error} className="mb-2" />
      )}
      <Table columns={ruleColumns((url) => void open(url))} data={rows} getRowKey={(r) => r.key} />
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
