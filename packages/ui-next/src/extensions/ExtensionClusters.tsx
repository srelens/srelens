import { useContext, useEffect, useRef, useState } from "react";
import {
  KUBECONFIG_FILES_CHANGED,
  listContexts,
  loadKubeconfigFiles,
  type ExtensionChange,
  type InstalledExtension,
} from "@srelens/core";
import { ExtensionControls } from "./ExtensionControls";
import { ErrorNotice } from "./ExtensionResults";
import { useResource } from "../lib/useResource";

/**
 * Which clusters an installed app is offered on. The list keeps each context's stable ID,
 * never its name: a name gains a `file/` prefix as soon as another kubeconfig declares the
 * same one, and a removed context's name can pass to another cluster (#265). On other
 * clusters the app's pages, tabs and actions are hidden and the host refuses its reads and
 * actions. Mount it keyed by the saved list, so a saved change resets the form.
 */
export function ExtensionClusters({
  plugin,
  busy,
  change,
}: {
  plugin: InstalledExtension;
  busy: boolean;
  change(action: ExtensionChange): Promise<boolean>;
}) {
  const { Button, Combobox } = useContext(ExtensionControls);
  const saved = plugin.contexts;
  const [limited, setLimited] = useState(Boolean(saved));
  const [chosen, setChosen] = useState<string[]>(saved ?? []);
  // The kubeconfig files in use: what Connections last published, or else what is stored.
  // A published list wins because storage may have refused the save.
  const files = useRef<string[] | undefined>(undefined);
  const listing = useResource(() => listContexts(files.current ?? loadKubeconfigFiles()), [], () => false);
  // Settings → Apps can stay mounted while Connections adds or removes a kubeconfig; list
  // again then, so the picker offers a new cluster and stops offering a removed one.
  const { reload } = listing;
  useEffect(() => {
    const onFilesChanged = (event: Event) => {
      const detail = (event as CustomEvent<string[]>).detail;
      if (Array.isArray(detail)) files.current = detail;
      reload();
    };
    window.addEventListener(KUBECONFIG_FILES_CHANGED, onFilesChanged);
    return () => window.removeEventListener(KUBECONFIG_FILES_CHANGED, onFilesChanged);
  }, [reload]);
  // `listContexts` reports a failed listing in its result as well as by rejecting.
  const failure = listing.status === "error" ? listing.error : listing.data?.error;
  const names = new Map((listing.data?.contexts ?? []).map((context) => [context.stableId, context.name]));
  // A chosen ID the kubeconfig no longer has shows as the ID itself, so it can still be removed.
  const label = (id: string) => names.get(id) ?? id;
  // A kubeconfig can hold hundreds of contexts, so they are searched rather than listed.
  const available = [...names]
    .filter(([id]) => !chosen.includes(id))
    .map(([id, name]) => ({ value: id, label: name }));
  const unchanged = limited
    ? Boolean(saved) && chosen.length === saved!.length && chosen.every((id) => saved!.includes(id))
    : !saved;
  const group = `${plugin.manifest.id}-clusters`;
  return (
    <fieldset className="extension-clusters">
      <legend>Clusters</legend>
      <label>
        <input type="radio" name={group} checked={!limited} disabled={busy} onChange={() => setLimited(false)} />{" "}
        All clusters
      </label>
      <label>
        <input type="radio" name={group} checked={limited} disabled={busy} onChange={() => setLimited(true)} />{" "}
        Only these clusters
      </label>
      {limited && (
        <>
          {failure ? (
            <ErrorNotice title="Could not list clusters" message={failure} retry={listing.reload} />
          ) : (
            listing.status === "loading" && (
              <p role="status" className="extension-message">
                Loading clusters…
              </p>
            )
          )}
          <Combobox
            value=""
            onValueChange={(id) =>
              id && setChosen((current) => (current.includes(id) ? current : [...current, id]))
            }
            options={available}
            placeholder="Add a cluster…"
            searchPlaceholder="Search clusters…"
            ariaLabel="Add a cluster"
          />
          {chosen.length === 0 ? (
            <p className="extension-message">No cluster chosen yet.</p>
          ) : (
            <ul className="extension-cluster-list" aria-label="Chosen clusters">
              {chosen.map((id) => (
                <li key={id}>
                  <code title={id}>{label(id)}</code>
                  <Button
                    variant="secondary"
                    size="xs"
                    aria-label={`Remove ${label(id)}`}
                    disabled={busy}
                    onClick={() => setChosen((current) => current.filter((each) => each !== id))}
                  >
                    Remove
                  </Button>
                </li>
              ))}
            </ul>
          )}
        </>
      )}
      <p className="extension-message">
        On other clusters the app's pages, tabs and actions are hidden, and the host refuses its reads and actions.
      </p>
      <Button
        variant="secondary"
        disabled={busy || unchanged || (limited && chosen.length === 0)}
        onClick={() =>
          void change({ action: "clusters", id: plugin.manifest.id, contexts: limited ? chosen : null })
        }
      >
        Save clusters
      </Button>
    </fieldset>
  );
}
