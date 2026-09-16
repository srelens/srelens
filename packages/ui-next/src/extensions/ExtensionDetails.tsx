import { useContext, useState } from "react";
import {
  CAPABILITY_CATALOG,
  saveTextFile,
  type ExtensionChange,
  type ExtensionPreviousVersion,
  type ExtensionSource,
  type InstalledExtension,
} from "@srelens/core";
import { CodeEditor } from "@srelens/ui-kit";
import { ExtensionControls } from "./ExtensionControls";
import { extensionLabel } from "./inventoryStore";

const facts = new Map(CAPABILITY_CATALOG.map((capability) => [capability.id, capability]));

/** What a granted host capability can do, from the backend registry's own annotations. */
function describeGrant(id: string): string {
  const fact = facts.get(id);
  if (!fact) return "Not provided by this host";
  return [
    fact.readOnly ? "Read-only" : "Changes resources",
    fact.requiresConfirm && "Asks for confirmation",
    fact.sensitive && "Sensitive",
    fact.destructive && "Destructive",
  ]
    .filter(Boolean)
    .join(" · ");
}

const from: Record<ExtensionSource, string> = { catalog: "from the Catalog", local: "local manifest" };

/**
 * Says only what the host verified. The installed version's proof is rechecked on every
 * load, and a failure quarantines the app; a kept version's is checked when it is restored.
 */
function origin(version: InstalledExtension | ExtensionPreviousVersion) {
  const installed = "history" in version;
  const signer = !version.signatureProof
    ? "Unsigned"
    : !installed
      ? "Signed; verified when restored"
      : version.quarantined
        ? "Signature not verified"
        : "Signed by srelens";
  return `${signer} · ${from[version.source]}`;
}

const installedOn = (seconds: number) => new Date(seconds * 1000).toLocaleString();

/** The manifest, grants, source, settings and kept versions of one installed app. */
export function ExtensionDetails({
  plugin,
  busy,
  change,
  onError,
}: {
  plugin: InstalledExtension;
  busy: boolean;
  change(action: ExtensionChange): Promise<boolean>;
  onError(message: string): void;
}) {
  const { Button } = useContext(ExtensionControls);
  const [resetting, setResetting] = useState(false);
  const [rollback, setRollback] = useState<ExtensionPreviousVersion | null>(null);
  const { manifest } = plugin;

  async function exportSettings() {
    try {
      await saveTextFile(`${manifest.id}-settings.json`, `${JSON.stringify(plugin.settings, null, 2)}\n`);
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
    }
  }

  // Rolling back grants the older manifest's permissions again, so it is reviewed like an
  // install whenever those differ from what is granted now.
  const requested = rollback?.manifest.permissions ?? [];
  const added = requested.filter((permission) => !plugin.grants.includes(permission));
  const dropped = plugin.grants.filter((grant) => !requested.includes(grant));
  const changesGrants = added.length > 0 || dropped.length > 0;

  return (
    <section className="extension-details" aria-label={`${extensionLabel(plugin)} details`}>
      <p className="extension-message">
        {origin(plugin)} · version {manifest.version}, revision {plugin.revision} · installed{" "}
        {installedOn(plugin.installedAt)}
      </p>

      <h3>Granted capabilities</h3>
      <ul className="extension-grants" aria-label="Granted capabilities">
        {plugin.grants.map((grant) => (
          <li key={grant}>
            <code>{grant}</code> <span>{describeGrant(grant)}</span>
          </li>
        ))}
      </ul>

      <h3>Manifest</h3>
      {/* A stored manifest can carry a format character this host now refuses (an app
          installed before the rule is quarantined, not rewritten). JSON escapes control
          characters but not those, so they are written as JSON escapes here rather than drawn:
          one `\uXXXX` per UTF-16 unit, so a code point above U+FFFF (a tag character, say)
          becomes its surrogate pair and the text still reads back as the same manifest. */}
      <CodeEditor
        value={JSON.stringify(manifest, null, 2).replace(/\p{Cf}/gu, (c) =>
          c.split("").map((unit) => `\\u${unit.charCodeAt(0).toString(16).padStart(4, "0")}`).join(""),
        )}
        readOnly
        language="none"
        ariaLabel={`${extensionLabel(plugin)} manifest`}
        minHeight={160}
        maxHeight={360}
      />

      <h3>Settings</h3>
      <p className="extension-message">
        Settings are plain JSON and are exported as saved; apps must not keep secrets in them.
      </p>
      <div className="extension-toolbar">
        <Button variant="secondary" disabled={busy} onClick={() => void exportSettings()}>
          Export settings
        </Button>
        <Button variant="secondary" disabled={busy} onClick={() => setResetting(true)}>
          Reset settings
        </Button>
      </div>
      {resetting && (
        <div
          className="extension-install"
          role="alertdialog"
          aria-label="Reset settings"
          onKeyDown={(e) => {
            if (e.key === "Escape" && !busy) setResetting(false);
          }}
        >
          <p>Reset {extensionLabel(plugin)} to its default settings? Its saved settings are removed.</p>
          <Button variant="secondary" autoFocus disabled={busy} onClick={() => setResetting(false)}>
            Cancel
          </Button>
          <Button
            variant="danger"
            disabled={busy}
            onClick={() =>
              void change({ action: "settings", id: manifest.id, settings: {} }).then((done) => {
                if (done) setResetting(false);
              })
            }
          >
            Reset to defaults
          </Button>
        </div>
      )}

      <h3>Previous versions</h3>
      {plugin.history.length === 0 ? (
        <p className="extension-message">No earlier version is kept. Each update keeps up to three.</p>
      ) : (
        <ul className="extension-versions" aria-label="Previous versions">
          {plugin.history.map((version) => (
            <li key={version.revision}>
              <span>
                {version.manifest.version} · {origin(version)} · installed {installedOn(version.installedAt)}
              </span>
              <Button variant="secondary" size="xs" disabled={busy} onClick={() => setRollback(version)}>
                Roll back to {version.manifest.version}
              </Button>
            </li>
          ))}
        </ul>
      )}
      {rollback && (
        <section className="extension-install extension-permission-review" aria-label="Review rollback">
          <p>
            Roll {extensionLabel(plugin)} back to {rollback.manifest.version}?{" "}
            {changesGrants ? (
              <>
                It requests: {requested.join(", ") || "no permissions"}.
                {dropped.length > 0 && ` It no longer uses: ${dropped.join(", ")}.`}
              </>
            ) : (
              "It uses the permissions granted now."
            )}{" "}
            Settings are kept, and the versions after it are discarded.
          </p>
          <Button
            disabled={busy}
            onClick={() =>
              void change({
                action: "rollback",
                id: manifest.id,
                revision: rollback.revision,
                grants: requested,
              }).then((done) => {
                if (done) setRollback(null);
              })
            }
          >
            {changesGrants ? "Roll back and grant permissions" : "Roll back"}
          </Button>
          <Button variant="secondary" onClick={() => setRollback(null)}>
            Cancel
          </Button>
        </section>
      )}
    </section>
  );
}
