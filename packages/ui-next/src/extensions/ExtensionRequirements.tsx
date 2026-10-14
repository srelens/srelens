import { useContext, type ReactNode } from "react";
import { listCrds, type ExtensionPage, type InstalledExtension } from "@srelens/core";
import { useResource } from "../lib/useResource";
import { ExtensionControls } from "./ExtensionControls";
import { ErrorNotice } from "./ExtensionResults";

/** Installation is global; only the APIs needed by this view are cluster-specific. */
export function ExtensionRequirements({ plugin, page, context, refresh, children }: {
  plugin: InstalledExtension;
  page: ExtensionPage;
  context: string;
  refresh: number;
  children: ReactNode;
}) {
  const { Button } = useContext(ExtensionControls);
  const capabilities = new Set([page.capability, ...(page.dashboard?.pages ?? []).flatMap(id =>
    plugin.manifest.contributions.pages.filter(p => p.id === id).map(p => p.capability))]);
  // A reader accepts its `versions` in preference order, or the one `arguments.version` it
  // fixes (#547); the host reads the first one the cluster serves, and nothing else.
  const required = plugin.manifest.capabilities.filter(b => capabilities.has(b.name) && b.target === "k8s.listCustomResource")
    .map(b => ({ group: String(b.arguments.group), versions: b.versions?.length ? b.versions : [String(b.arguments.version)], plural: String(b.arguments.plural), kind: String(b.arguments.kind), namespaced: b.arguments.namespaced === true }))
    .filter((r, i, rows) => rows.findIndex(other => other.group === r.group && other.versions.join() === r.versions.join() && other.plural === r.plural && other.namespaced === r.namespaced) === i);
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
    const crd = result.data?.crds?.find(c => c.group === r.group && c.plural === r.plural && c.kind === r.kind);
    const served = crd ? (crd.versions ?? [crd.version]) : [];
    const resolved = r.versions.find(version => served.includes(version));
    const status = error ? "Not verified" : !crd ? "Missing CRD" : !resolved ? "Required version unavailable" : crd.namespaced !== r.namespaced ? "Scope mismatch" : "Available";
    return { ...r, resolved, status };
  });
  const missing = !error && requirements.some(r => r.status !== "Available");
  if (!missing && !error) return <>{children}</>;
  return <>
    <section className="extension-requirements" aria-label={`${plugin.manifest.name} requirements`}>
      {error ? <ErrorNotice cluster message={error} retry={result.reload} guidance={{ title: "Could not check requirements", detail: "CRD discovery failed. This does not mean the APIs are missing; resource reads will still use your existing permissions." }} /> : <div className="extension-toolbar">
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
