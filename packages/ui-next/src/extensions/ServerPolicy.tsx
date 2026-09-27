import { useContext } from "react";
import type { ExtensionPolicy, InstalledExtension } from "@srelens/core";
import { ExtensionControls } from "./ExtensionControls";
import { plainText } from "./displayText";

const list = (items: string[]) => items.map(plainText).join(", ");

/** Each thing the policy limits, in words; what it leaves open is not listed. */
function rules(policy: ExtensionPolicy): string[] {
  const said: string[] = [];
  if (policy.allowedApps)
    said.push(policy.allowedApps.length ? `Only these apps: ${list(policy.allowedApps)}` : "No app is allowed");
  if (policy.blockedApps.length) said.push(`Blocked apps: ${list(policy.blockedApps)}`);
  if (policy.allowedPublishers)
    said.push(policy.allowedPublishers.length ? `Signed apps from: ${list(policy.allowedPublishers)}` : "No signed app is allowed");
  if (!policy.allowUnsignedApps) said.push("Unsigned apps are not allowed");
  if (policy.allowedCapabilities)
    said.push(policy.allowedCapabilities.length
      ? `Capabilities apps may use: ${list(policy.allowedCapabilities)}`
      : "Apps may use no host capability");
  if (!policy.allowWriteActions) said.push("Apps may not write to clusters or run commands in them");
  said.push(policy.networkCeiling.length
    ? `network.http may reach: ${list(policy.networkCeiling)}`
    : "network.http may reach no host");
  return said;
}

/**
 * What the administrator's policy allows on this host (#578), as the host reports it
 * with the user's apps, and the required apps the user has not installed. Read-only:
 * the host enforces the policy on every call, and this only says what it is.
 */
export function ServerPolicy({ policy, plugins, openCatalog }: {
  policy: ExtensionPolicy;
  plugins: InstalledExtension[];
  openCatalog: () => void;
}) {
  const { Button } = useContext(ExtensionControls);
  const missing = policy.requiredApps.filter((id) => !plugins.some((plugin) => plugin.manifest.id === id));
  return (
    <section className="extension-install extension-server-policy" aria-label="Server policy">
      <details>
        <summary>What this server's app policy allows</summary>
        <ul>
          {rules(policy).map((rule) => <li key={rule}>{rule}.</li>)}
        </ul>
        <p className="extension-message">srelens applies this policy on every call, so an app it no longer allows stops working at once.</p>
      </details>
      {missing.length > 0 && (
        <p className="extension-message">
          This server requires {missing.length === 1 ? "an app" : "apps"} you have not installed: {list(missing)}.{" "}
          <Button variant="secondary" onClick={openCatalog}>Open Catalog</Button>
        </p>
      )}
    </section>
  );
}
