import { networkHosts, type ExtensionChange, type InstalledExtension } from "@srelens/core";
import { HostText } from "./networkText";

/**
 * Where an installed app's `network.http` requests may go (#568), and the one switch a
 * person holds over them: plain HTTP to this computer, for a service with no HTTPS such
 * as Prometheus behind `kubectl port-forward`. Every other request is HTTPS, to these
 * hosts only, and the host checks each one and every redirect.
 */
export function ExtensionNetwork({
  plugin,
  busy,
  change,
}: {
  plugin: InstalledExtension;
  busy: boolean;
  change(action: ExtensionChange): Promise<boolean>;
}) {
  const hosts = networkHosts(plugin.manifest);
  if (hosts.length === 0) return null;
  const allowed = plugin.allowLoopbackHttp === true;
  return (
    <fieldset className="extension-network">
      <legend>Network</legend>
      <p className="extension-message">Requests go only to these hosts, over HTTPS:</p>
      <ul className="extension-network-hosts" aria-label="Hosts this app may reach">
        {hosts.map((host, index) => (
          <li key={index}>
            <HostText manifest={plugin.manifest} host={host} />
          </li>
        ))}
      </ul>
      <label>
        <input
          type="checkbox"
          checked={allowed}
          disabled={busy}
          onChange={() => void change({ action: "loopbackHttp", id: plugin.manifest.id, allowLoopbackHttp: !allowed })}
        />{" "}
        Allow plain HTTP to this computer (loopback)
      </label>
      <p className="extension-message">
        Only for one of the hosts above on this computer, such as a port-forwarded Prometheus. Requests anywhere else
        stay HTTPS.
      </p>
    </fieldset>
  );
}
