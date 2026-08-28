import { useEffect, useRef, useState } from "react";
import {
  getMcpToken,
  loadMcpSettings,
  mcpHttpStatus,
  notify,
  revokeMcpToken,
  rotateMcpToken,
  saveMcpSettings,
  startMcpHttp,
  stopMcpHttp,
} from "@srelens/core";
import { Badge, Button, ConfirmDialog, Panel, SubHead } from "@srelens/ui-kit";
import { FailureAlert } from "../../lib/errorCopy";
import { useMcpAutoStart } from "../../lib/mcpAutoStart";

/**
 * §23's `MCP server` pane: the loopback HTTP transport itself — start it, stop
 * it, and manage the bearer token clients authenticate with.
 *
 * **The start and stop are the whole reason this file changed.** It used to
 * read status and manage a token and nothing else: `startMcpHttp` and
 * `stopMcpHttp` appeared nowhere in this package, and their only callers were
 * classic's — `apps/desktop/src/App.tsx`'s auto-start effect and
 * `McpSettingsSection`. `main.tsx` mounts `App` or `NextApp` and never both, so
 * a reader in the new design could not bring the server up at all, and this
 * pane's empty-token note pointed them at "when the loopback HTTP server
 * starts" as though that were something they could make happen. The behaviour
 * is PORTED from `McpSettingsSection`, not reinvented: the same
 * `startMcpHttp(port)` / `stopMcpHttp()` pair, the same `McpSettings` record,
 * the same token re-read after a start (`mcp_http_start` mints one when none
 * exists), and the same revert-to-disabled when a start is refused.
 *
 * **And it does come back up on its own now** (#374 item 2). For one round it did
 * not: the auto-start effect was still classic's (`App.tsx:763-775`, gated on
 * its vault gate reporting ready), moving it belonged with that tree's launch
 * work rather than with a settings pane, and so this pane had to say once that a
 * start lasted the session. `Window` reads the persisted `enabled` back itself
 * now, once `LockGate` reports the vault usable — see the effect there for why
 * that ordering, and not a mount effect, is what the vault requires — so the
 * sentence is gone. It was half of the same defect rather than a caveat about
 * it: `start()` has always persisted `enabled: true`, and nothing read it.
 *
 * The PREFERENCE was always persisted, and still is for the second reason as
 * well: `McpSettings` is one record shared with classic, and a reader who starts
 * the server here and switches designs should not find the toggle over there
 * disagreeing with the server they are talking to.
 *
 * **The address is read, not written.** `mcpHttpStatus()` returns the running
 * server's URL and this pane used to discard it into a boolean while printing a
 * hardcoded `127.0.0.1:8765` in its head — so a reader who had set a
 * non-default port in classic, and whose server was bound to it, was shown an
 * endpoint with nothing on it. The live URL is kept and rendered now. For a
 * STOPPED server there is no live URL to read, so the address is composed from
 * the two facts that actually determine it: `start_server`
 * (`apps/desktop/src-tauri/src/mcp.rs`) binds `Ipv4Addr::LOCALHOST`
 * unconditionally and `url_for` renders `http://{addr}/mcp`, and the port is
 * the persisted one — the very value the Start button passes to
 * `startMcpHttp`. It is labelled as what Start would bind, not as a listener.
 * One element carries that claim, so there is one place to keep true.
 *
 * **`running` is `mcpHttpStatus()` — a live read of the process, not a proxy.**
 * A token existing is not the same fact as a listener being bound: rotate can
 * mint a fresh token while the server stays stopped ("rotating a token must
 * never switch the server on", `mcp.rs`), so a token surviving from an earlier
 * session says nothing about whether anything is listening right now.
 * `mcpHttpStatus` (`packages/core/src/lib/mcp.ts`) reads `McpHttpManager`'s own
 * `running` state (`mcp.rs`) directly — the one place that fact actually lives.
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
 * That discipline is why the start/stop control is drawn only once the status
 * is `ready`: the control has to NAME which of the two it is, and a status
 * that is still loading or failed has established neither. A button labelled
 * `Start server` beside an unread status would be the same claim-from-nothing
 * the three-state union exists to prevent; the failure banner says the check
 * did not answer, which is all this pane knows.
 *
 * Reveal, copy, rotate and revoke all act on the persisted token itself and
 * work whether or not the HTTP server is currently listening — so those
 * controls are gated on the token being known and present, not on `running`.
 *
 * **A late status response cannot un-say what an action just established.**
 * `revoke()` sets `statusRead` directly, because revoking a token is
 * guaranteed to stop the server (`mcp.rs`) — that is not a guess; so do the
 * start and the stop, which return the fact themselves. But the mount effect's
 * own `mcpHttpStatus()` call can still be in flight when any of them happens,
 * and if it resolves afterwards it would otherwise overwrite the accurate
 * value with a stale one. `statusSeq` guards exactly that: every write that
 * establishes the status bumps it, and a fetch only applies its result if
 * nothing has superseded it since it started.
 *
 * **`rotate()` does not need the same guard.** It sets `tokenRead` directly
 * too, but the Rotate button only exists once `tokenRead.kind === "ready"`
 * — which means the mount effect's `getMcpToken()` call has already
 * resolved by the time a click is even possible. There is no in-flight
 * initial read left to race against, and `busy` already serializes every
 * action on this pane against every other, so no second guard is load-bearing.
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
 * **Rotate and Revoke ask first**, because both invalidate the credential on
 * the click and neither is undoable. Rotation restarts a running server so the
 * new token takes effect, which drops every agent request in flight;
 * revocation stops the server outright — "it must never serve
 * unauthenticated" (`mcp.rs`) — and disconnects every HTTP client. Classic has
 * gated both behind `ConfirmDialog` since it shipped
 * (`McpSettingsSection.tsx:314-329`) and this pane called straight through, so
 * a mis-aimed click was an immediate outage. The kit's `ConfirmDialog` is
 * scoped to the tab it was opened from (#365), which is right here: the
 * question belongs to the Settings tab it was asked in, not to the window.
 *
 * The COPY is classic's, kept where it is still true and sharpened where this
 * pane knows more. Classic had to hedge — "If the MCP HTTP server is running,
 * rotating restarts it immediately" — because its dialog had no live status to
 * read. This pane does: {@link rotateQuestion} and {@link revokeQuestion} say
 * what will happen when the status read has answered, and fall back to
 * classic's conditional sentence, verbatim, for the one case where the read is
 * still loading or refused. A question is the last place to assert a fact from
 * nothing.
 *
 * **No port editor** (#374). Changing the port is a second, differently-shaped
 * job — validate, persist, and restart a listener that may be serving an agent
 * mid-call — and this pane has no control for it, so it makes no claim that it
 * has one. The persisted value is read and honoured; classic is where it is
 * still set.
 *
 * **`Clients` is not drawn** (#369): `mcpClientConfig` generates configuration
 * *for* a client to paste elsewhere; srelens does not track who connects.
 */

/**
 * Where a STOPPED server would bind, from the two facts that determine it:
 * `start_server` binds `Ipv4Addr::LOCALHOST` unconditionally and `url_for`
 * renders `http://{addr}/mcp` (`apps/desktop/src-tauri/src/mcp.rs`), so the
 * port is the only variable — and `port` here is the same value the Start
 * button hands `startMcpHttp`. A RUNNING server's address is never built this
 * way; that one comes back from `mcpHttpStatus()` itself.
 */
function wouldBindAt(port: number): string {
  return `http://127.0.0.1:${port}/mcp`;
}

/**
 * What rotating costs, from what this pane has actually established about the
 * listener — `true` running, `false` stopped, `null` not established.
 *
 * The `null` sentence is classic's, verbatim
 * (`apps/desktop/src/components/McpSettingsSection.tsx`), because classic's
 * dialog never had a live status and its hedge is exactly right for a pane
 * that has not read one yet. The other two say what will happen instead of
 * what might, which is the whole advantage this pane has over that one.
 */
function rotateQuestion(running: boolean | null): string {
  if (running === true) {
    return "The server is running, so rotating restarts it immediately for the new token to take effect — every agent request in flight is dropped. Connected clients need the new value or they stop working.";
  }
  if (running === false) {
    return "The server is not running, so nothing is dropped and nothing disconnects. The old value stops working the moment it is replaced: a client still holding it needs the new value before it can connect again.";
  }
  return "If the MCP HTTP server is running, rotating restarts it immediately so the new token takes effect — any in-flight agent request is dropped. Connected clients need the new value or they will stop working.";
}

/**
 * What revoking costs, the same three ways. Revoking always stops the server —
 * it must never serve unauthenticated (`mcp.rs`) — so the difference between
 * the first two is who is disconnected by it, and what is left afterwards.
 */
function revokeQuestion(running: boolean | null): string {
  if (running === true) {
    return "The server is running, and revoking stops it — it never serves without a valid token — so every client connected over HTTP disconnects and any request in flight is dropped. Starting the server again mints a new token.";
  }
  if (running === false) {
    return "The server is not running, so nothing disconnects. Revoking leaves srelens with no bearer token at all: nothing can connect over HTTP until a start mints a new one.";
  }
  return "Revoking also stops the MCP HTTP server: it never serves without a valid token. Any clients connected over HTTP will disconnect.";
}

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
  /** The running server's own URL, or `null` for "not running" — the value
   *  `mcpHttpStatus()` actually returns, kept rather than reduced to a flag. */
  const [statusRead, setStatusRead] = useState<Read<string | null>>(LOADING);
  const [revealed, setRevealed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [actionError, setActionError] = useState<unknown>(null);
  /** Which of the two destructive token actions has been asked about, if any. */
  const [asking, setAsking] = useState<null | "rotate" | "revoke">(null);
  const [serverError, setServerError] = useState<{ verb: "started" | "stopped"; error: unknown } | null>(
    null,
  );

  /** Read ONCE, at mount: the port a start uses and a stopped server's address
   *  is composed from. Not re-read per render — nothing in this tree writes it,
   *  and a value that changed between the address on screen and the number
   *  handed to `startMcpHttp` would be two different claims. */
  const [port] = useState(() => loadMcpSettings().port);

  /** Bumped by anything that authoritatively establishes `statusRead` — the
   * fetch below starting, and the direct sets in `start()`, `stop()` and
   * `revoke()`. A fetch's result is only applied if this still reads the value
   * it captured when it started, so a response that belongs to a superseded
   * read is discarded rather than applied over a newer, already-correct
   * state. */
  const statusSeq = useRef(0);

  /**
   * The shell's auto-start, as a value that CHANGES when it settles — nothing
   * more. `Window` brings an enabled server up the moment `LockGate` reports the
   * vault usable, and a saved Settings tab is restored at that same moment: the
   * status effect below can therefore answer `null` while the bind is still in
   * flight (`mcp_http_start` binds the listener before `McpHttpManager` records
   * anything as running), and until this existed nothing told this pane
   * afterwards. It sat permanently on `not running`, offering a Start that
   * restarts a live server and drops every agent request in flight.
   *
   * A signal and not a status, deliberately. What the effect does with it is
   * take its OWN `mcpHttpStatus()` read again — see the file comment on why
   * `running` here is a live read of the process rather than a stored flag, and
   * `lib/mcpAutoStart.ts` for why a URL published from the shell would be a
   * second source of truth for it that could lie in the other direction.
   */
  const shellStart = useMcpAutoStart();

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

  // Re-taken when the shell says its auto-start has settled, and that is the
  // only thing `shellStart` is for — the value itself means nothing. See the
  // note above `useMcpAutoStart` for the race: this pane's read can answer
  // `null` with the bind still in flight, and until the shell said so nothing
  // told it afterwards. Still THIS pane's own live read of the process; the
  // signal carries no claim about what is listening, because `running` here is
  // not a stored flag and must not become a proxy that can lie the other way.
  useEffect(() => {
    let cancelled = false;
    const seq = ++statusSeq.current;
    mcpHttpStatus()
      .then((url) => {
        if (cancelled || statusSeq.current !== seq) return;
        setStatusRead({ kind: "ready", value: url });
      })
      .catch((e) => {
        if (cancelled || statusSeq.current !== seq) return;
        setStatusRead({ kind: "error", error: e });
      });
    return () => {
      cancelled = true;
    };
  }, [shellStart]);

  const token = tokenRead.kind === "ready" ? tokenRead.value : null;
  const hasToken = tokenRead.kind === "ready" && tokenRead.value !== null;
  const running = statusRead.kind === "ready" && statusRead.value !== null;
  /** Whether a listener is up, as the THREE states the status read actually
   *  has: `null` is "not established", which is not the same claim as "not
   *  running" and is what the confirmations are worded from. */
  const listening: boolean | null = statusRead.kind === "ready" ? statusRead.value !== null : null;
  /** The one address claim this pane makes: the live URL when there is one, and
   *  otherwise what Start would bind. */
  const address = statusRead.kind === "ready" ? (statusRead.value ?? wouldBindAt(port)) : null;

  /** Establish the status from a value an action itself returned, superseding
   *  any read still in flight. Bumps the sequence FIRST: an in-flight fetch
   *  must be told it has been superseded before the authoritative value lands,
   *  or its late response can overwrite it right back. */
  function establishStatus(url: string | null) {
    statusSeq.current += 1;
    setStatusRead({ kind: "ready", value: url });
  }

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

  async function start() {
    setBusy(true);
    setServerError(null);
    try {
      establishStatus(await startMcpHttp(port));
      saveMcpSettings({ enabled: true, port });
      // The first start is also the first place a token can exist:
      // `mcp_http_start` mints one when none is stored (`mcp.rs`), so a
      // previously-read `null` is now stale. Re-read rather than assume —
      // this pane never invents a secret's value.
      try {
        setTokenRead({ kind: "ready", value: await getMcpToken() });
      } catch (e) {
        setTokenRead({ kind: "error", error: e });
      }
    } catch (e) {
      setServerError({ verb: "started", error: e });
      // Nothing to re-establish: `Start server` is only offered while the
      // status is a KNOWN not-running, and a refused start leaves it that way
      // — `start_server` binds the listener before it records anything as
      // running (`mcp.rs`), so nothing is bound when the bind is what failed.
      // A write here would be a no-op dressed as a fact; the mutation pass
      // caught it as one, since no reachable state could tell it apart.
      // The PREFERENCE does revert, as classic's does: a start that failed
      // must not leave `enabled` set for the next launch to retry blindly.
      saveMcpSettings({ enabled: false, port });
    } finally {
      setBusy(false);
    }
  }

  async function stop() {
    setBusy(true);
    setServerError(null);
    try {
      await stopMcpHttp();
      establishStatus(null);
      saveMcpSettings({ enabled: false, port });
    } catch (e) {
      setServerError({ verb: "stopped", error: e });
      // Deliberately NOT establishing a status here. A stop that rejected
      // says nothing about what is listening now, and the previous value is
      // the last thing that was actually read.
    } finally {
      setBusy(false);
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
      // Closed on a refusal as well as on success: the failure is reported by
      // this pane's own alert, and an alert behind a dialog that is still
      // asking the question it answered is two states at once.
      setAsking(null);
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
      establishStatus(null);
      // And the preference follows the server, exactly as classic's revoke
      // does: leaving `enabled` set would have the next launch (or a switch
      // to the other design) start a server the reader just took down.
      saveMcpSettings({ enabled: false, port });
    } catch (e) {
      setActionError(e);
    } finally {
      setBusy(false);
      setAsking(null);
    }
  }

  return (
    <Panel
      title={
        <span className="flex flex-wrap items-center gap-2">
          <span>MCP server · loopback http</span>
          {statusRead.kind === "ready" && (
            <Badge tone={running ? "ok" : "muted"}>{running ? "running" : "not running"}</Badge>
          )}
        </span>
      }
    >
      {/* The control and the address, together — one row, because the address
          is the thing the control acts on. Drawn only once the status is
          `ready`: see the file comment on why a Start/Stop label is a claim. */}
      {statusRead.kind === "ready" && address !== null && (
        <>
          <div className="flex flex-wrap items-center gap-2">
            <Button
              variant={running ? "secondary" : "primary"}
              size="sm"
              disabled={busy}
              onClick={() => void (running ? stop() : start())}
            >
              {running ? "Stop server" : "Start server"}
            </Button>
            {/* `min-w-0` and `break-all` for the same reason the token row has
                them: a URL beside a control is the shape that has bitten this
                migration under `min-width: auto`. */}
            <span
              data-testid="mcp-address"
              className="min-w-0 text-[0.75rem] leading-relaxed text-muted"
            >
              {running ? "Listening at " : "Not listening. Start binds "}
              <code className="code break-all rounded px-1.5 py-0.5 text-[0.6875rem]">{address}</code>
            </span>
          </div>
        </>
      )}

      {serverError !== null && (
        <FailureAlert
          tone="sev"
          title={`The MCP server could not be ${serverError.verb}`}
          error={serverError.error}
        />
      )}

      {tokenRead.kind === "loading" ? (
        <p className="mt-3 text-[0.75rem] text-muted">Checking the MCP server…</p>
      ) : tokenRead.kind === "ready" && hasToken ? (
        <>
          <SubHead className="mt-4">Bearer token</SubHead>
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
            <Button variant="secondary" size="sm" disabled={busy} onClick={() => setAsking("rotate")}>
              Rotate
            </Button>
            <Button variant="danger" size="sm" disabled={busy} onClick={() => setAsking("revoke")}>
              Revoke
            </Button>
          </div>
          <p className="mt-2 text-[0.75rem] leading-relaxed text-muted">
            Rotating restarts the server, drops in-flight requests, and invalidates clients still using the old
            token.
          </p>
        </>
      ) : tokenRead.kind === "ready" ? (
        <p data-testid="no-token-note" className="mt-3 text-[0.75rem] leading-relaxed text-muted">
          No bearer token has been minted yet, so there is nothing to reveal, copy or rotate.
          Starting the server mints one.
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

      {asking !== null && (
        <ConfirmDialog
          title={asking === "rotate" ? "Rotate the bearer token?" : "Revoke the bearer token?"}
          message={asking === "rotate" ? rotateQuestion(listening) : revokeQuestion(listening)}
          confirmLabel={asking === "rotate" ? "Rotate" : "Revoke"}
          // `danger` on the one that leaves no credential at all. Rotating is
          // disruptive and reversible-by-another-rotation; revoking takes the
          // server down with it and leaves nothing to connect with — the same
          // split classic draws.
          danger={asking === "revoke"}
          // Blocks every dismissal path while the call is in flight, so a
          // half-done rotation cannot be walked away from.
          busy={busy}
          onConfirm={() => void (asking === "rotate" ? rotate() : revoke())}
          onCancel={() => setAsking(null)}
        />
      )}
    </Panel>
  );
}
