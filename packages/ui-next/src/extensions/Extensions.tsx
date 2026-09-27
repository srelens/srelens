import { ExtensionDetails } from "./ExtensionDetails";
import { bytes, inactiveReason } from "./detailsText";
import { ExtensionSettingsForm } from "./ExtensionSettingsForm";
import { ExtensionBindings, ReviewManifest } from "./ExtensionBindings";
import { plainText } from "./displayText";
import { ExtensionRequirements } from "./ExtensionRequirements";
import { refreshContextIds, useContextLookup } from "./contextIds";
import { ExtensionLogo } from "./ExtensionLogo";
import { useContext, useRef, useState } from "react";
import {
  MAX_EXTENSION_PACKAGE_BYTES,
  clearExtensionSecret,
  configureExtensions,
  contributionKind,
  encodePackage,
  setExtensionSecret,
  extensionEnabledFor,
  isTauri,
  permissionName,
  reviewExtensionPackage,
  validateExtension,
  type ExtensionChange,
  type ExtensionPackageReview,
  type ExtensionValidationError,
  type ExtensionPermissionDiff,
  type InstalledExtension,
} from "@srelens/core";

import { ExtensionCatalog } from "./ExtensionCatalog";
import { ServerPolicy } from "./ServerPolicy";
import { ExtensionControls } from "./ExtensionControls";
export { ExtensionControlsProvider } from "./ExtensionControls";
import { ErrorNotice, ExtensionResults } from "./ExtensionResults";
export { ErrorNotice, ExtensionResults } from "./ExtensionResults";


import { extensionLabel as label, useExtensions } from "./inventoryStore";
export { useExtensions } from "./inventoryStore";
export { AMBIGUOUS_CONTEXT_MESSAGE, SHARED_CONTEXT_ID_MESSAGE, refreshContextIds, useContextId, useContextLookup } from "./contextIds";

/** What a review's problems ask of the reader: the manifest's own to fix, and the
    server's policy refusal (#578), which is not the manifest's. */
function problemsHeading(errors: ExtensionValidationError[]) {
  const policy = errors.some((problem) => problem.code === "EXTENSION_POLICY_REFUSED");
  const own = errors.filter((problem) => problem.code !== "EXTENSION_POLICY_REFUSED").length;
  if (own === 0) return "This server's policy does not allow installing this app:";
  if (policy)
    return `This server's policy does not allow installing this app, and the manifest has ${own === 1 ? "a problem" : `${own} problems`} to fix:`;
  return `Fix ${own === 1 ? "this problem" : `these ${own} problems`} in the manifest before installing:`;
}
/**
 * Where a reviewed manifest came from, which decides how it is installed: as its own text,
 * as the package file chosen here (sent again as base64, and verified again), or as a
 * catalog release's package, which the host downloads again (#562).
 */
type ReviewOrigin =
  | { kind: "manifest" }
  | { kind: "packageFile"; package: ExtensionPackageReview; file: string }
  | { kind: "catalogPackage"; package: ExtensionPackageReview; id: string; sha256: string };
const MANIFEST_ORIGIN: ReviewOrigin = { kind: "manifest" };

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
    /** How the reviewed manifest is installed; a package also shows what else it holds. */
    origin: ReviewOrigin;
    name: string;
    permissions: string[];
    /** The parsed manifest, whose bindings the review summarizes; undefined when it is not JSON. */
    manifest?: unknown;
    /** The manifest as the review shows it in full. */
    text: string;
    /** Numbers reviews, so a new one opens with its manifest collapsed. */
    id: number;
    /** Undefined while the host is still checking the manifest. */
    errors?: ExtensionValidationError[];
    permissionDiff?: ExtensionPermissionDiff;
    /** Why the check itself failed, as opposed to the problems it found. */
    checkError?: string;
    /**
     * This review's identity. The same bytes can be reviewed signed and unsigned, so a
     * check that answers late is applied only to the review that asked for it.
     */
    request: object;
  } | null>(null);
  const reviews = useRef(0);
  /**
   * Counts the reviews started from every entry point: pasted text, a package file, the
   * catalog. One whose manifest loads slowly (a package read and verified, a catalog
   * download) is dropped if another was started after it, rather than replacing it.
   */
  const latestReview = useRef(0);
  /** Starts a review, and answers whether it is still the one most recently started. */
  function beginReview() {
    const ticket = ++latestReview.current;
    return () => ticket === latestReview.current;
  }
  /** The package file input, hidden and opened by a kit button: its own label cannot be styled. */
  const packageInput = useRef<HTMLInputElement>(null);
  /** The ID of the app whose settings form is open. */
  const [settingsFor, setSettingsFor] = useState<string | null>(null);
  /** Saves through the host, which checks every value; rejects with its reason (#542). */
  async function saveSettings(id: string, settings: Record<string, unknown>) {
    await configureExtensions({ action: "settings", id, settings });
    inventory.reload();
  }
  /** Keeps a secret in the host's store (#543); the host answers only whether it is set. */
  async function setSecret(id: string, setting: string, secret: string) {
    await setExtensionSecret(id, setting, secret);
    inventory.reload();
  }
  async function clearSecret(id: string, setting: string) {
    await clearExtensionSecret(id, setting);
    inventory.reload();
  }
  async function change(action: ExtensionChange) {
    setBusy(true);
    setError("");
    try {
      await configureExtensions(action);
      inventory.reload();
      setReview(null);
      if (action.action === "install" || action.action === "installPackage" || action.action === "installCatalogPackage") setTab("installed");
      return true;
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      return false;
    } finally {
      setBusy(false);
    }
  }
  /** Opens the permission review, and offers to install only once the host finds no problems. */
  async function reviewManifest(manifest: string, signature?: number[], origin: ReviewOrigin = MANIFEST_ORIGIN) {
    let parsed: { name?: unknown; permissions?: unknown } = {};
    let value: unknown;
    // Shown indented, as Details shows an installed app: the same values the host checks,
    // readable whether the manifest came from the Catalog or was pasted on one line.
    let text = manifest;
    try {
      value = JSON.parse(manifest);
      text = JSON.stringify(value, null, 2);
      if (value && typeof value === "object") parsed = value as typeof parsed;
    } catch {
      // The host reports invalid JSON with a code and path, like any other problem.
    }
    // What an install grants: each entry's capability, `network.http` included (#568),
    // whose hosts the review shows and the host diffs.
    const names = Array.isArray(parsed.permissions) ? parsed.permissions.map(permissionName) : [];
    const permissions = names.every((name) => name !== undefined) ? (names as string[]) : [];
    const name = typeof parsed.name === "string" ? parsed.name : "This manifest";
    beginReview();
    const request = {};
    setError("");
    setReview({
      request,
      id: ++reviews.current,
      source: manifest,
      signature,
      origin,
      name,
      permissions,
      manifest: value,
      text,
    });
    try {
      // A package's signature is over its digest list, which names the manifest: the host
      // checks all three together, exactly as installing the package will.
      const { errors, permissionDiff } = await (origin.kind === "manifest"
        ? validateExtension(manifest, permissions, signature)
        : validateExtension(manifest, permissions, signature, origin.package.digests));
      setReview((current) => (current?.request === request ? { ...current, errors, permissionDiff } : current));
    } catch (e) {
      // The check did not run, which says nothing about the manifest: keep the review
      // open with the reason and a retry, and do not offer to install.
      const checkError = e instanceof Error ? e.message : String(e);
      setReview((current) => (current?.request === request ? { ...current, checkError } : current));
    }
  }
  /** Reads a package file chosen here, has the host verify it, and opens its review. */
  async function reviewPackageFile(file: File | undefined) {
    if (!file) return;
    const current = beginReview();
    setError("");
    setReview(null);
    if (file.size > MAX_EXTENSION_PACKAGE_BYTES) {
      setError(`${file.name} is ${bytes(file.size)}; a package may be at most ${bytes(MAX_EXTENSION_PACKAGE_BYTES)}.`);
      return;
    }
    setBusy(true);
    try {
      const content = new Uint8Array(await file.arrayBuffer());
      const verified = await reviewExtensionPackage(content);
      if (!current()) return;
      if (!verified.package) throw new Error("the host did not return what the package holds");
      void reviewManifest(verified.manifest, verified.signature ?? undefined, {
        kind: "packageFile", package: verified.package, file: encodePackage(content),
      });
    } catch (e) {
      if (current()) setError(`Could not review ${file.name}: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setBusy(false);
    }
  }
  /** The install that grants what was reviewed, for where the manifest came from. */
  function installation(current: NonNullable<typeof review>): ExtensionChange {
    const reviewed = current.permissionDiff?.previousRevision == null ? {} : { reviewedRevision: current.permissionDiff.previousRevision };
    const origin = current.origin;
    switch (origin.kind) {
      case "manifest":
        return { action: "install", manifest: current.source, ...(current.signature ? { signature: current.signature } : {}), grants: current.permissions, ...reviewed };
      case "packageFile":
        return { action: "installPackage", package: origin.file, grants: current.permissions, ...reviewed };
      case "catalogPackage":
        return { action: "installCatalogPackage", id: origin.id, sha256: origin.sha256, packageSha256: origin.package.sha256, grants: current.permissions, ...reviewed };
    }
  }
  if (inventory.status === "loading")
    return (
      <p role="status" className="extension-message">
        Loading apps…
      </p>
    );
  if (inventory.status === "error")
    return <ErrorNotice message={inventory.error} retry={inventory.reload} />;
  const state = inventory.data!;
  /** Whether the administrator's policy makes every user keep this app (#578). */
  const required = (plugin: InstalledExtension) => state.policy?.requiredApps.includes(plugin.manifest.id) ?? false;
  const signedOnly = state.policy?.allowUnsignedApps === false;
  return (
    <div className="extension-manager">
      <div className="extension-toolbar">
        <strong>Apps</strong>
        <Button variant="secondary" onClick={inventory.reload}>
          Refresh
        </Button>
      </div>
      {!isTauri() && (
        <p className="extension-message">
          The apps you install here are yours: everyone who signs in to this server has their own.
        </p>
      )}
      {state.policy && (
        <ServerPolicy policy={state.policy} plugins={state.plugins}
          openCatalog={() => { setTab("catalog"); setCatalogOpened(true); }} />
      )}
      {inventory.updates?.mode === "polling" && (
        <p role="status" className="extension-message">
          Live updates to this list are unavailable ({inventory.updates.reason}); a change made elsewhere shows within five seconds.
        </p>
      )}
      <div className="extension-install">
        <label>
          <input type="checkbox" checked={state.allowUnsignedApps ?? false} disabled={busy || signedOnly}
            onChange={(event) => void change({ action: "unsignedApps", allowUnsignedApps: event.target.checked })} />{" "}
          Allow unsigned apps to modify clusters and run code
        </label>
        {signedOnly && <p className="extension-message">This server's policy allows only signed apps.</p>}
        <p className="extension-message">Off by default. Read-only declarative apps need only their permission grants. Turning this off disables affected apps and keeps their settings. Turning it on does not re-enable them. Executable apps are not supported by this host.</p>
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
              {/* The name and the permission IDs are the manifest's own text, and a
                  manifest the host has not accepted may hold text that displays as
                  another app's or reorders this sentence. Neither is drawn until the
                  check comes back with no problems; nothing can be installed before
                  then either. */}
              {review.errors?.length === 0 ? (
                <>
                  {/* The logo is drawn with the name, once the host has accepted both; it
                      is decoration, and the label beside it says who signed the app. */}
                  {review.origin.kind !== "manifest" && <ExtensionLogo icon={review.origin.package.icon} name={review.name} size={24} />}{" "}
                  <strong>{plainText(review.name)}</strong> ({review.signature ? "Signature verified · srelens" : review.origin.kind === "manifest" ? "Unsigned manifest" : "Unsigned package"}) {!review.permissionDiff ? "could not have its access changes compared" : review.permissionDiff.previousRevision == null ? "requests a new installation" : "updates the installed app"}.
                </>
              ) : (
                <>
                  <strong>This {review.origin.kind === "manifest" ? "manifest" : "package"}</strong> ({review.signature ? "Signature verified · srelens" : review.origin.kind === "manifest" ? "Unsigned manifest" : "Unsigned package"}) has
                  not passed the host's checks, so its name and the permissions it requests are not shown.
                </>
              )}
            </p>
            {review.origin.kind !== "manifest" && (
              <details className="extension-package-files">
                <summary>
                  Package: {review.origin.package.files.length} file{review.origin.package.files.length === 1 ? "" : "s"}, {bytes(review.origin.package.files.reduce((total, file) => total + file.size, 0))}, each checked against its digest
                </summary>
                <ul aria-label="Package files">
                  {review.origin.package.files.map((file) => (
                    <li key={file.path}><code>{file.path}</code> <span className="extension-package-size">{bytes(file.size)}</span></li>
                  ))}
                </ul>
              </details>
            )}
            {review.errors?.length === 0 && review.permissionDiff && (
              <div aria-label="Access changes" className="extension-access-diff">
                <strong>{review.permissionDiff.previousRevision == null ? "Requested access" : "Access changes"}</strong>
                <ul aria-label="Added access">
                  {review.permissionDiff.added.map((entry) => <li key={entry}>Added: {plainText(entry)}</li>)}
                </ul>
                {review.permissionDiff.removed.length > 0 && <ul aria-label="Removed access">
                  {review.permissionDiff.removed.map((entry) => <li key={entry}>Removed: {plainText(entry)}</li>)}
                </ul>}
                {review.permissionDiff.unchanged.length > 0 && <details>
                  <summary>{review.permissionDiff.unchanged.length} unchanged access item{review.permissionDiff.unchanged.length === 1 ? "" : "s"}</summary>
                  <ul>{review.permissionDiff.unchanged.map((entry) => <li key={entry}>{plainText(entry)}</li>)}</ul>
                </details>}
              </div>
            )}
            {/* The incoming bindings follow the change summary, so an update's
                new and removed access is visible before the full permission list. */}
            {review.errors?.length === 0 && review.permissionDiff && (
              review.permissionDiff.previousRevision == null ? (
                <ExtensionBindings manifest={review.manifest} permissions={review.permissions} />
              ) : (
                <details>
                  <summary>Complete incoming bindings</summary>
                  <ExtensionBindings manifest={review.manifest} permissions={review.permissions} />
                </details>
              )
            )}
            <ReviewManifest key={review.id} text={review.text} />
            {review.checkError ? (
              <ErrorNotice
                title="Could not check the manifest"
                message={review.checkError}
                retry={() => void reviewManifest(review.source, review.signature, review.origin)}
              />
            ) : !review.errors ? (
              <p role="status" className="extension-message">Checking the manifest…</p>
            ) : review.errors.length > 0 ? (
              <div className="extension-problems">
                <p>
                  {/* A policy refusal is the server administrator's rule (#578): nothing
                      in the manifest would fix it, so it is never counted as the manifest's. */}
                  {problemsHeading(review.errors)}
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
            ) : !review.permissionDiff ? (
              <ErrorNotice title="Could not review access changes" message="The host did not return an access comparison. Review this manifest again." retry={() => void reviewManifest(review.source, review.signature, review.origin)} />
            ) : (
              <Button
                disabled={busy}
                onClick={() => void change(installation(review))}
              >
                {review.permissionDiff.previousRevision == null ? "Install and grant permissions" : "Update and grant permissions"}
              </Button>
            )}
            <Button variant="secondary" onClick={() => setReview(null)}>
              Cancel
            </Button>
          </section>
        )}
      <div hidden={tab !== "catalog"}>
        {catalogOpened && (
      <ExtensionCatalog autoLoad installed={state.plugins} onReviewStart={beginReview} onReview={(result, release) => void reviewManifest(
        result.manifest,
        result.signature ?? undefined,
        result.package ? { kind: "catalogPackage", package: result.package, ...release } : MANIFEST_ORIGIN,
      )} />
        )}
      </div>
      <div hidden={tab !== "installed"}>
      <details className="extension-local-tools">
        <summary>{isTauri() ? "Install a local manifest or package" : "Install a local manifest"}</summary>
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
      {/* A package's files live in a directory of the app's own on this computer (#562);
          the web host keeps none, so it installs single-file manifests only. */}
      {isTauri() && (
        <div className="extension-install">
          <label htmlFor="extension-package">Local app package (.srelens-extension)</label>
          <Button variant="secondary" disabled={busy} onClick={() => packageInput.current?.click()}>
            Choose a package file…
          </Button>
          <input
            ref={packageInput}
            id="extension-package"
            type="file"
            accept=".srelens-extension"
            hidden
            disabled={busy}
            onChange={(event) => {
              const file = event.target.files?.[0];
              // Cleared, so choosing the same file again reviews it again.
              event.target.value = "";
              void reviewPackageFile(file);
            }}
          />
          <p className="extension-message">Verified as a whole before review: every file against the package's digest list, and the list against its signature when it has one.</p>
        </div>
      )}
      </details>

      <p className="extension-message extension-catalog-meta">Apps are installed app-wide and are available on every cluster unless an app's Details limit it to chosen clusters. Each page checks the APIs it needs when opened.</p>
      {state.plugins.length === 0 && (
        <p className="extension-message">No apps installed.</p>
      )}
      {state.plugins.map((plugin) => (
        <section className="extension-installed" key={plugin.manifest.id}>
          <div className="extension-toolbar">
            <ExtensionLogo icon={plugin.icon} name={label(plugin)} size={24} />
            <strong>{label(plugin)}</strong>
            <span>{plugin.manifest.version} · {!plugin.signatureProof ? (plugin.source === "catalog" ? "Unsigned · Catalog" : "Unsigned local") : plugin.quarantined ? "Signature not verified" : "Signed by srelens"}</span>
            {required(plugin) && <span>Required by this server</span>}
            <label>
              <input
                aria-label={`Enable ${label(plugin)}`}
                type="checkbox"
                checked={plugin.enabled}
                disabled={busy || Boolean(plugin.quarantined) || Boolean(plugin.policyBlocked) || (required(plugin) && plugin.enabled)}
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
              aria-label={`Details for ${label(plugin)}`}
              aria-expanded={details === plugin.manifest.id}
              onClick={() => setDetails(details === plugin.manifest.id ? null : plugin.manifest.id)}
            >
              Details
            </Button>
            <Button
              variant="secondary"
              disabled={busy}
              aria-label={`Settings for ${label(plugin)}`}
              aria-expanded={settingsFor === plugin.manifest.id}
              onClick={() => setSettingsFor(settingsFor === plugin.manifest.id ? null : plugin.manifest.id)}
            >
              Settings
            </Button>
            <Button
              variant="danger"
              disabled={busy || required(plugin)}
              onClick={() =>
                setRemoving(plugin)
              }
            >
              Remove
            </Button>
          </div>
          <p className="extension-message">{plugin.manifest.id}</p>
          {(plugin.quarantined || plugin.policyBlocked) && (
            <p className="extension-error">Disabled: {inactiveReason(plugin)}</p>
          )}
          {!plugin.quarantined && !plugin.signatureProof && (plugin.manifest.actions?.length ?? 0) > 0 && (
            <p className="extension-message">Requires permission to run unsigned apps that modify clusters.</p>
          )}
          {settingsFor === plugin.manifest.id && (
            <ExtensionSettingsForm
              // Keyed by what it draws, not by revision: a save rolls the revision
              // too, and remounting then would drop "Settings saved." An update
              // that declares other settings starts again from what it kept.
              key={JSON.stringify(plugin.manifest.settings ?? [])}
              plugin={plugin}
              secretStore={state.secretStore}
              onSave={(settings) => saveSettings(plugin.manifest.id, settings)}
              onSetSecret={(setting, secret) => setSecret(plugin.manifest.id, setting, secret)}
              onClearSecret={(setting) => clearSecret(plugin.manifest.id, setting)}
              onClose={() => setSettingsFor(null)}
            />
          )}
          {details === plugin.manifest.id && (
            <ExtensionDetails plugin={plugin} busy={busy} change={change} onError={setError} />
          )}
        </section>
      ))}
      {removing && (
        <section className="extension-install" role="alertdialog" aria-label="Remove app" onKeyDown={e=>{if(e.key==="Escape" && !busy)setRemoving(null);}}>
          <strong>Remove {label(removing)}?</strong>
          <p>
            This removes the app and its saved settings
            {(removing.manifest.settings ?? []).some((setting) => setting.type === "secret-reference")
              ? ", and deletes its secrets from srelens's secrets vault."
              : "."}
          </p>
          <Button variant="secondary" autoFocus disabled={busy} onClick={()=>setRemoving(null)}>Cancel</Button>
          <Button variant="danger" disabled={busy} onClick={()=>{void change({action:"remove",id:removing.manifest.id}).then(removed=>{if(removed)setRemoving(null);});}}>Remove app</Button>
        </section>
      )}
      </div>
    </div>
  );
}
/** The detail tabs and detail links apps offer for a kind, on a cluster they are enabled for. */
export function useExtensionContributions(context: string, kind: string, group?: string) {
  const inventory = useExtensions();
  // App scope keys on the context's key, not its name (#265) or stable ID (#623).
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
     * not be listed (with a retry).
     */
    lookupProblem:
      lookup.status === "failed" && enabled.some((p) => p.contexts && offersHere(p))
        ? { title: "Could not list clusters", message: lookup.error, retry: () => void refreshContextIds() }
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
