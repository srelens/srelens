import React, { useEffect, useState } from "react";
import { Copy, Download, Radio } from "lucide-react";
import { Button, TextInput } from "../ui";
import { notify } from "../lib/notify";
import {
  loadMcpSettings,
  saveMcpSettings,
  type McpSettings,
} from "../lib/settings";
import {
  startMcpHttp,
  stopMcpHttp,
  mcpHttpStatus,
  installSrelensCli,
  srelensCliStatus,
  type CliStatus,
} from "../lib/mcp";
import { mcpClientConfig, MCP_TOOLS, type McpTool, type McpTransport } from "../lib/mcpClients";

async function copy(text: string) {
  try {
    await navigator.clipboard?.writeText(text);
    notify.success("Copied to clipboard");
  } catch {
    notify.error("Could not copy");
  }
}

/**
 * Settings → MCP. Toggles the in-app loopback MCP HTTP server, installs the
 * `srelens` CLI onto PATH (so clients can spawn `srelens --mcp-stdio`), and
 * shows ready-to-paste config for each MCP client.
 */
export function McpSettingsSection() {
  const [settings, setSettings] = useState<McpSettings>(loadMcpSettings);
  const [runningUrl, setRunningUrl] = useState<string | null>(null);
  const [serverError, setServerError] = useState("");
  const [cli, setCli] = useState<CliStatus | null>(null);
  const [cliMessage, setCliMessage] = useState("");
  const [tool, setTool] = useState<McpTool>("claude-code");
  const [transport, setTransport] = useState<McpTransport>("stdio");

  useEffect(() => {
    void mcpHttpStatus().then(setRunningUrl).catch(() => {});
    void srelensCliStatus().then(setCli).catch(() => {});
  }, []);

  function persist(next: McpSettings) {
    setSettings(next);
    saveMcpSettings(next);
  }

  async function toggleServer(enabled: boolean) {
    setServerError("");
    persist({ ...settings, enabled });
    try {
      if (enabled) setRunningUrl(await startMcpHttp(settings.port));
      else {
        await stopMcpHttp();
        setRunningUrl(null);
      }
    } catch (e) {
      setServerError(String(e));
      persist({ ...settings, enabled: false });
      setRunningUrl(null);
    }
  }

  async function changePort(value: string) {
    const port = Number(value);
    if (!Number.isInteger(port) || port <= 0 || port >= 65536) return;
    persist({ ...settings, port });
    if (settings.enabled) {
      setServerError("");
      try {
        setRunningUrl(await startMcpHttp(port));
      } catch (e) {
        setServerError(String(e));
      }
    }
  }

  async function installCli() {
    setCliMessage("");
    try {
      const path = await installSrelensCli();
      setCliMessage(`Installed at ${path}`);
      setCli(await srelensCliStatus());
      notify.success("srelens CLI installed");
    } catch (e) {
      setCliMessage(String(e));
    }
  }

  const url = runningUrl ?? `http://127.0.0.1:${settings.port}/mcp`;
  const config = mcpClientConfig(tool, transport, { url });

  return (
    <div className="flex flex-col gap-6">
      <p className="text-sm text-muted-foreground">
        srelens is MCP-native: every action it can take is exposed as an MCP tool, so agents and other
        MCP clients can drive your clusters. Connect over stdio (spawning the srelens CLI) or the
        loopback HTTP server below.
      </p>

      {/* Server toggle */}
      <section className="flex flex-col gap-3">
        <div className="flex flex-wrap items-center gap-3">
          <label className="flex items-center gap-2 text-sm font-medium">
            <input
              type="checkbox"
              checked={settings.enabled}
              onChange={(e) => void toggleServer(e.target.checked)}
              aria-label="Run MCP HTTP server"
            />
            Run MCP server (HTTP) while srelens is open
          </label>
          <label className="ml-auto flex items-center gap-2 text-sm text-muted-foreground">
            Port
            <TextInput
              type="number"
              value={String(settings.port)}
              onValueChange={changePort}
              aria-label="MCP server port"
              className="w-24"
            />
          </label>
        </div>
        {settings.enabled && runningUrl && (
          <div className="flex items-center gap-2 text-sm">
            <Radio className="size-4 text-green-600 dark:text-green-500" aria-hidden />
            <span>Listening at</span>
            <code className="fl-mono">{runningUrl}</code>
            <Button variant="ghost" size="sm" onClick={() => void copy(runningUrl)} aria-label="Copy MCP URL">
              <Copy data-icon="inline-start" />
              Copy
            </Button>
          </div>
        )}
        {serverError && <p className="text-sm text-destructive">Error: {serverError}</p>}
      </section>

      {/* CLI install */}
      <section className="flex flex-col gap-2">
        <h4 className="text-sm font-medium">srelens CLI</h4>
        <p className="text-sm text-muted-foreground">
          Installs a <code className="fl-mono">srelens</code> command on your PATH so MCP clients can
          spawn <code className="fl-mono">srelens --mcp-stdio</code>.
        </p>
        <div className="flex flex-wrap items-center gap-3">
          <Button onClick={() => void installCli()}>
            <Download data-icon="inline-start" />
            {cli?.installed ? "Reinstall srelens CLI" : "Install srelens CLI"}
          </Button>
          {cli?.installed && (
            <span className="text-sm text-muted-foreground">
              Installed at <code className="fl-mono">{cli.path}</code>
            </span>
          )}
        </div>
        {cli?.installed && !cli.on_path && (
          <p className="text-sm text-amber-600 dark:text-amber-500">
            Its directory isn't on your PATH yet — add it (e.g.{" "}
            <code className="fl-mono">export PATH="$HOME/.local/bin:$PATH"</code>) so clients can find{" "}
            <code className="fl-mono">srelens</code>.
          </p>
        )}
        {cliMessage && <p className="whitespace-pre-wrap text-sm text-muted-foreground">{cliMessage}</p>}
      </section>

      {/* Per-tool config */}
      <section className="flex flex-col gap-3">
        <h4 className="text-sm font-medium">Connect a client</h4>
        <div className="flex flex-wrap gap-1" role="group" aria-label="MCP client">
          {MCP_TOOLS.map((t) => (
            <Button
              key={t.id}
              variant={tool === t.id ? "primary" : "ghost"}
              size="sm"
              aria-pressed={tool === t.id}
              onClick={() => setTool(t.id)}
            >
              {t.label}
            </Button>
          ))}
        </div>
        <div className="flex gap-1" role="group" aria-label="Transport">
          {(["stdio", "http"] as McpTransport[]).map((tr) => (
            <Button
              key={tr}
              variant={transport === tr ? "primary" : "ghost"}
              size="sm"
              aria-pressed={transport === tr}
              onClick={() => setTransport(tr)}
            >
              {tr}
            </Button>
          ))}
        </div>
        <p className="text-xs text-muted-foreground">{config.hint}</p>
        <div className="relative">
          <pre className="max-h-64 overflow-auto rounded-md border border-border bg-muted/40 p-3 text-xs">
            <code>{config.snippet}</code>
          </pre>
          <Button
            variant="ghost"
            size="sm"
            className="absolute right-2 top-2"
            onClick={() => void copy(config.snippet)}
            aria-label="Copy config"
          >
            <Copy data-icon="inline-start" />
            Copy
          </Button>
        </div>
      </section>
    </div>
  );
}
