import { AgeCell } from "../lib/ageCell";
import { ExtensionResourceDetails } from "./ExtensionResourceDetails";
import { ResizeHandle } from "@srelens/ui-kit";
import { clampPeekWidth, savePeekWidth, setPeekWidth, usePeekBounds, usePeekWidth } from "../lib/peekWidth";
import { ExtensionResourceNavigation } from "./resourceNavigation";
import { useContext, useEffect, useRef, useState } from "react";
import {
  describeError,
  onExtensionResourceChanged,
  readExtension,
  type InstalledExtension,
} from "@srelens/core";
import { ExtensionControls } from "./ExtensionControls";
import { useResource } from "../lib/useResource";

export function ErrorNotice({
  message,
  retry,
  cluster = false,
  guidance,
}: {
  message?: string;
  retry: () => void;
  cluster?: boolean;
  guidance?: { title: string; detail: string };
}) {
  const { Button } = useContext(ExtensionControls);
  const error = describeError(message, {
    domain: cluster ? "cluster" : "local",
  });
  return (
    <div className="extension-error" role="alert">
      <div>
        <strong>{guidance?.title ?? error.title}</strong>
        <p>{guidance?.detail ?? error.detail}</p>
        {error.raw !== (guidance?.detail ?? error.detail) && (
          <details>
            <summary>Original error</summary>
            <pre>{error.raw}</pre>
          </details>
        )}
      </div>
      <Button variant="secondary" onClick={retry}>
        Retry
      </Button>
    </div>
  );
}
function ResultValue({ value, column }: { value?: string; column: string }) {
  const raw = value || "—";
  const boolean = raw.toLowerCase();
  let label = raw;
  let tone = "neutral";
  if (column === "Ready" && (boolean === "true" || boolean === "false")) {
    label = boolean === "true" ? "Ready" : "Not ready";
    tone = boolean === "true" ? "ready" : "warning";
  } else if (
    ["Suspended", "Reconciling"].includes(column) &&
    ["true", "false"].includes(boolean)
  ) {
    label =
      boolean === "false"
        ? "No"
        : column === "Reconciling"
          ? "In progress"
          : "Yes";
    tone = boolean === "true" ? "warning" : "muted";
  }
  const revision = raw.match(/^(.*?)@sha(?:1|256):([a-f0-9]{16,})$/i);
  if (revision) label = `${revision[1]}@${revision[2].slice(0, 8)}`;
  return (
    <span
      className="extension-value"
      data-tone={tone}
      title={raw}
      aria-label={revision ? raw : undefined}
    >
      {label}
    </span>
  );
}

export function ExtensionResults({
  plugin,
  capability,
  context,
  namespace = "",
  search = "",
  refresh = 0,
  hideToolbar = false,
}: {
  plugin: InstalledExtension;
  capability: string;
  context: string;
  namespace?: string;
  search?: string;
  refresh?: number;
  hideToolbar?: boolean;
}) {
  const { Button } = useContext(ExtensionControls);
  const openResource = useContext(ExtensionResourceNavigation);
  const rowButtons = useRef(new Map<string,HTMLButtonElement>());
  const listRow = usePeekBounds();
  const peekWidth = clampPeekWidth(usePeekWidth(), listRow.bounds);
  const scope = JSON.stringify([plugin.manifest.id,plugin.revision,capability,context,namespace]);
  const [selected,setSelected] = useState<{scope:string;name:string;namespace:string}|null>(null);
  const data = useResource(
    async () =>
      context
        ? readExtension(
            plugin.manifest.id,
            plugin.revision,
            capability,
            context,
            namespace,
            true,
          )
        : null,
    [
      plugin.manifest.id,
      plugin.revision,
      capability,
      context,
      namespace,
      refresh,
    ],
  );
  const { reload } = data;
  // Refresh when an action on one of this list's resources is accepted, from any view.
  useEffect(
    () =>
      onExtensionResourceChanged((changed) => {
        if (
          changed.id === plugin.manifest.id &&
          changed.context === context &&
          changed.capability === capability &&
          (!namespace || changed.namespace === namespace)
        )
          reload();
      }),
    [plugin.manifest.id, context, capability, namespace, reload],
  );
  const binding = plugin.manifest.capabilities.find(
    (b) => b.name === capability,
  );
  const columns = data.data?.printerColumns ?? (Array.isArray(binding?.arguments.printerColumns)
    ? (binding.arguments.printerColumns as Array<{ name: string }>)
    : []);
  if (!context)
    return (
      <p className="extension-message">
        Choose a cluster before opening an app page.
      </p>
    );
  if (data.status === "error" && selected?.scope !== scope) {
    // A 404 identifies an unavailable endpoint, not why it is unavailable.
    // Name the required API without claiming that discovery proved it absent.
    const args = binding?.arguments;
    const notFound =
      /\bApiError:\s*404\b|\bcode:\s*404\b|\b404 page not found\b/i.test(
        data.error ?? "",
      );
    const guidance =
      notFound &&
      typeof args?.group === "string" &&
      typeof args.version === "string" &&
      typeof args.plural === "string" &&
      typeof args.kind === "string"
        ? {
            title: `${args.kind} API unavailable`,
            detail: `This extension reads ${args.plural} from ${args.group}/${args.version}. Check that the selected cluster serves this API version. Installing an extension does not install its Kubernetes APIs.`,
          }
        : undefined;
    return (
      <ErrorNotice
        cluster
        message={data.error}
        retry={data.reload}
        guidance={guidance}
      />
    );
  }
  if (data.status === "loading" && selected?.scope !== scope)
    return (
      <p role="status" className="extension-message">
        Loading app resources…
      </p>
    );
  const rows = (data.data?.items ?? []).filter((row) =>
    [row.name, row.namespace, ...row.columns]
      .join(" ")
      .toLowerCase()
      .includes(search.toLowerCase()),
  );
  return (
    <section className="extension-results" ref={listRow.ref}>
      <div className="extension-resource-list">
      {data.data?.columnsError && <p className="extension-message" role="status">Could not load CRD columns: {data.data.columnsError}. Showing app-defined columns.</p>}
      {!hideToolbar && (
        <div className="extension-toolbar">
          <span>
            {binding?.arguments.namespaced === false
              ? "Cluster-scoped resources"
              : namespace
                ? `Namespace: ${namespace}`
                : "All namespaces"}
          </span>
          <Button variant="secondary" onClick={data.reload}>
            Refresh
          </Button>
        </div>
      )}
      {data.status === "loading" ? <p className="extension-message" role="status">Refreshing resources…</p> : data.status === "error" ? <ErrorNotice cluster message={data.error} retry={data.reload}/> : rows.length ? (
        <div className="extension-table-scroll">
          <table>
            <thead>
              <tr>
                <th>Name</th>
                <th>Namespace</th>
                {columns.map((c, i) => (
                  <th key={i}>{c.name}</th>
                ))}
                <th>Age</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((row) => (
                <tr key={`${row.namespace}/${row.name}`} aria-selected={selected?.scope===scope && selected.name===row.name && selected.namespace===row.namespace} onDoubleClick={openResource && binding?.target === "k8s.listCustomResource" ? ()=>openResource({id:plugin.manifest.id,revision:plugin.revision,capability,context,namespace:row.namespace,name:row.name}):undefined} onClick={binding?.target === "k8s.listCustomResource" ? ()=>setSelected({scope,name:row.name,namespace:row.namespace}):undefined}>
                  <td>
                    {binding?.target === "k8s.listCustomResource" ? <button className="extension-resource-link" ref={node=>{const key=`${row.namespace}/${row.name}`;if(node)rowButtons.current.set(key,node);else rowButtons.current.delete(key);}} onKeyDown={e=>{if(e.key==="Enter" && openResource){e.preventDefault();openResource({id:plugin.manifest.id,revision:plugin.revision,capability,context,namespace:row.namespace,name:row.name});}}} onClick={()=>setSelected({scope,name:row.name,namespace:row.namespace})}>{row.name}</button> : <span className="extension-resource-name" title={row.name}>{row.name}</span>}
                  </td>
                  <td className="extension-namespace">
                    {row.namespace || "—"}
                  </td>
                  {columns.map((column, i) => (
                    <td key={i}>
                      <ResultValue
                        value={row.columns?.[i]}
                        column={column.name}
                      />
                    </td>
                  ))}
                  <td><AgeCell created={row.created} age={row.age} /></td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : (
        <p className="extension-message">
          {data.data?.items.length ? "No matching resources." : "No resources returned by this app."}
        </p>
      )}
      </div>
      {selected?.scope === scope && <div className="extension-detail-peek" style={{width:peekWidth}}>
        <ResizeHandle label="the resource details" width={peekWidth} minWidth={listRow.bounds.minWidth} maxWidth={listRow.bounds.maxWidth} edge="left" onResize={setPeekWidth} onCommit={savePeekWidth}/>
        <ExtensionResourceDetails key={`${scope}/${selected.namespace}/${selected.name}`} selection={{id:plugin.manifest.id,revision:plugin.revision,capability,context,namespace:selected.namespace,name:selected.name}} onClose={()=>{setSelected(null);rowButtons.current.get(`${selected.namespace}/${selected.name}`)?.focus();}} />
      </div>}
    </section>
  );
}
