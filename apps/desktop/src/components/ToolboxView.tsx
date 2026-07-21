import { useEffect, useState } from "react";
import { Download, RefreshCw, Search, Trash2, Wrench } from "lucide-react";
import { Badge, Button, PageHeader, SectionPanel, Spinner, StatusPill, TextInput } from "../ui";
import { ConfirmDialog } from "../ui/ConfirmDialog";
import { listContexts } from "../lib/clusters";
import {
  diagnoseContext,
  installKubectl,
  installPlugin,
  removePlugin,
  searchPlugins,
  startToolInstall,
  toolboxStatus,
  type DiagnosisReport,
  type Plugin,
  type RequirementStatus,
  type ToolStatus,
} from "../lib/toolbox";

const STATUS_KIND: Record<RequirementStatus, "success" | "warning" | "danger"> = {
  found: "success",
  "not-on-app-path": "warning",
  missing: "danger",
};

const STATUS_LABEL: Record<RequirementStatus, string> = {
  found: "Found",
  "not-on-app-path": "Not on app PATH",
  missing: "Missing",
};

/** The top-level Toolbox page: manage the CLI toolchain and diagnose contexts. */
export function ToolboxView({ initialContext }: { initialContext?: string | null }) {
  const [tools, setTools] = useState<ToolStatus[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [progress, setProgress] = useState<number | null>(null);
  const [error, setError] = useState("");

  const refreshStatus = async () => {
    const r = await toolboxStatus();
    setTools(r.data ?? []);
    setError(r.error ?? "");
  };
  useEffect(() => {
    void refreshStatus();
  }, []);

  const runInstall = async (label: string, fn: () => Promise<{ error?: string }>) => {
    setBusy(label);
    const r = await fn();
    if (r.error) setError(r.error);
    await refreshStatus();
    setBusy(null);
  };

  // Tools install with a streaming download progress bar.
  const installTool = async (tool: "kubectl" | "helm" | "krew") => {
    setBusy(tool);
    setProgress(null);
    const r = await startToolInstall(tool, setProgress);
    if (r.error) setError(r.error);
    await refreshStatus();
    setBusy(null);
    setProgress(null);
  };

  return (
    <div className="fl-toolbox">
      <PageHeader
        eyebrow="Toolbox"
        title="CLI toolchain"
        description="Install and manage the kubectl toolchain srelens drives, and diagnose what each context needs."
        actions={
          <Button variant="ghost" onClick={() => void refreshStatus()} aria-label="Refresh">
            <RefreshCw data-icon="inline-start" /> Refresh
          </Button>
        }
      />

      {error && (
        <p className="fl-toolbox-error" role="alert">
          {error}
        </p>
      )}

      <SectionPanel title="Tools" description="kubectl, krew and helm. srelens installs into ~/.srelens/bin and never touches system installs.">
        {tools === null ? (
          <Spinner />
        ) : (
          <div className="fl-toolbox-tools">
            {tools.map((tool) => (
              <ToolCard
                key={tool.name}
                tool={tool}
                busy={busy === tool.name}
                progress={busy === tool.name ? progress : undefined}
                onInstall={() => installTool(tool.name as "kubectl" | "helm" | "krew")}
              />
            ))}
          </div>
        )}
      </SectionPanel>

      <PluginsSection />

      <ContextHealthSection initialContext={initialContext ?? null} onInstall={runInstall} busy={busy} />
    </div>
  );
}

function ToolCard({
  tool,
  busy,
  progress,
  onInstall,
}: {
  tool: ToolStatus;
  busy: boolean;
  /** Download percent while installing (null = unknown/finishing), undefined when idle. */
  progress?: number | null;
  onInstall: () => void;
}) {
  return (
    <div className="fl-toolbox-card">
      <div className="fl-toolbox-card__head">
        <Wrench aria-hidden="true" />
        <strong>{tool.name}</strong>
        {tool.installed && tool.source && (
          <Badge variant={tool.source === "managed" ? "info" : "neutral"}>{tool.source}</Badge>
        )}
      </div>
      {tool.installed ? (
        <>
          <p className="fl-toolbox-card__meta">
            <code>{tool.version ?? "installed"}</code>
          </p>
          <small className="fl-toolbox-card__path">{tool.path}</small>
        </>
      ) : (
        <>
          <p className="fl-toolbox-card__meta">
            {busy ? (progress != null ? `Downloading… ${progress}%` : "Installing…") : "Not installed"}
          </p>
          <Button onClick={onInstall} disabled={busy} aria-label={`Install ${tool.name}`}>
            {busy ? <Spinner /> : <Download data-icon="inline-start" />} Install
          </Button>
        </>
      )}
    </div>
  );
}

function PluginsSection() {
  const [query, setQuery] = useState("");
  const [plugins, setPlugins] = useState<Plugin[] | null>(null);
  const [searching, setSearching] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [confirm, setConfirm] = useState<string | null>(null);
  const [error, setError] = useState("");

  const runSearch = async () => {
    if (!query.trim()) return;
    setSearching(true);
    const r = await searchPlugins(query.trim());
    setPlugins(r.data ?? []);
    setError(r.error ?? "");
    setSearching(false);
  };

  const doInstall = async (name: string) => {
    setConfirm(null);
    setBusy(name);
    const r = await installPlugin(name);
    if (r.error) setError(r.error);
    await runSearch();
    setBusy(null);
  };

  const doRemove = async (name: string) => {
    setBusy(name);
    const r = await removePlugin(name);
    if (r.error) setError(r.error);
    await runSearch();
    setBusy(null);
  };

  return (
    <SectionPanel title="Plugins" description="Search the krew index and install kubectl plugins.">
      <form
        className="fl-toolbox-search"
        onSubmit={(e) => {
          e.preventDefault();
          void runSearch();
        }}
      >
        <TextInput value={query} onValueChange={setQuery} onEnter={() => void runSearch()} type="search" placeholder="Search krew plugins…" aria-label="Search krew plugins" />
        <Button type="submit" variant="secondary" disabled={searching}>
          {searching ? <Spinner /> : <Search data-icon="inline-start" />} Search
        </Button>
      </form>

      {error && (
        <p className="fl-toolbox-error" role="alert">
          {error}
        </p>
      )}

      {plugins?.length === 0 && <p className="fl-toolbox-empty">No plugins match “{query}”.</p>}

      {plugins && plugins.length > 0 && (
        <table className="fl-toolbox-plugins">
          <tbody>
            {plugins.map((p) => (
              <tr key={p.name}>
                <td>
                  <strong>{p.name}</strong>
                  <small>{p.description}</small>
                </td>
                <td className="fl-toolbox-plugins__action">
                  {p.installed ? (
                    <Button variant="ghost" onClick={() => void doRemove(p.name)} disabled={busy === p.name} aria-label={`Remove ${p.name}`}>
                      {busy === p.name ? <Spinner /> : <Trash2 data-icon="inline-start" />} Remove
                    </Button>
                  ) : (
                    <Button variant="secondary" onClick={() => setConfirm(p.name)} disabled={busy === p.name}>
                      {busy === p.name ? <Spinner /> : <Download data-icon="inline-start" />} Install
                    </Button>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}

      {confirm && (
        <ConfirmDialog
          title={`Install ${confirm}?`}
          message="krew plugins are community-maintained and run with your kubectl. Only install plugins you trust."
          confirmLabel="Install"
          onConfirm={() => void doInstall(confirm)}
          onCancel={() => setConfirm(null)}
        />
      )}
    </SectionPanel>
  );
}

function ContextHealthSection({
  initialContext,
  onInstall,
  busy,
}: {
  initialContext: string | null;
  onInstall: (label: string, fn: () => Promise<{ error?: string }>) => Promise<void>;
  busy: string | null;
}) {
  const [contexts, setContexts] = useState<string[]>([]);
  const [reports, setReports] = useState<Record<string, DiagnosisReport | "loading">>({});

  useEffect(() => {
    void listContexts().then((o) => setContexts((o.contexts ?? []).map((c) => c.name)));
  }, []);

  const diagnose = async (context: string) => {
    setReports((r) => ({ ...r, [context]: "loading" }));
    const out = await diagnoseContext(context);
    if (out.data) setReports((r) => ({ ...r, [context]: out.data! }));
  };

  useEffect(() => {
    if (initialContext) void diagnose(initialContext);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [initialContext]);

  return (
    <SectionPanel title="Context health" description="Diagnose the exec-auth tools each context needs.">
      {contexts.length === 0 && <p className="fl-toolbox-empty">No contexts loaded.</p>}
      <ul className="fl-toolbox-contexts">
        {contexts.map((ctx) => {
          const report = reports[ctx];
          return (
            <li key={ctx} className="fl-toolbox-context">
              <div className="fl-toolbox-context__head">
                <span>{ctx}</span>
                <Button variant="ghost" onClick={() => void diagnose(ctx)} disabled={report === "loading"}>
                  {report === "loading" ? <Spinner /> : "Check"}
                </Button>
              </div>
              {report && report !== "loading" && (
                report.items.length === 0 ? (
                  <StatusPill status="No external tools needed" kind="success" />
                ) : (
                  <ul className="fl-toolbox-reqs">
                    {report.items.map((item) => (
                      <li key={item.binary} className="fl-toolbox-req">
                        <code>{item.binary}</code>
                        <StatusPill status={STATUS_LABEL[item.status]} kind={STATUS_KIND[item.status]} />
                        {item.status === "missing" && item.installable && (
                          <Button
                            variant="secondary"
                            disabled={busy !== null}
                            onClick={() =>
                              onInstall(
                                item.binary,
                                item.kind === "kubectl"
                                  ? installKubectl
                                  : () => installPlugin(item.plugin ?? item.binary),
                              )
                            }
                          >
                            {busy === item.binary ? <Spinner /> : "Install"}
                          </Button>
                        )}
                        {item.status === "missing" && !item.installable && (
                          <small className="fl-toolbox-req__hint">install {item.binary} yourself</small>
                        )}
                      </li>
                    ))}
                  </ul>
                )
              )}
            </li>
          );
        })}
      </ul>
    </SectionPanel>
  );
}
