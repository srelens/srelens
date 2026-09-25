import { ExtensionLogo } from "./ExtensionLogo";
import { useContext, useEffect, useRef, useState } from "react";
import { isTauri, listExtensionCatalog, reviewCatalogExtension, openExternal, type ExtensionCatalogSnapshot, type InstalledExtension } from "@srelens/core";
import { ExtensionControls } from "./ExtensionControls";

export function ExtensionCatalog({ installed, onReview, autoLoad = false }: {
  autoLoad?: boolean;
  installed: InstalledExtension[];
  onReview: (manifest: string, signature?: number[]) => void;
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
  useEffect(() => { if (autoLoad) load(false); }, [autoLoad]);
  const entries = data?.catalog.extensions.filter(e => `${e.name} ${e.id} ${e.description}`.toLowerCase().includes(query.toLowerCase()));
  return <section className="extension-catalog" aria-label="App catalog">
    <div className="extension-toolbar">
      <strong>Available apps</strong>
      {data && <input className="extension-catalog-search" aria-label="Find an app" placeholder="Find an app…" value={query} onChange={e => setQuery(e.target.value)} />}
      <Button variant="secondary" disabled={busy} onClick={() => load(Boolean(data))}>{data ? "Refresh catalog" : "Browse catalog"}</Button>
    </div>
    {!data && <p className="extension-message">Discover native apps from the srelens catalog. Each app is maintained in its own repository.</p>}
    {busy && <p className="extension-message" role="status">Loading…</p>}
    {error && <p className="extension-error" role="alert">{error}</p>}
    {data && <>
      <p className="extension-message extension-catalog-meta">{data.stale ? "Cached catalog" : "Catalog checked"} · {new Date(data.fetchedAt * 1000).toLocaleString()} · Host API {data.hostApiVersions.join(", ")}</p>
      {/* The web server shares one catalog between its users and fetches it itself (#515), so a
          Refresh here shows its copy rather than fetching one: say so, or the time above reads as wrong. */}
      {!isTauri() && <p className="extension-message">This server keeps one catalog for everyone who signs in, and fetches it again once a day.</p>}
      {data.error && <p className="extension-warning" role="alert">Refresh failed: {data.error}. Showing the cached catalog.</p>}
      <p className="extension-message">Official srelens app signatures are verified before installation review. Review permissions before installing.</p>
      {entries?.length === 0 && <p className="extension-message">No matching apps.</p>}
      {entries?.map(entry => {
        const current = installed.find(p => p.manifest.id === entry.id);
        const incompatible = data.incompatible.includes(entry.id);
        return <article className="extension-installed extension-catalog-entry" aria-label={entry.name} key={entry.id}>
          <div className="extension-toolbar">
            <ExtensionLogo id={entry.id} name={entry.name} size={28} />
            <div className="extension-catalog-description">
              <strong>{entry.name}</strong>
              <p>{entry.description}</p>
              <p className="extension-catalog-meta">{entry.id} · Requires API {entry.release.srelensApiVersion}{incompatible ? " · Incompatible with this app" : ""}{current ? ` · Installed ${current.manifest.version}` : ""}</p>
            </div>
            <span className="extension-catalog-meta">{entry.release.version}{entry.release.prerelease ? " · Preview" : ""} · {entry.license}</span>
            <Button variant="secondary" disabled={busy} onClick={() => void run(() => openExternal(entry.repository))}>Repository</Button>
            <Button disabled={busy || incompatible} onClick={() => {
              const request = ++generation.current;
              void run(async () => {
                const result = await reviewCatalogExtension(entry.id, entry.release.sha256);
                if (request === generation.current) onReview(result.manifest, result.signature ?? undefined);
              });
            }}>{current ? "Review replacement" : "Review installation"}</Button>
          </div>
        </article>;
      })}
    </>}
  </section>;
}
