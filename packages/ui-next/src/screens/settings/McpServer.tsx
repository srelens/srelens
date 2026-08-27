import { useEffect, useState } from "react";
import { getMcpToken, notify, revokeMcpToken, rotateMcpToken } from "@srelens/core";
import { Badge, Button, Panel, SubHead } from "@srelens/ui-kit";
import { FailureAlert } from "../../lib/errorCopy";

/**
 * §23's `MCP server` pane: the loopback HTTP transport's own bearer token —
 * masked until the reader asks for it, with reveal, copy, rotate and revoke.
 *
 * **The address is fixed, not read back.** `start_server` (`apps/desktop/src-
 * tauri/src/mcp.rs`) binds `Ipv4Addr::LOCALHOST` unconditionally; there is no
 * setting anywhere for it, so there is nothing here to read from a running
 * server that a constant wouldn't already say.
 *
 * **`running` is inferred from the token, not read live.** The only signal
 * this pane's given interfaces expose is `getMcpToken()`, and the backend's
 * own invariant makes one direction of that inference sound: `mcp_token_get`
 * / `mcp_token_revoke` (`mcp.rs`) guarantee the HTTP transport is "never...
 * unauthenticated", so NO TOKEN really does mean NOT RUNNING. The other
 * direction is weaker — `mcp_token_rotate` can mint a token while the server
 * stays stopped ("rotating a token must never switch the server on"), and
 * nothing restarts it on launch, so a token surviving from an earlier session
 * can read as `running` here before anything is actually listening. A live
 * signal exists (`mcpHttpStatus()`, `packages/core/src/lib/mcp.ts`) but is not
 * part of this pane's given interfaces — flagged for the branch's review
 * rather than reached past what this task was scoped to.
 *
 * **`getMcpTokenStorage()` is deliberately NOT read here**, despite being
 * named for this file. Its own doc comment says what it actually reports:
 * where the *secrets vault's master key* lives (keychain / file / locked) —
 * `mcp_token_storage` (`mcp.rs`) returns `vault.key_source()`. The MCP bearer
 * itself is a plain `FileTokenStore` (`crates/mcp/src/auth.rs`), unrelated to
 * the vault; captioning this pane with the vault's key source would tell the
 * reader something false about how THIS credential is protected. Classic's
 * `McpSettingsSection` never called it either — only `SecuritySettingsSection`
 * did, and that pane (not this one) is where vault key storage belongs.
 *
 * **`Clients` is not drawn** (#369): `mcpClientConfig` generates configuration
 * *for* a client to paste elsewhere; srelens does not track who connects.
 */

const ADDRESS = "127.0.0.1:8765";

/**
 * How many bullets the mask shows. FIXED, independent of the real token's
 * length — a mask that grew with the secret would tell a reader (or a
 * screenshot) how long it is, which is exactly the property this pane must
 * not leak. The literal `srl_` label is cosmetic, not sliced from the real
 * token: the backend mints a 64-character hex string with no prefix
 * (`Token::generate`, `crates/mcp/src/auth.rs`).
 */
const MASK_BULLETS = 12;

function maskedToken(): string {
  return `srl_${"•".repeat(MASK_BULLETS)}`;
}

export function McpServer() {
  const [token, setToken] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [revealed, setRevealed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);

  useEffect(() => {
    let cancelled = false;
    getMcpToken()
      .then((t) => {
        if (!cancelled) setToken(t);
      })
      .catch((e) => {
        if (!cancelled) setError(e);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const running = token !== null;

  async function copyToken() {
    if (!token) return;
    try {
      await navigator.clipboard.writeText(token);
      notify.success("Copied to clipboard");
    } catch {
      // No clipboard on a non-secure origin, and nothing to recover: Reveal
      // already shows the value in full and it can be selected by hand.
    }
  }

  async function rotate() {
    setBusy(true);
    setError(null);
    try {
      const next = await rotateMcpToken();
      setToken(next);
      setRevealed(false);
    } catch (e) {
      setError(e);
    } finally {
      setBusy(false);
    }
  }

  async function revoke() {
    setBusy(true);
    setError(null);
    try {
      await revokeMcpToken();
      setToken(null);
      setRevealed(false);
    } catch (e) {
      setError(e);
    } finally {
      setBusy(false);
    }
  }

  return (
    <Panel
      title={
        <span className="flex flex-wrap items-center gap-2">
          <span>MCP server · loopback http · {ADDRESS}</span>
          {!loading && <Badge tone={running ? "ok" : "muted"}>{running ? "running" : "not running"}</Badge>}
        </span>
      }
    >
      {loading ? (
        <p className="text-[0.75rem] text-muted">Checking the MCP server…</p>
      ) : running ? (
        <>
          <SubHead className="mt-1">Bearer token</SubHead>
          {/* `min-w-0` on the flex child holding the token: a 64-character
              secret beside four controls is the exact shape that has bitten
              this migration eight times under `min-width: auto`. `break-all`,
              not `truncate` — the revealed value must stay fully readable and
              carries no `title`, the same rule `CopyCommand` follows for the
              same reason. */}
          <div className="mt-2 flex items-center gap-2">
            <code className="code min-w-0 flex-1 break-all rounded px-2 py-1 text-[0.75rem]">
              {revealed ? token : maskedToken()}
            </code>
            <Button variant="ghost" size="sm" onClick={() => setRevealed((r) => !r)}>
              {revealed ? "Hide" : "Reveal"}
            </Button>
            <Button variant="ghost" size="sm" onClick={() => void copyToken()}>
              Copy
            </Button>
            <Button variant="secondary" size="sm" disabled={busy} onClick={() => void rotate()}>
              Rotate
            </Button>
            <Button variant="danger" size="sm" disabled={busy} onClick={() => void revoke()}>
              Revoke
            </Button>
          </div>
          <p className="mt-2 text-[0.75rem] leading-relaxed text-muted">
            Rotating restarts the server, drops in-flight requests, and invalidates clients still using the old
            token.
          </p>
        </>
      ) : (
        <p className="text-[0.75rem] leading-relaxed text-muted">
          No bearer token has been generated, so the MCP server is not accepting connections — there is nothing to
          reveal, copy or rotate yet.
        </p>
      )}

      {error !== null && (
        <FailureAlert tone="sev" title="The MCP server's token could not be updated" error={error} />
      )}

      {/* #369: mcpClientConfig GENERATES config FOR a client to paste
          elsewhere; srelens never learns who used it, so a `Clients` list
          (§23 draws one) would be a claim this pane cannot back. */}
      <p className="mt-3 text-[0.75rem] leading-relaxed text-muted">
        srelens cannot say which clients are connected right now — it only generates configuration for a client to
        paste elsewhere, and never learns who used it.
      </p>
    </Panel>
  );
}
