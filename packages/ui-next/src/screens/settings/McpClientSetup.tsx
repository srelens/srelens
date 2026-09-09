import { useEffect, useState } from "react";
import { installSrelensCli, srelensCliStatus, mcpClientConfig, MCP_TOOLS, type CliStatus, type McpTool, type McpTransport } from "@srelens/core";
import { Button, Field, Panel, Select } from "@srelens/ui-kit";
import { FailureAlert } from "../../lib/errorCopy";

interface McpClientSetupProps {
  url: string | null;
  token: string | null;
  statusLoading?: boolean;
  tokenLoading?: boolean;
  statusError?: unknown;
  tokenError?: unknown;
  onRetryStatus?: () => void;
  onRetryToken?: () => void;
}

export function McpClientSetup({ url, token, statusLoading = false, tokenLoading = false,
  statusError, tokenError, onRetryStatus, onRetryToken }: McpClientSetupProps) {
  const [cli, setCli] = useState<CliStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [readError, setReadError] = useState<unknown>(null);
  const [installError, setInstallError] = useState<unknown>(null);
  const [installedAt, setInstalledAt] = useState("");
  const [nonce, setNonce] = useState(0);
  const [tool, setTool] = useState<McpTool>("claude-code");
  const [transport, setTransport] = useState<McpTransport>("stdio");
  const [copyState, setCopyState] = useState<"idle" | "copied" | "failed">("idle");
  const [revealed, setRevealed] = useState(false);
  const windows = /win/i.test(navigator.platform ?? "");

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    srelensCliStatus().then(status => {
      if (!cancelled) { setCli(status); setReadError(null); }
    }).catch(error => { if (!cancelled) { setCli(null); setReadError(error); } })
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, [nonce]);
  useEffect(() => { setCopyState("idle"); setRevealed(false); }, [url, token, tool, transport]);

  async function install() {
    setBusy(true); setInstallError(null); setInstalledAt("");
    try { setInstalledAt(await installSrelensCli()); setNonce(n => n + 1); }
    catch (error) { setInstallError(error); }
    finally { setBusy(false); }
  }
  const command = windows ? cli?.path || undefined : undefined;
  const ready = transport === "stdio" ? (!windows || !!command) : (!statusLoading && !tokenLoading && statusError === undefined && tokenError === undefined && !!url && !!token);
  const config = mcpClientConfig(tool, transport, { url: url ?? undefined, token, command });
  const preview = mcpClientConfig(tool, transport, { url: url ?? undefined, token: token && !revealed ? "<hidden token>" : token, command });
  async function copy() {
    if (!ready) return;
    try { await navigator.clipboard.writeText(config.snippet); setCopyState("copied"); }
    catch { if (transport === "http") setRevealed(true); setCopyState("failed"); }
  }
  return <>
    <Panel title="srelens CLI" description={windows ? "Windows MCP clients start the installed desktop executable directly." : "Install the srelens command so MCP clients can start a stdio connection."}>
      {loading ? <p role="status" className="text-[0.75rem] text-muted">Checking CLI installation…</p> :
        cli && <p className="text-[0.75rem] text-muted">{windows ? (cli.path ? <>Using <code>{cli.path}</code></> : "The desktop executable could not be located.") :
          cli.installed ? <>Installed at <code>{cli.path}</code></> : "The srelens CLI is not installed."}</p>}
      <div className="mt-2 flex flex-wrap gap-2">
        {!windows && <Button variant="secondary" disabled={loading || busy} onClick={() => void install()}>
          {busy ? "Installing…" : cli?.installed ? "Reinstall srelens CLI" : "Install srelens CLI"}
        </Button>}
        {readError !== null && <Button variant="ghost" disabled={loading || busy} onClick={() => setNonce(n => n + 1)}>Retry status</Button>}
      </div>
      {!windows && cli?.installed && !cli.on_path && <p className="mt-2 text-[0.75rem] text-muted">Add <code>{cli.path.replace(/[/\\][^/\\]+$/, "")}</code> to PATH, then restart your MCP client.</p>}
      {installedAt && !cli?.installed && <p role="status" className="mt-2 text-[0.75rem]">CLI installed at <code>{installedAt}</code>.</p>}
      {readError !== null && <FailureAlert tone="sev" title="Could not check the CLI installation" error={readError} />}
      {installError !== null && <FailureAlert tone="sev" title="Could not install the srelens CLI" error={installError} />}
    </Panel>
    <Panel title="Connect a client">
      <div className="flex flex-wrap gap-3">
        <Field label="MCP client"><Select value={tool} onValueChange={value => setTool(value as McpTool)} options={MCP_TOOLS.map(t => ({ value: t.id, label: t.label }))} /></Field>
        <Field label="Transport"><Select value={transport} onValueChange={value => setTransport(value as McpTransport)} options={[{ value: "stdio", label: "stdio" }, { value: "http", label: "HTTP" }]} /></Field>
      </div>
      <p className="mt-2 text-[0.75rem] text-muted">{preview.hint}</p>
      {transport === "http" && (statusLoading || tokenLoading) ? <p role="status" className="mt-2 text-[0.75rem] text-muted">Checking the MCP server address and bearer token…</p> :
        transport === "http" && (statusError !== undefined || tokenError !== undefined) ? <div className="mt-2 space-y-2">
          {statusError !== undefined && <><FailureAlert tone="sev" title="The MCP server status could not be read" error={statusError} />
            {onRetryStatus && <Button variant="ghost" onClick={onRetryStatus}>Retry server status</Button>}</>}
          {tokenError !== undefined && <><FailureAlert tone="sev" title="The MCP bearer token could not be read" error={tokenError} />
            {onRetryToken && <Button variant="ghost" onClick={onRetryToken}>Retry bearer token</Button>}</>}
        </div> : transport === "http" && !ready ? <p className="mt-2 text-[0.75rem] text-muted">Start the MCP server to obtain its address and bearer token.</p> :
        transport === "stdio" && !ready ? <p className="mt-2 text-[0.75rem] text-muted">The desktop executable path is required before a Windows stdio configuration can be generated.</p> :
        <pre data-testid="mcp-client-config" className="scroll mt-2 whitespace-pre border-y border-rule py-2 text-[0.75rem]"><code>{preview.snippet}</code></pre>}
      <div className="mt-2 flex flex-wrap items-center gap-2">
        <Button variant="secondary" disabled={!ready} onClick={() => void copy()}>Copy configuration</Button>
        {transport === "http" && ready && <>
          <Button variant="ghost" onClick={() => setRevealed(value => !value)}>{revealed ? "Hide configuration token" : "Reveal configuration token"}</Button>
          <span className="text-[0.6875rem] text-muted">Copy includes the bearer token.</span>
        </>}
        {copyState === "copied" && <span role="status" className="text-[0.75rem]">Configuration copied.</span>}
        {copyState === "failed" && <span role="alert" className="text-[0.75rem]">Could not copy. The usable configuration is revealed above; select it and copy manually.</span>}
      </div>
    </Panel>
  </>;
}
