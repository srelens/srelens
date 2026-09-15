import { useContext, useState } from "react";
import {
  listContexts,
  loadKubeconfigFiles,
  type ExtensionChange,
  type InstalledExtension,
} from "@srelens/core";
import { ExtensionControls } from "./ExtensionControls";
import { ErrorNotice } from "./ExtensionResults";
import { useResource } from "../lib/useResource";

/**
 * Which clusters an installed app is offered on, by kubeconfig context name. On the others
 * its pages, tabs and actions are hidden and the host refuses its reads and actions.
 * Mount it keyed by the saved list, so a saved change resets the form.
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
  const listing = useResource(() => listContexts(loadKubeconfigFiles()), [], () => false);
  // `listContexts` reports a failed listing in its result as well as by rejecting.
  const failure = listing.status === "error" ? listing.error : listing.data?.error;
  // A kubeconfig can hold hundreds of contexts, so they are searched rather than listed;
  // only the chosen ones are shown, including any the kubeconfig no longer has.
  const available = (listing.data?.contexts ?? [])
    .map((context) => context.name)
    .filter((name) => !chosen.includes(name));
  const unchanged = limited
    ? Boolean(saved) && chosen.length === saved!.length && chosen.every((name) => saved!.includes(name))
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
            onValueChange={(name) =>
              name && setChosen((current) => (current.includes(name) ? current : [...current, name]))
            }
            options={available.map((name) => ({ value: name, label: name }))}
            placeholder="Add a cluster…"
            searchPlaceholder="Search clusters…"
            ariaLabel="Add a cluster"
          />
          {chosen.length === 0 ? (
            <p className="extension-message">No cluster chosen yet.</p>
          ) : (
            <ul className="extension-cluster-list" aria-label="Chosen clusters">
              {chosen.map((name) => (
                <li key={name}>
                  <code>{name}</code>
                  <Button
                    variant="secondary"
                    size="xs"
                    aria-label={`Remove ${name}`}
                    disabled={busy}
                    onClick={() => setChosen((current) => current.filter((each) => each !== name))}
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
