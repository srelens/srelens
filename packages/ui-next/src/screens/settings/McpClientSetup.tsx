import { useEffect, useState } from "react";
import { installSrelensCli, srelensCliStatus, mcpClientConfig, MCP_TOOLS, type CliStatus, type McpTool, type McpTransport } from "@srelens/core";
import { Button, Field, Panel, Select } from "@srelens/ui-kit";
import { FailureAlert } from "../../lib/errorCopy";

export function McpClientSetup({ url, token }: { url: string | null; token: string | null }) {
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
  const ready = transport === "stdio" || (!!url && !!token);
  const config = mcpClientConfig(tool, transport, { url: url ?? undefined, token });
  const preview = mcpClientConfig(tool, transport, { url: url ?? undefined, token: token && !revealed ? "<hidden token>" : token });
  async function copy() {
    if (!ready) return;
    try { await navigator.clipboard.writeText(config.snippet); setCopyState("copied"); }
    catch { setCopyState("failed"); }
  }
  return <>
    <Panel title="srelens CLI" description="Install the srelens command so MCP clients can start a stdio connection.">
      {loading ? <p role="status" className="text-[0.75rem] text-muted">Checking CLI installation…</p> :
        cli && <p className="text-[0.75rem] text-muted">{cli.installed ? <>Installed at <code>{cli.path}</code></> : "The srelens CLI is not installed."}</p>}
      <div className="mt-2 flex flex-wrap gap-2">
        <Button variant="secondary" disabled={loading || busy} onClick={() => void install()}>
          {busy ? "Installing…" : cli?.installed ? "Reinstall srelens CLI" : "Install srelens CLI"}
        </Button>
        {readError !== null && <Button variant="ghost" disabled={loading || busy} onClick={() => setNonce(n => n + 1)}>Retry status</Button>}
      </div>
      {cli?.installed && !cli.on_path && <p className="mt-2 text-[0.75rem] text-muted">Add <code>{cli.path.replace(/[/\\][^/\\]+$/, "")}</code> to PATH, then restart your MCP client.</p>}
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
      {transport === "http" && !ready ? <p className="mt-2 text-[0.75rem] text-muted">Start the MCP server to obtain its address and bearer token.</p> :
        <pre data-testid="mcp-client-config" className="scroll mt-2 whitespace-pre border-y border-rule py-2 text-[0.75rem]"><code>{preview.snippet}</code></pre>}
      <div className="mt-2 flex flex-wrap items-center gap-2">
        <Button variant="secondary" disabled={!ready} onClick={() => void copy()}>Copy configuration</Button>
        {transport === "http" && ready && <>
          <Button variant="ghost" onClick={() => setRevealed(value => !value)}>{revealed ? "Hide configuration token" : "Reveal configuration token"}</Button>
          <span className="text-[0.6875rem] text-muted">Copy includes the bearer token.</span>
        </>}
        {copyState === "copied" && <span role="status" className="text-[0.75rem]">Configuration copied.</span>}
        {copyState === "failed" && <span role="alert" className="text-[0.75rem]">Could not copy. Select the configuration and copy it manually.</span>}
      </div>
    </Panel>
  </>;
}
