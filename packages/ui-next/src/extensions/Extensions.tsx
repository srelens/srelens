import { useContext, useEffect, useState } from "react";
import {
  configureExtensions,
  contributionKind,
  EXTENSIONS_CHANGED,
  isTauri,
  listExtensions,
  type ExtensionChange,
  type ExtensionContribution,
  type InstalledExtension,
} from "@srelens/core";
import { useResource } from "../lib/useResource";

import { ExtensionControls } from "./ExtensionControls";
export { ExtensionControlsProvider } from "./ExtensionControls";
import { ErrorNotice, ExtensionResults } from "./ExtensionResults";
export { ExtensionResults } from "./ExtensionResults";
import { ExtensionWorkspace } from "./ExtensionWorkspace";

export function useExtensions() {
  const [revision, setRevision] = useState(0);
  useEffect(() => {
    const reload = () => setRevision((v) => v + 1);
    window.addEventListener(EXTENSIONS_CHANGED, reload);
    return () => window.removeEventListener(EXTENSIONS_CHANGED, reload);
  }, []);
  return useResource(
    async () =>
      isTauri()
        ? listExtensions()
        : {
            schemaVersion: 1,
            developerMode: false,
            nextRevision: 1,
            plugins: [],
          },
    [revision],
  );
}
export function ExtensionManager({
  contexts = [],
  onOpen,
}: {
  contexts?: Array<{ name: string; label?: string }>;
  onOpen?: (
    plugin: InstalledExtension,
    page: ExtensionContribution,
    context: string,
  ) => void;
}) {
  const { Button, Combobox } = useContext(ExtensionControls);
  const inventory = useExtensions();
  const [archive, setArchive] = useState<{
    name: string;
    content: string;
  } | null>(null);
  const [source, setSource] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [review, setReview] = useState<{
    source: string;
    name: string;
    permissions: string[];
  } | null>(null);
  const [context, setContext] = useState("");
  const [opened, setOpened] = useState<{ id: string; page: string } | null>(
    null,
  );
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
      setArchive(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }
  if (!isTauri())
    return (
      <p className="extension-message">
        Local extensions are available in the desktop app.
      </p>
    );
  if (inventory.status === "loading")
    return (
      <p role="status" className="extension-message">
        Loading extensions…
      </p>
    );
  if (inventory.status === "error")
    return <ErrorNotice message={inventory.error} retry={inventory.reload} />;
  const state = inventory.data!;
  const enabled = state.plugins.filter((p) => p.enabled && state.developerMode);
  const selected = enabled.find((p) => p.manifest.id === opened?.id);
  const selectedPage = selected?.manifest.contributions.pages.find(
    (p) => p.id === opened?.page,
  );
  return (
    <div className="extension-manager">
      <div className="extension-toolbar">
        <strong>Extensions</strong>
        <label>
          <input
            aria-label="Extension developer mode"
            type="checkbox"
            checked={state.developerMode}
            disabled={busy}
            onChange={(e) =>
              void change({
                action: "developerMode",
                enabled: e.target.checked,
              })
            }
          />{" "}
          Developer mode
        </label>
        <Button variant="secondary" onClick={inventory.reload}>
          Refresh
        </Button>
      </div>
      {state.developerMode && (
        <p className="extension-warning" role="status">
          Developer mode: unsigned local extensions are allowed. Only read-only
          custom-resource manifests and the audited Freelens FluxCD 5.3.1
          archive are supported.
        </p>
      )}
      {error && (
        <p role="alert" className="extension-error">
          {error}
        </p>
      )}
      <div className="extension-install">
        <label htmlFor="extension-archive">
          Freelens FluxCD 5.3.1 archive (.tgz)
        </label>
        <input
          id="extension-archive"
          type="file"
          accept=".tgz,.tar.gz"
          disabled={busy || !state.developerMode}
          onChange={async (e) => {
            const file = e.target.files?.[0];
            setArchive(null);
            if (!file) return;
            if (file.size > 1024 * 1024) {
              setError("Extension archive exceeds 1 MiB");
              return;
            }
            try {
              const bytes = new Uint8Array(await file.arrayBuffer());
              let binary = "";
              for (const byte of bytes) binary += String.fromCharCode(byte);
              setArchive({ name: file.name, content: btoa(binary) });
              setError("");
            } catch (error) {
              setError(String(error));
            }
          }}
        />
        {archive && (
          <section aria-label="Review archive permissions">
            <p>
              <strong>{archive.name}</strong> requests read access to Flux
              custom resources, their details, namespaces, and Flux events in
              the cluster you open. Cluster writes are unavailable. The backend
              verifies the original release archive before installing.
            </p>
            <Button
              disabled={busy || !state.developerMode}
              onClick={() =>
                void change({
                  action: "installArchive",
                  archive: archive.content,
                  grants: ["freelens.flux.read"],
                })
              }
            >
              Install archive and grant read access
            </Button>
          </section>
        )}
      </div>
      <div className="extension-install">
        <label htmlFor="extension-manifest">
          Local extension manifest (JSON)
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
          disabled={!source.trim() || !state.developerMode || busy}
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
        {review && (
          <section aria-label="Review extension permissions">
            <p>
              <strong>{review.name}</strong> requests:{" "}
              {review.permissions.join(", ")}. Installing an existing ID
              replaces its manifest and refreshes its open pages.
            </p>
            <Button
              disabled={busy}
              onClick={() =>
                void change({
                  action: "install",
                  manifest: review.source,
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
      </div>
      {state.plugins.length === 0 && (
        <p className="extension-message">No extensions installed.</p>
      )}
      {state.plugins.map((plugin) => (
        <section className="extension-installed" key={plugin.manifest.id}>
          <div className="extension-toolbar">
            <strong>{plugin.manifest.name}</strong>
            <span>{plugin.manifest.version} · Unsigned local</span>
            <label>
              <input
                aria-label={`Enable ${plugin.manifest.name}`}
                type="checkbox"
                checked={plugin.enabled}
                disabled={busy || !state.developerMode}
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
                void change({ action: "remove", id: plugin.manifest.id })
              }
            >
              Remove
            </Button>
          </div>
          <p className="extension-message">{plugin.manifest.id}</p>
        </section>
      ))}
      {settings && (
        <section className="extension-install">
          <label htmlFor="extension-settings">
            Extension settings (JSON object)
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
      {enabled.length > 0 && (
        <section>
          <div className="extension-toolbar">
            <strong>Extension pages</strong>
            <Combobox
              ariaLabel="Extension cluster"
              value={context}
              onValueChange={(value) => {
                setContext(value);
                setOpened(null);
              }}
              options={contexts.map((c) => ({
                value: c.name,
                label: c.label ?? c.name,
              }))}
              placeholder="Choose a cluster"
            />
          </div>
          {enabled.map((plugin) => (
            <div key={plugin.manifest.id}>
              <h3 className="extension-message">{plugin.manifest.name}</h3>
              <div className="extension-toolbar">
                {plugin.manifest.contributions.pages.map((page) => (
                  <Button
                    variant="secondary"
                    key={page.id}
                    disabled={!context}
                    onClick={() =>
                      onOpen
                        ? onOpen(plugin, page, context)
                        : setOpened({ id: plugin.manifest.id, page: page.id })
                    }
                  >
                    {page.group ? `${page.group} · ${page.title}` : page.title}
                  </Button>
                ))}
              </div>
            </div>
          ))}
        </section>
      )}
      {selected && selectedPage && (
        <section>
          <h3 className="extension-message">{selectedPage.title}</h3>
          <ExtensionWorkspace
            key={`${context}/${selected.manifest.id}`}
            plugin={selected}
            page={selectedPage}
            context={context}
            onPage={(page) => setOpened({ id: selected.manifest.id, page })}
          />
        </section>
      )}
    </div>
  );
}
export function useExtensionContributions(kind: string) {
  const inventory = useExtensions();
  const plugins = inventory.data?.developerMode
    ? inventory.data.plugins.filter((p) => p.enabled)
    : [];
  const qualified = contributionKind(kind);
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
  namespace,
  name,
}: {
  context: string;
  kind: string;
  namespace: string | null;
  name: string;
}) {
  const { Button, Tabs } = useContext(ExtensionControls);
  const { inventory, tabs, actions } = useExtensionContributions(kind);
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
  const ns = kind === "Namespace" ? name : (namespace ?? "");
  return (
    <section className="extension-installed extension-resource-slot">
      <div className="extension-toolbar">
        <strong>Extensions</strong>
        {actions.length > 0 && (
          <details>
            <summary>Extension actions</summary>
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
          label="Extension views"
          tabs={panelTabs.map((c) => ({
            id: c.id,
            label: c.contribution.title,
          }))}
          active={active}
          onChange={setSelected}
        />
      )}
      {current && inventory.status !== "loading" && (
        <ExtensionResults
          key={`${context}/${kind}/${namespace}/${name}/${current.id}`}
          plugin={current.plugin}
          capability={current.contribution.capability}
          context={context}
          namespace={ns}
        />
      )}
    </section>
  );
}

/** Kept visible outside Settings while unsigned local extensions are enabled. */
export function ExtensionWarning() {
  const inventory = useExtensions();
  if (
    !inventory.data?.developerMode ||
    !inventory.data.plugins.some((p) => p.enabled)
  )
    return null;
  return (
    <div className="extension-warning extension-banner" role="status">
      Developer mode · Unsigned local extensions enabled
    </div>
  );
}
