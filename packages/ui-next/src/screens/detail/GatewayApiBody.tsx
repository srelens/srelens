import { listCrds, listCustomResource, str, type K8sObject } from "@srelens/core";
import { Alert, Button, LoadingState, Table, type Column } from "@srelens/ui-kit";
import { detailRoute } from "../../lib/detailRoute";
import { FailureAlert } from "../../lib/errorCopy";
import {
  GATEWAY_API_GROUP,
  PARENT_COLUMNS,
  ROUTE_KINDS,
  gatewayListeners,
  routeParents,
  routeRules,
  routesAttachedTo,
  type GatewayListener,
  type RouteBackend,
  type RouteParent,
  type RouteRule,
} from "../../lib/gatewayApi";
import { openTab } from "../../lib/tabsStore";
import { Section } from "./Section";
import { SectionFailure, useSectionList } from "./sectionList";

const mono = (text: string) => <span className="font-mono">{text || "—"}</span>;

/** A resource named in a cell, as the way to it: a tab of its own, on this cluster. */
function Opens({ label, route, context }: { label: string; route: string; context: string }) {
  return (
    <Button type="button" variant="link" onClick={() => openTab(route, { clusterName: context })}>
      <span className="whitespace-nowrap font-mono">{label}</span>
    </Button>
  );
}

const LISTENER_COLUMNS: Column<GatewayListener>[] = [
  { key: "name", header: "Listener", render: (l) => mono(l.name) },
  { key: "protocol", header: "Protocol", render: (l) => l.protocol || "—" },
  { key: "port", header: "Port", render: (l) => mono(l.port) },
  { key: "hostname", header: "Hostname", render: (l) => mono(l.hostname) },
  { key: "routes", header: "Routes", align: "end", render: (l) => (l.attachedRoutes === null ? "—" : String(l.attachedRoutes)) },
  { key: "programmed", header: "Programmed", render: (l) => l.programmed || "—" },
];

interface AttachedRoute {
  kind: string;
  namespace: string;
  name: string;
}

const ATTACHED_COLUMNS: Column<AttachedRoute>[] = [
  { key: "name", header: "Route", render: (r) => mono(r.name) },
  { key: "namespace", header: "Namespace", render: (r) => r.namespace },
  { key: "kind", header: "Kind", render: (r) => r.kind },
];

/**
 * The routes attached to a Gateway.
 *
 * A Gateway does not name its routes; each route names its gateways. So this
 * lists every route kind the cluster serves, in every namespace — a route may
 * attach from anywhere the gateway's listeners allow — asking the host for
 * each route's parents and keeping the ones that name this gateway.
 */
function GatewayRoutesSection({ object, context }: { object: K8sObject; context: string }) {
  const gateway = { namespace: str(object.metadata?.namespace), name: str(object.metadata?.name) };
  const state = useSectionList<{ rows: AttachedRoute[]; failed: { kind: string; error: string }[]; truncated: boolean }>(true, [context, gateway.namespace, gateway.name], async () => {
    const discovered = await listCrds(context);
    if (discovered.error) return { error: discovered.error };
    const kinds = (discovered.crds ?? []).filter(
      (crd) => crd.group === GATEWAY_API_GROUP && ROUTE_KINDS.includes(crd.kind),
    );
    // A role bound in one namespace cannot list cluster-wide: fall back to the
    // gateway's own namespace. A kind still unreadable is named, not fatal.
    const lists = await Promise.all(
      kinds.map(async (crd) => {
        const ref = { ...crd, printerColumns: PARENT_COLUMNS };
        const all = await listCustomResource(context, ref, null);
        return all.error ? listCustomResource(context, ref, gateway.namespace) : all;
      }),
    );
    const failed = kinds.flatMap((crd, i) => {
      const error = lists[i].error;
      return error ? [{ kind: crd.kind, error }] : [];
    });
    if (kinds.length > 0 && failed.length === kinds.length) return { error: lists[0].error };
    return {
      data: {
        failed,
        // Filtered from a list cut at the row cap, the routes that attach
        // here may be the ones past it: say so rather than claim the count.
        truncated: lists.some((list) => list.truncated),
        rows: lists.flatMap((list, i) =>
          routesAttachedTo(list.items ?? [], gateway).map((row) => ({
            kind: kinds[i].kind,
            namespace: row.namespace,
            name: row.name,
          })),
        ),
      },
    };
  });
  const rows = state.data?.rows ?? [];
  const failed = state.data?.failed ?? [];
  const truncated = state.data?.truncated ?? false;
  const open = (route: AttachedRoute) =>
    openTab(detailRoute(route.kind, route.namespace, route.name, { cluster: context, group: GATEWAY_API_GROUP }), {
      clusterName: context,
    });

  return (
    <Section title={state.status === "ready" ? `Routes (${rows.length}${truncated ? "+" : ""})` : "Routes"} id="Routes">
      {state.status === "loading" ? (
        <LoadingState compact label={`Loading routes attached to ${gateway.name}`} />
      ) : state.status === "error" ? (
        <SectionFailure what="routes" error={state.error} onRetry={state.retry} />
      ) : (
        <>
          {failed.length > 0 && (
            // The rows that were read stay; the kinds that were not say why and can be asked again.
            <FailureAlert
              title={`Could not list ${failed.map((f) => f.kind).join(", ")}`}
              error={failed[0].error}
              onRetry={state.retry}
              className="mb-2"
            />
          )}
          {truncated && (
            <Alert tone="info" title="Not every route was read" className="mb-2">
              More routes remain on the cluster. This list stops at the shared list row cap, so routes
              attached here past it are not shown.
            </Alert>
          )}
          <Table
            columns={ATTACHED_COLUMNS}
            data={rows}
            getRowKey={(r) => `${r.kind}/${r.namespace}/${r.name}`}
            emptyText={truncated ? "None among the routes read" : "No routes attach to this gateway"}
            onRowClick={open}
            onRowActivate={open}
          />
        </>
      )}
    </Section>
  );
}

function parentColumns(context: string): Column<RouteParent>[] {
  return [
    {
      key: "parent",
      header: "Parent",
      render: (p) =>
        p.kind === "Gateway" ? (
          <Opens
            label={`${p.namespace}/${p.name}`}
            route={detailRoute("Gateway", p.namespace, p.name, { cluster: context, group: GATEWAY_API_GROUP })}
            context={context}
          />
        ) : (
          // A Service parent is a mesh route; another kind is an implementation's own.
          mono(`${p.kind}/${p.namespace}/${p.name}`)
        ),
    },
    {
      key: "section",
      header: "Listener",
      render: (p) => mono([p.sectionName, p.port && `:${p.port}`].filter(Boolean).join(" ")),
    },
    { key: "accepted", header: "Accepted", render: (p) => p.accepted || "—" },
    { key: "resolved", header: "Resolved refs", render: (p) => p.resolvedRefs || "—" },
  ];
}

/** One backend: a Service is the way to it; anything else is named and no more. */
function Backend({ backend, routeNamespace, context }: { backend: RouteBackend; routeNamespace: string; context: string }) {
  const port = backend.port ? `:${backend.port}` : "";
  const elsewhere = backend.namespace !== routeNamespace ? `${backend.namespace}/` : "";
  const weight = backend.weight ? <span className="text-muted"> ×{backend.weight}</span> : null;
  return (
    <li className="flex items-center gap-1">
      {backend.isService ? (
        <Opens
          label={`${elsewhere}${backend.name}${port}`}
          route={detailRoute("Service", backend.namespace, backend.name, { cluster: context })}
          context={context}
        />
      ) : (
        mono(`${backend.kind}/${elsewhere}${backend.name}${port}`)
      )}
      {weight}
    </li>
  );
}

function ruleColumns(routeNamespace: string, context: string): Column<RouteRule>[] {
  return [
    { key: "matches", header: "Matches", render: (r) => mono(r.matches) },
    {
      key: "backends",
      header: "Backends",
      render: (r) =>
        r.backends.length === 0 ? (
          "—"
        ) : (
          <ul className="flex flex-col gap-0.5">
            {r.backends.map((backend, i) => (
              <Backend key={i} backend={backend} routeNamespace={routeNamespace} context={context} />
            ))}
          </ul>
        ),
    },
  ];
}

/**
 * The Gateway API's own blocks for a Gateway or a route (#1089), ahead of the
 * conditions and status every custom resource shows: how traffic gets in, and
 * where it is sent. Any other kind in the group adds nothing here.
 */
export function GatewayApiBody({ object, context }: { object: K8sObject; context: string }) {
  const kind = object.kind ?? "";
  if (kind === "Gateway") {
    return (
      <>
        <Section title="Listeners">
          <Table columns={LISTENER_COLUMNS} data={gatewayListeners(object)} getRowKey={(l) => l.name} emptyText="No listeners" />
        </Section>
        <GatewayRoutesSection object={object} context={context} />
      </>
    );
  }
  if (!ROUTE_KINDS.includes(kind)) return null;
  const namespace = str(object.metadata?.namespace);
  return (
    <>
      <Section title="Parents">
        <Table
          columns={parentColumns(context)}
          data={routeParents(object)}
          getRowKey={(p) => `${p.kind}/${p.namespace}/${p.name}/${p.sectionName}/${p.port}`}
          emptyText="Attached to nothing"
        />
      </Section>
      <Section title="Rules">
        <Table columns={ruleColumns(namespace, context)} data={routeRules(object)} getRowKey={(r) => r.key} emptyText="No rules" />
      </Section>
    </>
  );
}
