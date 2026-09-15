import { ExtensionDetails } from "./ExtensionDetails";
import { ExtensionRequirements } from "./ExtensionRequirements";
import { SHARED_CONTEXT_ID_MESSAGE, refreshContextIds, useContextLookup } from "./contextIds";
import { ExtensionLogo } from "./ExtensionLogo";
import { useContext, useState } from "react";
import {
  configureExtensions,
  contributionKind,
  extensionEnabledFor,
  isTauri,
  validateExtension,
  type ExtensionChange,
  type ExtensionValidationError,
  type InstalledExtension,
} from "@srelens/core";

import { ExtensionCatalog } from "./ExtensionCatalog";
import { ExtensionControls } from "./ExtensionControls";
export { ExtensionControlsProvider } from "./ExtensionControls";
import { ErrorNotice, ExtensionResults } from "./ExtensionResults";
export { ErrorNotice, ExtensionResults } from "./ExtensionResults";


import { useExtensions } from "./inventoryStore";
export { useExtensions } from "./inventoryStore";
export { SHARED_CONTEXT_ID_MESSAGE, refreshContextIds, useContextId, useContextLookup } from "./contextIds";

export function ExtensionManager() {
  const { Button, Tabs } = useContext(ExtensionControls);
  const [tab, setTab] = useState("installed");
  const [catalogOpened, setCatalogOpened] = useState(false);
  const inventory = useExtensions();
  const [source, setSource] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [removing, setRemoving] = useState<InstalledExtension | null>(null);
  /** The ID of the app whose details are open. */
  const [details, setDetails] = useState<string | null>(null);
  const [review, setReview] = useState<{
    source: string;
    signature?: number[];
    name: string;
    permissions: string[];
    /** Undefined while the host is still checking the manifest. */
    errors?: ExtensionValidationError[];
    /** Why the check itself failed, as opposed to the problems it found. */
    checkError?: string;
    /**
     * This review's identity. The same bytes can be reviewed signed and unsigned, so a
     * check that answers late is applied only to the review that asked for it.
     */
    request: object;
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
  /** Opens the permission review, and offers to install only once the host finds no problems. */
  async function reviewManifest(manifest: string, signature?: number[]) {
    let parsed: { name?: unknown; permissions?: unknown } = {};
    try {
      const value: unknown = JSON.parse(manifest);
      if (value && typeof value === "object") parsed = value as typeof parsed;
    } catch {
      // The host reports invalid JSON with a code and path, like any other problem.
    }
    const permissions =
      Array.isArray(parsed.permissions) && parsed.permissions.every((p) => typeof p === "string")
        ? (parsed.permissions as string[])
        : [];
    const name = typeof parsed.name === "string" ? parsed.name : "This manifest";
    const request = {};
    setError("");
    setReview({ request, source: manifest, signature, name, permissions });
    try {
      const { errors } = await validateExtension(manifest, permissions, signature);
      setReview((current) => (current?.request === request ? { ...current, errors } : current));
    } catch (e) {
      // The check did not run, which says nothing about the manifest: keep the review
      // open with the reason and a retry, and do not offer to install.
      const checkError = e instanceof Error ? e.message : String(e);
      setReview((current) => (current?.request === request ? { ...current, checkError } : current));
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
              {review.permissions.join(", ") || "no permissions"}. Installing an existing ID
              replaces its manifest and refreshes its open pages.
            </p>
            {review.checkError ? (
              <ErrorNotice
                title="Could not check the manifest"
                message={review.checkError}
                retry={() => void reviewManifest(review.source, review.signature)}
              />
            ) : !review.errors ? (
              <p role="status" className="extension-message">Checking the manifest…</p>
            ) : review.errors.length > 0 ? (
              <div className="extension-problems">
                <p>
                  Fix {review.errors.length === 1 ? "this problem" : `these ${review.errors.length} problems`} in the manifest before installing:
                </p>
                <ul aria-label="Manifest problems">
                  {review.errors.map((problem, index) => (
                    <li key={`${index}:${problem.code}:${problem.path}`}>
                      <code className="extension-problem-path">{problem.path || "manifest"}</code> {problem.message}{" "}
                      <span className="extension-problem-code">{problem.code}</span>
                    </li>
                  ))}
                </ul>
              </div>
            ) : (
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
            )}
            <Button variant="secondary" onClick={() => setReview(null)}>
              Cancel
            </Button>
          </section>
        )}
      <div hidden={tab !== "catalog"}>
        {catalogOpened && (
      <ExtensionCatalog autoLoad installed={state.plugins} onReview={(manifest, signature) => void reviewManifest(manifest, signature)} />
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
          onClick={() => void reviewManifest(source)}
        >
          Review manifest
        </Button>
      </div>
      </details>

      <p className="extension-message extension-catalog-meta">Apps are installed app-wide and are available on every cluster unless an app's Details limit it to chosen clusters. Each page checks the APIs it needs when opened.</p>
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
              aria-label={`Details for ${plugin.manifest.name}`}
              aria-expanded={details === plugin.manifest.id}
              onClick={() => setDetails(details === plugin.manifest.id ? null : plugin.manifest.id)}
            >
              Details
            </Button>
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
          {details === plugin.manifest.id && (
            <ExtensionDetails plugin={plugin} busy={busy} change={change} onError={setError} />
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
/** The detail tabs and detail links apps offer for a kind, on a cluster they are enabled for. */
export function useExtensionContributions(context: string, kind: string, group?: string) {
  const inventory = useExtensions();
  // App scope keys on the context's stable ID, not its name (#265).
  const lookup = useContextLookup(context);
  const contextId = lookup.status === "found" ? lookup.id : undefined;
  const qualified = contributionKind(kind, group);
  const enabled = inventory.data?.plugins.filter((p) => p.enabled) ?? [];
  const offersHere = (plugin: InstalledExtension) =>
    [...plugin.manifest.contributions.detailTabs, ...plugin.manifest.contributions.detailLinks].some((c) =>
      c.forKinds?.includes(qualified),
    );
  const plugins = enabled.filter((p) => extensionEnabledFor(p, contextId));
  return {
    inventory,
    /**
     * Why a limited app that offers something here cannot be checked: the clusters could
     * not be listed (with a retry), or this cluster shares its ID with another.
     */
    lookupProblem:
      lookup.status === "failed" && enabled.some((p) => p.contexts && offersHere(p))
        ? { title: "Could not list clusters", message: lookup.error, retry: () => void refreshContextIds() }
        : lookup.status === "shared" && enabled.some((p) => p.contexts && offersHere(p))
          ? { title: "Cluster ID is shared", message: SHARED_CONTEXT_ID_MESSAGE }
          : undefined,
    tabs: plugins.flatMap((plugin) =>
      plugin.manifest.contributions.detailTabs
        .filter((c) => c.forKinds?.includes(qualified))
        .map((contribution) => ({
          plugin,
          contribution,
          id: `extension:${plugin.manifest.id}/${contribution.id}`,
        })),
    ),
    links: plugins.flatMap((plugin) =>
      plugin.manifest.contributions.detailLinks
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
  const { inventory, tabs, links, lookupProblem } = useExtensionContributions(context, kind, group);
  const [selected, setSelected] = useState("");
  const active = [...tabs, ...links].some((c) => c.id === selected)
    ? selected
    : tabs[0]?.id || "";
  const current = [...tabs, ...links].find((c) => c.id === active);
  const panelTabs =
    current && !tabs.some((c) => c.id === current.id)
      ? [...tabs, current]
      : tabs;
  if (inventory.status === "error")
    return <ErrorNotice message={inventory.error} retry={inventory.reload} />;
  // An uncheckable cluster hides limited apps' views; say why instead of dropping them silently.
  const lookupNotice = lookupProblem !== undefined && (
    lookupProblem.retry ? (
      <ErrorNotice title={lookupProblem.title} message={lookupProblem.message} retry={lookupProblem.retry} />
    ) : (
      <div className="extension-error" role="alert">
        <div>
          <strong>{lookupProblem.title}</strong>
          <p>{lookupProblem.message}</p>
        </div>
      </div>
    )
  );
  if (!tabs.length && !links.length) return lookupNotice || null;
  const ns = contributionKind(kind, group) === "/Namespace" ? name : (namespace ?? "");
  return (
    <section className="extension-installed extension-resource-slot">
      {lookupNotice}
      <div className="extension-toolbar">
        <strong>Apps</strong>
        {links.length > 0 && (
          <details>
            <summary>App links</summary>
            {links.map((c) => (
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
