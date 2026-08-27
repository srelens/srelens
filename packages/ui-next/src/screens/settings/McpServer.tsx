import { useEffect, useRef, useState } from "react";
import { getMcpToken, mcpHttpStatus, notify, revokeMcpToken, rotateMcpToken } from "@srelens/core";
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
 * **`running` is `mcpHttpStatus()` — a live read of the process, not a proxy.**
 * A token existing is not the same fact as a listener being bound: rotate can
 * mint a fresh token while the server stays stopped ("rotating a token must
 * never switch the server on", `mcp.rs`), and nothing here restarts it on
 * launch, so a token surviving from an earlier session says nothing about
 * whether anything is on 127.0.0.1:8765 right now. `mcpHttpStatus`
 * (`packages/core/src/lib/mcp.ts`) reads `McpHttpManager`'s own `running`
 * state (`mcp.rs`) directly — the one place that fact actually lives.
 *
 * **The token and the listener are independent facts, read independently.**
 * `getMcpToken()` and `mcpHttpStatus()` run in their OWN effects, not a
 * shared `Promise.all` — a status check failing must not cost a token that
 * was fetched successfully, and vice versa. A round of review caught the
 * combined form doing exactly that: a rejecting `mcpHttpStatus()` discarded
 * an already-resolved token, which then rendered as "No bearer token has
 * been generated" beside a failure banner — a fact asserted from a value
 * that was never actually absent, only unlearned.
 *
 * **A boolean cannot carry three states, so neither fact is one.** `token`
 * and `running` are each `"loading" | "error" | { known value }` unions, not
 * a `string | null` / `boolean` that collapses "haven't read it yet" and
 * "the read failed" onto the same falsy value as "definitely absent" /
 * "definitely not running". Only the `ready` case renders a claim about the
 * fact; `loading` renders nothing yet, and `error` renders a failure banner
 * and otherwise says nothing about what the fact actually is — the same
 * three-state shape the contexts store uses for "which cluster is in focus"
 * (still loading / failed / actually none), for the same reason.
 *
 * Reveal, copy, rotate and revoke all act on the persisted token itself and
 * work whether or not the HTTP server is currently listening — so those
 * controls are gated on the token being known and present, not on `running`.
 * A token that's present while the server reads not-running still shows its
 * row, plus one sentence saying the server isn't listening right now.
 *
 * **A late status response cannot un-say what `revoke()` just established.**
 * `revoke()` sets `statusRead` to `{ ready, false }` directly, because
 * revoking a token is guaranteed to stop the server (`mcp.rs`) — that is not
 * a guess. But the mount effect's own `mcpHttpStatus()` call can still be in
 * flight when that happens (the Revoke button only waits on the TOKEN read,
 * not the status read), and if it resolves afterwards it would otherwise
 * overwrite the accurate post-revoke value with a stale one. `statusSeq`
 * guards exactly that: every write that establishes the status — the fetch
 * starting, and `revoke()`'s direct set — bumps it, and a fetch only applies
 * its result if nothing has superseded it since it started. One source, one
 * sequence — this pane has one status read to protect, not the three-guard
 * shape a listing-plus-per-item read needs.
 *
 * **`rotate()` does not need the same guard.** It sets `tokenRead` directly
 * too, but the Rotate button only exists once `tokenRead.kind === "ready"`
 * — which means the mount effect's `getMcpToken()` call has already
 * resolved by the time a click is even possible. There is no in-flight
 * initial read left to race against, and `busy` already serializes rotate
 * and revoke against each other, so no second guard is load-bearing here.
 *
 * **`getMcpTokenStorage()` is deliberately NOT read here**, despite being
 * named for this file — but the reason first written down was wrong. It said
 * the MCP bearer "is a plain `FileTokenStore` (`crates/mcp/src/auth.rs`),
 * unrelated to the vault". In the desktop it is not: `main.rs:184` and
 * `lib.rs:405-406` register a `VaultTokenStore`, so the bearer is one of the
 * two secrets the vault actually seals (`Secrets.mcp_token`,
 * `apps/desktop/src-tauri/src/vault.rs`).
 *
 * The refusal stands, for the opposite reason. `mcp_token_storage` (`mcp.rs`)
 * returns `vault.key_source()` — where the vault's MASTER KEY lives (keychain /
 * file / locked / biometric), which is a fact about the vault and not about
 * this credential. Captioning a `Bearer token` panel with it would answer a
 * question the reader did not ask with a value that looks like an answer to the
 * one they did. Where the master key lives belongs on the Security pane, which
 * is where classic put it too: `McpSettingsSection` never called this, only
 * `SecuritySettingsSection` did.
 *
 * **`Clients` is not drawn** (#369): `mcpClientConfig` generates configuration
 * *for* a client to paste elsewhere; srelens does not track who connects.
 */

const ADDRESS = "127.0.0.1:8765";

/**
 * How many bullets the mask shows. FIXED, independent of the real token's
 * length — a mask that grew with the secret would tell a reader (or a
 * screenshot) how long it is, which is exactly the property this pane must
 * not leak.
 *
 * **No invented prefix.** An earlier version prepended a literal `srl_` label
 * to the mask ("`srl_••••••••••••`"), but the real backend mints a bare
 * 64-character hex string with no prefix at all (`Token::generate`,
 * `crates/mcp/src/auth.rs`) — Reveal showed a value that didn't start with
 * what the mask claimed it did. Masked and revealed now agree on format:
 * both are just characters, neither carries a label the other doesn't.
 */
const MASK_BULLETS = 16;

function maskedToken(): string {
  return "•".repeat(MASK_BULLETS);
}

/** Whether a fact this pane reads once at mount is still loading, failed, or
 * landed. Kept generic so `token` and `running` don't each reinvent it, and
 * so neither can collapse "unknown" onto the same value as "known absent". */
type Read<T> = { kind: "loading" } | { kind: "error"; error: unknown } | { kind: "ready"; value: T };

const LOADING: Read<never> = { kind: "loading" };

export function McpServer() {
  const [tokenRead, setTokenRead] = useState<Read<string | null>>(LOADING);
  const [statusRead, setStatusRead] = useState<Read<boolean>>(LOADING);
  const [revealed, setRevealed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [actionError, setActionError] = useState<unknown>(null);

  /** Bumped by anything that authoritatively establishes `statusRead` — the
   * fetch below starting, and `revoke()`'s direct set. A fetch's result is
   * only applied if this still reads the value it captured when it started,
   * so a response that belongs to a superseded read is discarded rather than
   * applied over a newer, already-correct state. */
  const statusSeq = useRef(0);

  // Two independent effects, not one `Promise.all` — a status failure must
  // not cost an already-resolved token, and a token failure must not cost an
  // already-resolved status. See the file-level comment.
  useEffect(() => {
    let cancelled = false;
    getMcpToken()
      .then((t) => {
        if (!cancelled) setTokenRead({ kind: "ready", value: t });
      })
      .catch((e) => {
        if (!cancelled) setTokenRead({ kind: "error", error: e });
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    const seq = ++statusSeq.current;
    mcpHttpStatus()
      .then((url) => {
        if (cancelled || statusSeq.current !== seq) return;
        setStatusRead({ kind: "ready", value: url !== null });
      })
      .catch((e) => {
        if (cancelled || statusSeq.current !== seq) return;
        setStatusRead({ kind: "error", error: e });
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const token = tokenRead.kind === "ready" ? tokenRead.value : null;
  const hasToken = tokenRead.kind === "ready" && tokenRead.value !== null;
  const running = statusRead.kind === "ready" && statusRead.value;

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
    setActionError(null);
    try {
      const next = await rotateMcpToken();
      setTokenRead({ kind: "ready", value: next });
      setRevealed(false);
      // Rotating never changes whether the server is listening — it restarts
      // it in place if it was already running, and leaves it stopped
      // otherwise (`mcp.rs`) — so `statusRead` is left untouched here.
    } catch (e) {
      setActionError(e);
    } finally {
      setBusy(false);
    }
  }

  async function revoke() {
    setBusy(true);
    setActionError(null);
    try {
      await revokeMcpToken();
      setTokenRead({ kind: "ready", value: null });
      setRevealed(false);
      // Revoking always stops the server too — "it must never serve
      // unauthenticated" (`mcp.rs`) — so this is a known fact, not a guess.
      // Bump the sequence FIRST: a still-in-flight initial status fetch must
      // be told it's been superseded before this authoritative value lands,
      // or its late response can overwrite it right back.
      statusSeq.current += 1;
      setStatusRead({ kind: "ready", value: false });
    } catch (e) {
      setActionError(e);
    } finally {
      setBusy(false);
    }
  }

  return (
    <Panel
      title={
        <span className="flex flex-wrap items-center gap-2">
          <span>MCP server · loopback http · {ADDRESS}</span>
          {statusRead.kind === "ready" && (
            <Badge tone={running ? "ok" : "muted"}>{running ? "running" : "not running"}</Badge>
          )}
        </span>
      }
    >
      {tokenRead.kind === "loading" ? (
        <p className="text-[0.75rem] text-muted">Checking the MCP server…</p>
      ) : tokenRead.kind === "ready" && hasToken ? (
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
          {statusRead.kind === "ready" && !running && (
            <p className="mt-2 text-[0.75rem] leading-relaxed text-muted">
              The server is not currently listening on {ADDRESS}, so no client can reach it right now.
            </p>
          )}
        </>
      ) : tokenRead.kind === "ready" ? (
        <p data-testid="no-token-note" className="text-[0.75rem] leading-relaxed text-muted">
          No bearer token has been minted yet, so there is nothing to reveal, copy or rotate. One is
          minted when the loopback HTTP server starts.
        </p>
      ) : null}

      {tokenRead.kind === "error" && (
        <FailureAlert tone="sev" title="The MCP server's token could not be read" error={tokenRead.error} />
      )}
      {statusRead.kind === "error" && (
        <FailureAlert
          tone="sev"
          title="Whether the MCP server is running could not be checked"
          error={statusRead.error}
        />
      )}
      {actionError !== null && (
        <FailureAlert tone="sev" title="The MCP server's token could not be updated" error={actionError} />
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
