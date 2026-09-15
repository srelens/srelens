import { useContext, useState } from "react";
import {
  listContexts,
  loadKubeconfigFiles,
  type ExtensionChange,
  type InstalledExtension,
} from "@srelens/core";
import { ExtensionControls } from "./ExtensionControls";
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
  const { Button } = useContext(ExtensionControls);
  const saved = plugin.contexts;
  const [limited, setLimited] = useState(Boolean(saved));
  const [chosen, setChosen] = useState<string[]>(saved ?? []);
  const listing = useResource(() => listContexts(loadKubeconfigFiles()), [], () => false);
  // A saved name the kubeconfig no longer lists stays visible, so it can be removed.
  const names = [
    ...new Set([...(listing.data?.contexts ?? []).map((context) => context.name), ...(saved ?? [])]),
  ];
  const toggle = (name: string) =>
    setChosen((current) =>
      current.includes(name) ? current.filter((each) => each !== name) : [...current, name],
    );
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
          {listing.status === "loading" && (
            <p role="status" className="extension-message">
              Loading clusters…
            </p>
          )}
          {listing.data?.error && (
            <p className="extension-warning">Could not list every kubeconfig context: {listing.data.error}</p>
          )}
          <ul className="extension-cluster-list">
            {names.map((name) => (
              <li key={name}>
                <label>
                  <input type="checkbox" checked={chosen.includes(name)} disabled={busy} onChange={() => toggle(name)} />{" "}
                  {name}
                </label>
              </li>
            ))}
          </ul>
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
