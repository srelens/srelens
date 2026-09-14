import { ExtensionRequirements } from "./ExtensionRequirements";
import { ExtensionLogo } from "./ExtensionLogo";
import { useContext, useState } from "react";
import {
  configureExtensions,
  contributionKind,
  isTauri,
  type ExtensionChange,
  type ExtensionContribution,
  type InstalledExtension,
} from "@srelens/core";

import { ExtensionCatalog } from "./ExtensionCatalog";
import { ExtensionControls } from "./ExtensionControls";
export { ExtensionControlsProvider } from "./ExtensionControls";
import { ErrorNotice, ExtensionResults } from "./ExtensionResults";
export { ExtensionResults } from "./ExtensionResults";


import { useExtensions } from "./inventoryStore";
export { useExtensions } from "./inventoryStore";

export function ExtensionManager() {
  const { Button, Tabs } = useContext(ExtensionControls);
  const [tab, setTab] = useState("installed");
  const [catalogOpened, setCatalogOpened] = useState(false);
  const inventory = useExtensions();
  const [source, setSource] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [removing, setRemoving] = useState<InstalledExtension | null>(null);
  const [review, setReview] = useState<{
    source: string;
    signature?: number[];
    name: string;
    permissions: string[];
  } | null>(null);
  const [settings, setSettings] = useState<{ id: string; text: string } | null>(
    null,
  );
  async function change(action: ExtensionChange) {
    setBusy(true);
    setError("");
    try {
      await configureExtensions(action);
      inventory.reload();
      setReview(null);
      if (action.action === "install") setTab("installed");
      return true;
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      return false;
    } finally {
      setBusy(false);
    }
  }
  if (!isTauri())
    return (
      <p className="extension-message">
        Local apps are available in the desktop app.
      </p>
    );
  if (inventory.status === "loading")
    return (
      <p role="status" className="extension-message">
        Loading apps…
      </p>
    );
  if (inventory.status === "error")
    return <ErrorNotice message={inventory.error} retry={inventory.reload} />;
  const state = inventory.data!;
  return (
    <div className="extension-manager">
      <div className="extension-toolbar">
        <strong>Apps</strong>
        <Button variant="secondary" onClick={inventory.reload}>
          Refresh
        </Button>
      </div>
      {error && (
        <p role="alert" className="extension-error">
          {error}
        </p>
      )}
      <Tabs variant="underline" label="App settings" tabs={[
        { id: "installed", label: "Apps" },
        { id: "catalog", label: "Catalog" },
      ]} active={tab} onChange={(next) => { setTab(next); if (next === "catalog") setCatalogOpened(true); }} />
        {review && (
          <section className="extension-install extension-permission-review" aria-label="Review app permissions">
            <p>
              <strong>{review.name}</strong> ({review.signature ? "Signature verified · srelens" : "Unsigned local manifest"}) requests:{" "}
              {review.permissions.join(", ")}. Installing an existing ID
              replaces its manifest and refreshes its open pages.
            </p>
            <Button
              disabled={busy}
              onClick={() =>
                void change({
                  action: "install",
                  manifest: review.source,
                  ...(review.signature ? {signature: review.signature} : {}),
                  grants: review.permissions,
                })
              }
            >
              Install and grant permissions
            </Button>
            <Button variant="secondary" onClick={() => setReview(null)}>
              Cancel
            </Button>
          </section>
        )}
      <div hidden={tab !== "catalog"}>
        {catalogOpened && (
      <ExtensionCatalog autoLoad installed={state.plugins} onReview={(manifest, signature) => {
        const parsed = JSON.parse(manifest);
        setReview({ source: manifest, signature, name: parsed.name, permissions: parsed.permissions });
        setError("");
      }} />
        )}
      </div>
      <div hidden={tab !== "installed"}>
      <details className="extension-local-tools">
        <summary>Install a local manifest</summary>
      <div className="extension-install">
        <label htmlFor="extension-manifest">
          Local app manifest (JSON)
        </label>
        <textarea
          id="extension-manifest"
          value={source}
          onChange={(e) => {
            setSource(e.target.value);
            setReview(null);
          }}
          spellCheck={false}
          rows={5}
          disabled={busy}
        />
        <Button
          disabled={!source.trim() || busy}
          onClick={() => {
            try {
              const m = JSON.parse(source);
              if (
                typeof m.name !== "string" ||
                !Array.isArray(m.permissions) ||
                !m.permissions.every((p: unknown) => typeof p === "string")
              )
                throw new Error("Manifest must declare a name and permissions");
              setReview({ source, name: m.name, permissions: m.permissions });
              setError("");
            } catch (e) {
              setError(e instanceof Error ? e.message : String(e));
            }
          }}
        >
          Review manifest
        </Button>
      </div>
      </details>

      <p className="extension-message extension-catalog-meta">Apps are installed app-wide and available across clusters. Each page checks the APIs it needs when opened.</p>
      {state.plugins.length === 0 && (
        <p className="extension-message">No apps installed.</p>
      )}
      {state.plugins.map((plugin) => (
        <section className="extension-installed" key={plugin.manifest.id}>
          <div className="extension-toolbar">
            <ExtensionLogo id={plugin.manifest.id} name={plugin.manifest.name} size={24} />
            <strong>{plugin.manifest.name}</strong>
            <span>{plugin.manifest.version} · {!plugin.signatureProof ? "Unsigned local" : plugin.quarantined ? "Signature not verified" : "Signed by srelens"}</span>
            <label>
              <input
                aria-label={`Enable ${plugin.manifest.name}`}
                type="checkbox"
                checked={plugin.enabled}
                disabled={busy || Boolean(plugin.quarantined)}
                onChange={(e) =>
                  void change({
                    action: "enable",
                    id: plugin.manifest.id,
                    enabled: e.target.checked,
                  })
                }
              />{" "}
              Enabled
            </label>
            <Button
              variant="secondary"
              disabled={busy}
              onClick={() =>
                setSettings({
                  id: plugin.manifest.id,
                  text: JSON.stringify(plugin.settings, null, 2),
                })
              }
            >
              Settings
            </Button>
            <Button
              variant="danger"
              disabled={busy}
              onClick={() =>
                setRemoving(plugin)
              }
            >
              Remove
            </Button>
          </div>
          <p className="extension-message">{plugin.manifest.id}</p>
          {plugin.quarantined && (
            <p className="extension-error">
              Disabled: {plugin.quarantined}. Remove it or reinstall it from the Catalog.
            </p>
          )}
        </section>
      ))}
      {removing && (
        <section className="extension-install" role="alertdialog" aria-label="Remove app" onKeyDown={e=>{if(e.key==="Escape" && !busy)setRemoving(null);}}>
          <strong>Remove {removing.manifest.name}?</strong>
          <p>This removes the app and its saved settings.</p>
          <Button variant="secondary" autoFocus disabled={busy} onClick={()=>setRemoving(null)}>Cancel</Button>
          <Button variant="danger" disabled={busy} onClick={()=>{void change({action:"remove",id:removing.manifest.id}).then(removed=>{if(removed)setRemoving(null);});}}>Remove app</Button>
        </section>
      )}
      {settings && (
        <section className="extension-install">
          <label htmlFor="extension-settings">
            App settings (JSON object)
          </label>
          <textarea
            id="extension-settings"
            rows={4}
            value={settings.text}
            onChange={(e) => setSettings({ ...settings, text: e.target.value })}
          />
          <Button
            disabled={busy}
            onClick={() => {
              try {
                const value = JSON.parse(settings.text);
                if (!value || Array.isArray(value) || typeof value !== "object")
                  throw new Error("Settings must be a JSON object");
                void change({
                  action: "settings",
                  id: settings.id,
                  settings: value,
                });
              } catch (e) {
                setError(e instanceof Error ? e.message : String(e));
              }
            }}
          >
            Save settings
          </Button>
          <Button variant="secondary" onClick={() => setSettings(null)}>
            Close
          </Button>
        </section>
      )}
      </div>
    </div>
  );
}
export function useExtensionContributions(kind: string, group?: string) {
  const inventory = useExtensions();
  const plugins = inventory.data?.plugins.filter((p) => p.enabled) ?? [];
  const qualified = contributionKind(kind, group);
  return {
    inventory,
    tabs: plugins.flatMap((plugin) =>
      plugin.manifest.contributions.detailTabs
        .filter((c) => c.forKinds?.includes(qualified))
        .map((contribution) => ({
          plugin,
          contribution,
          id: `extension:${plugin.manifest.id}/${contribution.id}`,
        })),
    ),
    actions: plugins.flatMap((plugin) =>
      plugin.manifest.contributions.rowActions
        .filter((c) => c.forKinds?.includes(qualified))
        .map((contribution) => ({
          plugin,
          contribution,
          id: `extension:${plugin.manifest.id}/${contribution.id}`,
        })),
    ),
  };
}
/** A shared resource contribution slot, used by both desktop designs. */
export function ExtensionResourceSlot({
  context,
  kind,
  group,
  namespace,
  name,
}: {
  context: string;
  kind: string;
  group?: string;
  namespace: string | null;
  name: string;
}) {
  const { Button, Tabs } = useContext(ExtensionControls);
  const { inventory, tabs, actions } = useExtensionContributions(kind, group);
  const [selected, setSelected] = useState("");
  const active = [...tabs, ...actions].some((c) => c.id === selected)
    ? selected
    : tabs[0]?.id || "";
  const current = [...tabs, ...actions].find((c) => c.id === active);
  const panelTabs =
    current && !tabs.some((c) => c.id === current.id)
      ? [...tabs, current]
      : tabs;
  if (inventory.status === "error")
    return <ErrorNotice message={inventory.error} retry={inventory.reload} />;
  if (!tabs.length && !actions.length) return null;
  const ns = contributionKind(kind, group) === "/Namespace" ? name : (namespace ?? "");
  return (
    <section className="extension-installed extension-resource-slot">
      <div className="extension-toolbar">
        <strong>Apps</strong>
        {actions.length > 0 && (
          <details>
            <summary>App actions</summary>
            {actions.map((c) => (
              <Button
                variant="secondary"
                key={c.id}
                onClick={() => setSelected(c.id)}
              >
                {c.contribution.title}
              </Button>
            ))}
          </details>
        )}
      </div>
      {panelTabs.length > 0 && (
        <Tabs
          variant="underline"
          label="App views"
          tabs={panelTabs.map((c) => ({
            id: c.id,
            label: c.contribution.title,
          }))}
          active={active}
          onChange={setSelected}
        />
      )}
      {current && inventory.status !== "loading" && (
        <ExtensionRequirements plugin={current.plugin} page={current.contribution} context={context} refresh={0}>
        <ExtensionResults
          key={`${context}/${kind}/${namespace}/${name}/${current.id}`}
          plugin={current.plugin}
          capability={current.contribution.capability}
          context={context}
          namespace={ns}
        />
        </ExtensionRequirements>
      )}
    </section>
  );
}

export { ExtensionWorkspace } from "./ExtensionWorkspace";
export { ExtensionLogo } from "./ExtensionLogo";

export { ExtensionResourceNavigation } from "./resourceNavigation";
export { ExtensionResourceDetails } from "./ExtensionResourceDetails";
