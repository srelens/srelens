import { type ReactNode } from "react";
import { listCrds, type ExtensionPage, type InstalledExtension } from "@srelens/core";
import { useResource } from "../lib/useResource";
import { Button } from "@srelens/ui-kit";
import { FailureAlert } from "../lib/errorCopy";
import { crdRequirements, pageReaders, requirementStatus } from "./requirements";

/** Installation is global; only the APIs needed by this view are cluster-specific. */
export function ExtensionRequirements({ plugin, page, context, refresh, children }: {
  plugin: InstalledExtension;
  page: ExtensionPage;
  context: string;
  refresh: number;
  children: ReactNode;
}) {
  const required = crdRequirements(plugin, pageReaders(plugin, page));
  const key = JSON.stringify([context, plugin.manifest.id, plugin.revision, required]);
  const result = useResource(async () => {
    if (!context || !required.length) return null;
    try {
      const discovered = await listCrds(context);
      return { key, ...discovered, error: discovered.error || (!discovered.crds ? "Cluster discovery returned no CRD list." : undefined) };
    } catch (e) { return { key, error: String(e), crds: undefined }; }
  }, [key, refresh]);
  if (!context || !required.length) return <>{children}</>;
  if (result.status === "loading" || result.data?.key !== key)
    return <p className="extension-message" role="status">Checking app requirements…</p>;
  const error = result.data?.error;
  const requirements = required.map(r => {
    const { resolved, status } = requirementStatus(r, result.data?.crds ?? []);
    return { ...r, resolved, status: error ? "Not verified" : status };
  });
  const missing = !error && requirements.some(r => r.status !== "Available");
  if (!missing && !error) return <>{children}</>;
  return <>
    <section className="extension-requirements" aria-label={`${plugin.manifest.name} requirements`}>
      {error ? <FailureAlert tone="sev" title="Could not check requirements" detail="CRD discovery failed. This does not mean the APIs are missing; resource reads will still use your existing permissions." error={error} onRetry={result.reload} /> : <div className="extension-toolbar">
        <strong>Missing requirements</strong>
        <Button variant="secondary" onClick={result.reload}>Check again</Button>
      </div>}
      <p className="extension-message">{plugin.manifest.name} is installed for the whole app. This page requires the following APIs on the selected cluster. {!error && "Install the corresponding operator/CRDs, or select a cluster that provides them. "}Installing the extension does not change your cluster.</p>
      <div className="extension-table-scroll extension-results">
        <table><thead><tr><th>Required CRD</th><th>Accepted API versions</th><th>Reads</th><th>Status</th></tr></thead>
          <tbody>{requirements.map(r => <tr key={`${r.group}/${r.versions.join()}/${r.plural}/${r.namespaced}`}>
            <td>{r.plural}.{r.group}</td>
            <td>{r.group}/{r.versions.join(", ")}{r.versions.length > 1 && <>{" "}<span className="extension-value" data-tone="muted">(in order of preference)</span></>}</td>
            <td aria-label="Reads">{error ? "Not verified" : r.resolved ?? "None served"}</td>
            <td>{r.status}</td>
          </tr>)}</tbody>
        </table>
      </div>
    </section>
    {!missing && children}
  </>;
}
