import React, { useEffect, useState } from "react";
import { Copy, Download, Eye, EyeOff, Radio, RefreshCw, Trash2 } from "lucide-react";
import { Button, ConfirmDialog, TextInput } from "../ui";
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
import { getMcpToken, getMcpTokenStorage, revokeMcpToken, rotateMcpToken } from "../lib/mcpSecurity";
import { mcpClientConfig, MCP_TOOLS, type McpTool, type McpTransport } from "../lib/mcpClients";
import { McpAuditList } from "./McpAuditList";
import { McpPromptIssues } from "./McpPromptIssues";

/** Masked by default: only the last 4 characters are shown until revealed. */
function maskToken(token: string): string {
  return `••••${token.slice(-4)}`;
}

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
  // `token` is `null` both while it's still loading and once we know for
  // certain there isn't one yet (e.g. the server has never been enabled, so
  // `mcp_http_start` — the only place that mints one — hasn't run). `tokenLoading`
  // tells those two apart so the row can show "Loading…" only for the former
  // and a real "generate one" action for the latter, instead of getting stuck
  // on "Loading…" forever.
  const [token, setToken] = useState<string | null>(null);
  const [tokenLoading, setTokenLoading] = useState(true);
  const [tokenRevealed, setTokenRevealed] = useState(false);
  const [tokenBusy, setTokenBusy] = useState(false);
  const [tokenError, setTokenError] = useState("");
  const [tokenConfirm, setTokenConfirm] = useState<"rotate" | "revoke" | null>(null);
  const [tokenStorage, setTokenStorage] = useState<"keychain" | "file" | "locked" | null>(null);
  // Bumped by McpAuditList's own Refresh button so the prompt-issues panel
  // re-reads too: a user who fixes their prompt file should see that
  // reflected without restarting srelens, and without a second Refresh
  // button next to the one that already exists.
  const [promptIssuesNonce, setPromptIssuesNonce] = useState(0);

  async function refreshToken() {
    try {
      setToken(await getMcpToken());
    } catch {
      setToken(null);
    } finally {
      setTokenLoading(false);
    }
  }

  useEffect(() => {
    void mcpHttpStatus().then(setRunningUrl).catch(() => {});
    void srelensCliStatus().then(setCli).catch(() => {});
    void refreshToken();
    void getMcpTokenStorage().then(setTokenStorage).catch(() => {});
  }, []);

  function persist(next: McpSettings) {
    setSettings(next);
    saveMcpSettings(next);
  }

  async function toggleServer(enabled: boolean) {
    setServerError("");
    persist({ ...settings, enabled });
    try {
      if (enabled) {
        setRunningUrl(await startMcpHttp(settings.port));
        // Starting the server for the first time is also the first place a
        // token can get minted (`mcp_http_start` mints one on first use), so
        // the previously-fetched `null` may now be stale.
        void refreshToken();
      } else {
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

  async function confirmRotate() {
    setTokenBusy(true);
    setTokenError("");
    try {
      const next = await rotateMcpToken();
      setToken(next);
      setTokenRevealed(false);
      notify.success("Token rotated — the server restarted and connected clients need the new value.");
      setTokenConfirm(null);
    } catch (e) {
      setTokenError(String(e));
    } finally {
      setTokenBusy(false);
    }
  }

  async function confirmRevoke() {
    setTokenBusy(true);
    setTokenError("");
    try {
      await revokeMcpToken();
      await refreshToken();
      setTokenRevealed(false);
      notify.success("Token revoked. The MCP HTTP server has stopped.");
      // The server stops as a side effect of revocation; reflect that instead
      // of leaving the toggle showing a server that's no longer listening.
      const status = await mcpHttpStatus().catch(() => null);
      setRunningUrl(status);
      if (!status) persist({ ...settings, enabled: false });
      setTokenConfirm(null);
    } catch (e) {
      setTokenError(String(e));
    } finally {
      setTokenBusy(false);
    }
  }

  const url = runningUrl ?? `http://127.0.0.1:${settings.port}/mcp`;
  const config = mcpClientConfig(tool, transport, { url, token });

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

      {/* Access token */}
      <section className="flex flex-col gap-2">
        <h4 className="text-sm font-medium">Access token</h4>
        <p className="text-sm text-muted-foreground">
          The HTTP transport requires this token — clients must send it as{" "}
          <code className="fl-mono">Authorization: Bearer &lt;token&gt;</code>. Stdio connections (spawned
          via the CLI) don't need it.
        </p>
        <div className="flex flex-wrap items-center gap-2">
          <code className="fl-mono rounded-md border border-border bg-muted/40 px-2 py-1 text-sm">
            {tokenLoading ? "Loading…" : token ? (tokenRevealed ? token : maskToken(token)) : "No token yet"}
          </code>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => setTokenRevealed((r) => !r)}
            disabled={!token}
            aria-label={tokenRevealed ? "Hide token" : "Reveal token"}
          >
            {tokenRevealed ? <EyeOff data-icon="inline-start" /> : <Eye data-icon="inline-start" />}
            {tokenRevealed ? "Hide" : "Reveal"}
          </Button>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => token && void copy(token)}
            disabled={!token}
            aria-label="Copy token"
          >
            <Copy data-icon="inline-start" />
            Copy
          </Button>
          <Button
            variant="secondary"
            size="sm"
            // Rotating an existing token restarts a running server and drops
            // in-flight requests, so that path still confirms first. Minting
            // the very first token has nothing to warn about — there's no
            // previous value to invalidate — so it just runs.
            onClick={() => (token ? setTokenConfirm("rotate") : void confirmRotate())}
            disabled={tokenLoading || tokenBusy}
          >
            <RefreshCw data-icon="inline-start" />
            {token ? "Rotate token" : "Generate token"}
          </Button>
          <Button
            variant="danger"
            size="sm"
            onClick={() => setTokenConfirm("revoke")}
            disabled={!token || tokenBusy}
          >
            <Trash2 data-icon="inline-start" />
            Revoke token
          </Button>
        </div>
        {!tokenLoading && !token && (
          <p className="text-sm text-muted-foreground">
            No token has been generated yet. Generate one to connect over HTTP — stdio connections
            don't need it.
          </p>
        )}
        {tokenStorage === "file" && (
          <p className="text-sm text-amber-600 dark:text-amber-500">
            No OS keychain is available here, so the key that encrypts srelens's secrets is stored
            in a plain file on disk (readable only by your user account) rather than the OS
            keychain — the encrypted secrets file is then only obfuscation.
          </p>
        )}
        {tokenStorage === "locked" && (
          <p className="text-sm text-destructive">
            srelens couldn't load the key that encrypts its secrets when it started — the OS
            keychain was unreachable, or the key file couldn't be created. Stored secrets can't be
            read or changed right now; they are untouched. Restart srelens once the keychain is
            available again.
          </p>
        )}
        {tokenError && <p className="text-sm text-destructive">Error: {tokenError}</p>}
      </section>

      {tokenConfirm && (
        <ConfirmDialog
          title={tokenConfirm === "rotate" ? "Rotate access token?" : "Revoke access token?"}
          message={
            tokenConfirm === "rotate" ? (
              "If the MCP HTTP server is running, rotating restarts it immediately so the new token takes effect — any in-flight agent request is dropped. Connected clients need the new value or they'll stop working."
            ) : (
              "Revoking also stops the MCP HTTP server: it never serves without a valid token. Any clients connected over HTTP will disconnect."
            )
          }
          confirmLabel={tokenConfirm === "rotate" ? "Rotate" : "Revoke"}
          danger={tokenConfirm === "revoke"}
          busy={tokenBusy}
          onConfirm={() => void (tokenConfirm === "rotate" ? confirmRotate() : confirmRevoke())}
          onCancel={() => setTokenConfirm(null)}
        />
      )}

      {/* Recent agent activity */}
      <section className="flex flex-col gap-2">
        <h4 className="text-sm font-medium">Recent agent activity</h4>
        <McpPromptIssues nonce={promptIssuesNonce} />
        <McpAuditList onRefresh={() => setPromptIssuesNonce((n) => n + 1)} />
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
