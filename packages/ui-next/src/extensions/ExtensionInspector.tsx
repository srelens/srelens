import { useContext, useEffect, useId, useState, type ReactNode } from "react";
import {
  inspectExtension,
  type ExtensionChange,
  type ExtensionInspection,
  type ExtensionOpenStream,
  type ExtensionProcess,
  type InstalledExtension,
} from "@srelens/core";
import { ExtensionControls } from "./ExtensionControls";
import { ErrorNotice } from "./ExtensionResults";
import { LogLineRow, clock } from "./ExtensionLogs";
import { bytes, describeGrant, inactiveReason, installedOn, origin } from "./detailsText";
import { plainText } from "./displayText";

const POLL_MS = 5000;

const count = (value: number) => value.toLocaleString("en-US");
const plural = (value: number, one: string, many = `${one}s`) => `${count(value)} ${value === 1 ? one : many}`;
/** Memory in MiB, without a trailing ".0". */
const mib = (value: number) => `${Number((value / (1024 * 1024)).toFixed(1)).toLocaleString("en-US")} MiB`;
const ms = (value: number | null) => (value === null ? "—" : `${value.toLocaleString("en-US", { maximumFractionDigits: 1 })} ms`);
const delay = (value: number) =>
  value < 1000 ? `${count(value)} ms` : `${(value / 1000).toLocaleString("en-US", { maximumFractionDigits: 1 })} s`;
const since = (at: number) => `${new Date(at).toLocaleDateString()} ${clock(at, false)}`;

/** What a manifest contributes, per kind, in the order the manifest reference lists them. */
function contributions(plugin: InstalledExtension): string[] {
  const { contributions: c, actions } = plugin.manifest;
  const kinds: Array<[number, string]> = [
    [c.pages.length, "page"],
    [c.detailTabs.length, "detail tab"],
    [c.detailLinks.length, "detail link"],
    [c.tableColumns?.length ?? 0, "table column"],
    [c.dashboardCards?.length ?? 0, "dashboard card"],
    [c.detailPanels?.length ?? 0, "detail panel"],
    [c.statusResolvers?.length ?? 0, "status resolver"],
    [c.badges?.length ?? 0, "badge"],
    [c.resourceLinks?.length ?? 0, "resource link"],
    [c.commands?.length ?? 0, "palette command"],
    [actions?.length ?? 0, "action"],
  ];
  return kinds.filter(([n]) => n > 0).map(([n, kind]) => plural(n, kind));
}

/** Where the contributions are live, or why they are not. */
function activity(plugin: InstalledExtension): string {
  const reason = inactiveReason(plugin);
  if (reason) return `Inactive: ${reason}`;
  if (!plugin.contexts) return "Active on every cluster.";
  return plugin.contexts.length === 1
    ? "Active on the cluster chosen in Overview."
    : `Active on the ${count(plugin.contexts.length)} clusters chosen in Overview.`;
}

const STATE: Record<ExtensionProcess["state"], string> = {
  starting: "Starting",
  running: "Running",
  restarting: "Restarting",
  disabled: "Disabled",
  refused: "Refused",
  stopping: "Stopping",
  stopped: "Stopped",
};
const ENFORCEMENT: Record<ExtensionProcess["memory"]["enforcement"], string> = {
  kernel: "Enforced by the kernel",
  host: "Enforced by srelens, by sampling",
  missing: "Not enforced",
};

/** One titled part of the Inspector, named by its heading for assistive technology. */
function Part({ title, children }: { title: string; children: ReactNode }) {
  const heading = useId();
  return (
    <section className="extension-inspector-part" aria-labelledby={heading}>
      <h3 id={heading}>{title}</h3>
      {children}
    </section>
  );
}

function Facts({ rows }: { rows: Array<[string, ReactNode]> }) {
  return (
    <dl className="extension-facts">
      {rows.map(([term, value]) => (
        <div key={term}>
          <dt>{term}</dt>
          <dd>{value}</dd>
        </div>
      ))}
    </dl>
  );
}

function Streams({ label, streams, empty }: { label: string; streams: ExtensionOpenStream[]; empty: string }) {
  return (
    <>
      <h4>{label}</h4>
      {streams.length === 0 ? (
        <p className="extension-message">{empty}</p>
      ) : (
        <ul className="extension-inspector-streams" aria-label={label}>
          {streams.map((stream) => (
            <li key={stream.stream}>
              <code title={plainText(stream.stream)}>{plainText(stream.view)}</code>{" "}
              <span>
                {plainText(stream.source)} · revision {stream.revision} · {plural(stream.messages, "message")} ·{" "}
                {bytes(stream.bytes)}
              </span>
            </li>
          ))}
        </ul>
      )}
    </>
  );
}

function Process({
  inspection,
  plugin,
  busy,
  change,
  onViewLogs,
}: {
  inspection: ExtensionInspection;
  plugin: InstalledExtension;
  busy: boolean;
  change(action: ExtensionChange): Promise<boolean>;
  onViewLogs(): void;
}) {
  const { Button } = useContext(ExtensionControls);
  const process = inspection.process;
  if (inspection.runtime === "declarative")
    return (
      <p className="extension-message">
        No process. This is a declarative app: srelens runs nothing of its own for it, so there is no process, memory or
        request traffic to show.
      </p>
    );
  if (!process) return <p className="extension-message">No process is running for this app.</p>;
  const { memory, rpc, restart } = process;
  const rows: Array<[string, ReactNode]> = [["State", <span data-state={process.state}>{STATE[process.state]}</span>]];
  // With a headline, the reason is said once, in the block beside its actions.
  if (process.reason && !process.message) rows.push(["Reason", plainText(process.reason)]);
  if (restart) rows.push(["Restart", `Attempt ${count(restart.attempt)} of ${count(restart.of)} in ${delay(restart.delayMs)}`]);
  rows.push(
    ["Sidecar API version", process.apiVersion === null ? "Not negotiated" : plainText(process.apiVersion)],
    ["PID", process.pid === null ? "None" : String(process.pid)],
    ["Running since", process.startedAt === null ? "Not running" : since(process.startedAt)],
    ["Launches", count(process.launches)],
    ["Unexpected exits", count(process.unexpectedExits)],
    [
      "Memory",
      memory.bytes === null
        ? `Not measured on this OS; the limit is ${mib(memory.limitBytes)}`
        : `${mib(memory.bytes)} of ${mib(memory.limitBytes)}`,
    ],
    ["Memory limit", ENFORCEMENT[memory.enforcement]],
    ["CPUs", count(process.cpus)],
    [
      "Requests",
      `${count(rpc.answered)} answered · ${count(rpc.failed)} failed · ${count(rpc.timedOut)} timed out · ${count(rpc.refused)} refused · ${count(rpc.inFlight)} in flight`,
    ],
    [
      "Latency",
      rpc.latency.samples === 0
        ? "No requests answered yet"
        : `p50 ${ms(rpc.latency.p50Ms)} · p95 ${ms(rpc.latency.p95Ms)} · max ${ms(rpc.latency.maxMs)}, over ${plural(rpc.latency.samples, "request")}`,
    ],
    ["Sidecar streams", `${count(process.streams.open)} open of ${count(process.streams.limit)} · ${count(process.streams.opened)} opened`],
  );
  return (
    <>
      {process.message && (
        <div className="extension-process-alert" role="alert">
          <strong>{plainText(process.message)}</strong>
          {process.reason && <p>{plainText(process.reason)}</p>}
          <div className="extension-toolbar">
            {/* The supervisor offers "restart" too, but no host capability restarts a sidecar yet (#574). */}
            {process.actions.includes("viewLogs") && (
              <Button variant="secondary" onClick={onViewLogs}>
                View logs
              </Button>
            )}
            {process.actions.includes("disable") && (
              <Button
                variant="danger"
                disabled={busy || !plugin.enabled}
                onClick={() => void change({ action: "enable", id: plugin.manifest.id, enabled: false })}
              >
                Disable
              </Button>
            )}
          </div>
        </div>
      )}
      <Facts rows={rows} />
    </>
  );
}

/**
 * What an installed app is and what it is doing (#575): its identity, grants and
 * contributions from the manifest, and its process, streams and recent errors from the
 * host, read on open and every five seconds after. The host keeps those in memory only.
 */
export function ExtensionInspector({
  plugin,
  busy,
  change,
  onViewLogs,
}: {
  plugin: InstalledExtension;
  busy: boolean;
  change(action: ExtensionChange): Promise<boolean>;
  /** Opens the Logs tab: the supervisor's "View logs" action. */
  onViewLogs(): void;
}) {
  const { manifest } = plugin;
  const id = manifest.id;
  const [inspection, setInspection] = useState<ExtensionInspection>();
  const [error, setError] = useState<string>();
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    let live = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const read = () =>
      inspectExtension(id).then(
        (answer) => {
          if (!live) return;
          setInspection(answer);
          timer = setTimeout(read, POLL_MS);
        },
        // No read while a failure stands: the reader chooses when to try again.
        (e: unknown) => live && setError(e instanceof Error ? e.message : String(e)),
      );
    void read();
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [id, attempt]);

  function retry() {
    setError(undefined);
    setInspection(undefined);
    setAttempt((n) => n + 1);
  }

  const contributed = contributions(plugin);
  return (
    <div className="extension-inspector">
      <Part title="Identity">
        <Facts
          rows={[
            ["ID", <code>{id}</code>],
            ["Version", plainText(manifest.version)],
            ["Extension API version", plainText(manifest.srelensApiVersion)],
            ["Revision", String(plugin.revision)],
            ["Source and signature", origin(plugin)],
            ["Installed", installedOn(plugin.installedAt)],
          ]}
        />
      </Part>

      <Part title="Grants">
        {plugin.grants.length === 0 ? (
          <p className="extension-message">No capabilities are granted.</p>
        ) : (
          <ul className="extension-grants" aria-label="Granted capabilities">
            {plugin.grants.map((grant) => (
              <li key={grant}>
                <code>{grant}</code> <span>{describeGrant(grant)}</span>
              </li>
            ))}
          </ul>
        )}
      </Part>

      <Part title="Active contributions">
        <p className="extension-message">{activity(plugin)}</p>
        {contributed.length === 0 ? (
          <p className="extension-message">This app contributes nothing.</p>
        ) : (
          <ul className="extension-inspector-counts">
            {contributed.map((each) => (
              <li key={each}>{each}</li>
            ))}
          </ul>
        )}
      </Part>

      <Part title="Registered capabilities">
        {manifest.capabilities.length === 0 ? (
          <p className="extension-message">No capabilities are registered.</p>
        ) : (
          <div className="extension-inspector-table">
            <table>
              <thead>
                <tr>
                  <th scope="col">Name</th>
                  <th scope="col">Title</th>
                  <th scope="col">Target</th>
                </tr>
              </thead>
              <tbody>
                {manifest.capabilities.map((capability) => (
                  <tr key={capability.name}>
                    <td>
                      <code>{plainText(capability.name)}</code>
                    </td>
                    <td>{plainText(capability.title)}</td>
                    <td>
                      <code>{plainText(capability.target)}</code>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </Part>

      {error !== undefined ? (
        <ErrorNotice title="Could not inspect this app" message={error} retry={retry} />
      ) : !inspection ? (
        <p role="status" className="extension-message">
          Loading…
        </p>
      ) : (
        <>
          <Part title="Process">
            <Process inspection={inspection} plugin={plugin} busy={busy} change={change} onViewLogs={onViewLogs} />
          </Part>

          <Part title="Streams and watches">
            <Streams label="Open streams" streams={inspection.streams.open} empty="No open streams." />
            <Streams label="Watches" streams={inspection.streams.watches} empty="No watches." />
            <Facts
              rows={[
                ["Opened since srelens started", count(inspection.streams.opened)],
                ["Received", `${plural(inspection.streams.messages, "message")}, ${bytes(inspection.streams.bytes)}`],
                ["Stopped for exceeding the message rate", count(inspection.streams.rateLimited)],
                [`Refused at the cap of ${count(inspection.streams.maxOpen)} open streams`, count(inspection.streams.refused)],
                ["Ended with their window", count(inspection.streams.windowEnded)],
              ]}
            />
          </Part>

          <Part title="Recent errors">
            {inspection.recentErrors.length === 0 ? (
              <p className="extension-message">
                {inspection.runtime === "declarative"
                  ? "No errors recorded since srelens started. Its pages and cards show their own failures where they happen."
                  : "No errors recorded since srelens started."}
              </p>
            ) : (
              // Focusable: it scrolls both ways, and a keyboard must be able to reach the rest.
              <div className="extension-app-log" role="log" aria-label="Recent errors" tabIndex={0}>
                {inspection.recentErrors.map((line) => (
                  <LogLineRow key={line.seq} line={line} level={false} />
                ))}
              </div>
            )}
          </Part>
        </>
      )}
    </div>
  );
}
