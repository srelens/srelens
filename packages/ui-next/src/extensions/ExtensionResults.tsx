import { useContext } from "react";
import {
  describeError,
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
  const data = useResource(
    async () =>
      context
        ? readExtension(
            plugin.manifest.id,
            plugin.revision,
            capability,
            context,
            namespace,
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
  const binding = plugin.manifest.capabilities.find(
    (b) => b.name === capability,
  );
  const columns = Array.isArray(binding?.arguments.printerColumns)
    ? (binding.arguments.printerColumns as Array<{ name: string }>)
    : [];
  if (!context)
    return (
      <p className="extension-message">
        Choose a cluster before opening an extension page.
      </p>
    );
  if (data.status === "error") {
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
  if (data.status === "loading")
    return (
      <p role="status" className="extension-message">
        Loading extension resources…
      </p>
    );
  const rows = (data.data?.items ?? []).filter((row) =>
    [row.name, row.namespace, ...row.columns]
      .join(" ")
      .toLowerCase()
      .includes(search.toLowerCase()),
  );
  return (
    <section className="extension-results">
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
      {rows.length ? (
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
                <tr key={`${row.namespace}/${row.name}`}>
                  <td>{row.name}</td>
                  <td>{row.namespace || "—"}</td>
                  {columns.map((_, i) => (
                    <td key={i}>{row.columns?.[i] || "—"}</td>
                  ))}
                  <td>{row.age}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : (
        <p className="extension-message">
          No resources returned by this extension.
        </p>
      )}
    </section>
  );
}
