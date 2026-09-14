import { useContext, useEffect, useRef, useState } from "react";
import { listExtensionCatalog, reviewCatalogExtension, openExternal, type ExtensionCatalogSnapshot, type InstalledExtension } from "@srelens/core";
import { ExtensionControls } from "./ExtensionControls";

export function ExtensionCatalog({ developerMode, installed, onReview }: {
  developerMode: boolean;
  installed: InstalledExtension[];
  onReview: (manifest: string) => void;
}) {
  const { Button } = useContext(ExtensionControls);
  const [data, setData] = useState<ExtensionCatalogSnapshot>();
  const [query, setQuery] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const generation = useRef(0);
  useEffect(() => () => { generation.current++; }, []);
  async function run(action: () => Promise<void>) {
    setBusy(true); setError("");
    try { await action(); }
    catch (e) { setError(e instanceof Error ? e.message : String(e)); }
    finally { setBusy(false); }
  }
  function load(refresh: boolean) {
    const request = ++generation.current;
    void run(async () => {
      const result = await listExtensionCatalog(refresh);
      if (request === generation.current) setData(result);
    });
  }
  const entries = data?.catalog.extensions.filter(e => `${e.name} ${e.id} ${e.description}`.toLowerCase().includes(query.toLowerCase()));
  return <section aria-label="Extension catalog">
    <div className="extension-toolbar">
      <strong>Native extension catalog</strong>
      {data && <input className="extension-catalog-search" aria-label="Find an extension" placeholder="Find an extension…" value={query} onChange={e => setQuery(e.target.value)} />}
      <Button variant="secondary" disabled={busy} onClick={() => load(Boolean(data))}>{data ? "Refresh catalog" : "Browse catalog"}</Button>
    </div>
    {!data && <p className="extension-message">Discover native extensions from the srelens catalog. Each extension is maintained in its own repository.</p>}
    {busy && <p className="extension-message" role="status">Loading…</p>}
    {error && <p className="extension-error" role="alert">{error}</p>}
    {data && <>
      <p className="extension-message">{data.stale ? "Cached catalog" : "Catalog checked"} · {new Date(data.fetchedAt * 1000).toLocaleString()} · Host API {data.hostApiVersion}</p>
      {data.error && <p className="extension-warning" role="alert">Refresh failed: {data.error}. Showing the cached catalog.</p>}
      <p className="extension-message">Releases are unsigned. Review permissions before installing. {!developerMode && "Enable developer mode to install."}</p>
      {entries?.length === 0 && <p className="extension-message">No matching extensions.</p>}
      {entries?.map(entry => {
        const current = installed.find(p => p.manifest.id === entry.id);
        const incompatible = data.incompatible.includes(entry.id);
        return <article className="extension-installed" key={entry.id}>
          <div className="extension-toolbar">
            <strong>{entry.name}</strong>
            <span>{entry.release.version}{entry.release.prerelease ? " · Preview" : ""} · {entry.license}</span>
            <Button variant="secondary" disabled={busy} onClick={() => void run(() => openExternal(entry.repository))}>Repository</Button>
            <Button disabled={busy || !developerMode || incompatible} onClick={() => {
              const request = ++generation.current;
              void run(async () => {
                const result = await reviewCatalogExtension(entry.id, entry.release.sha256);
                if (request === generation.current) onReview(result.manifest);
              });
            }}>{current ? "Review replacement" : "Review installation"}</Button>
          </div>
          <p className="extension-message">{entry.description}</p>
          <p className="extension-message">{entry.id} · Requires API {entry.release.srelensApiVersion}{incompatible ? " · Incompatible with this app" : ""}{current ? ` · Installed ${current.manifest.version}` : ""}</p>
        </article>;
      })}
    </>}
  </section>;
}
