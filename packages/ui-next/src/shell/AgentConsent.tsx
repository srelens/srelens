import { useEffect, useState } from "react";
import {
  describeError,
  isTauri,
  notify,
  on,
  respondToConfirm,
  type ConfirmRequest,
} from "@srelens/core";
import { ConfirmDialog } from "@srelens/ui-kit";
import { useWorkspaceSealed } from "./LockGate";

/**
 * The consent prompt for an MCP tool call: this design's port of classic's
 * `McpConfirmDialog` (`apps/desktop/src/components/McpConfirmDialog.tsx`).
 *
 * **Why it had to be ported, and why now.** Every mutating capability srelens
 * registers is confirm-gated by a build-time invariant
 * (`assert_mutating_capabilities_are_gated`, `crates/mcp/src/completeness.rs`),
 * and so is the one sensitive read. The gate blocks the call in Rust, raises
 * and focuses the window, emits `mcp://confirm-request` and waits sixty seconds
 * (`apps/desktop/src-tauri/src/mcp_confirm.rs`); a request nobody answers is
 * DENIED. Classic's modal was the only listener, and `main.tsx` mounts that
 * tree or this one and never both — so in the new design every agent mutation
 * and every Secret read hung for a minute and was then refused, with nothing on
 * screen. That was filed as #374 item 1 and left alone while this design could not
 * start the MCP server at all; the pane's Start button changed that, so the
 * issue this branch made reachable is closed here.
 *
 * **A port, not a redesign.** The queue, the resolution listener, the failure
 * report and the two labels are classic's. Requests QUEUE rather than replace
 * one another: two agents can call concurrently, and dropping one would leave
 * that call hanging until its own timeout denied it. A resolution announced by
 * the backend — answered here, answered on classic's inline assistant card, or
 * timed out — drops the request however it settled, so this never lingers over
 * an already-decided call. Removal is BY ID and not by position, because the
 * `mcp://confirm-resolved` event may already have taken the entry out and
 * slicing blindly would then drop the NEXT request instead.
 *
 * Two things differ, both because of a package boundary rather than a
 * judgement:
 *
 * - The bus is core's `on` rather than `listen` from `@tauri-apps/api/event`.
 *   This package depends on `@srelens/core` and `@srelens/ui-kit` and nothing
 *   else, and `on` is the abstraction every other backend event in srelens goes
 *   through. It hands the payload as `unknown`, so {@link asRequest} narrows it
 *   instead of casting — a malformed payload is ignored rather than rendered as
 *   a question with `undefined` in it.
 * - The failure detail goes through `describeError`, as everything in this
 *   package does, rather than `String(e)`.
 *
 * **This is the WINDOW's question, so it is mounted outside every tab.** Since
 * PR #365 the kit's dialogs are scoped to the portal surface they are opened
 * in: a dialog mounted inside a tab covers that tab, marks that tab inert, and
 * leaves the rest of the window live. That is right for a question a tab asked
 * and exactly wrong for this one — the reader could switch tabs away from a
 * call the backend is blocking on, and the request would then be waiting behind
 * a tab. `Window` mounts this at the level `Chrome` and `Status` sit at, which
 * is outside `TabSurface`'s `PortalScopeProvider` — so `useOpenLayer` reports
 * no scope, and the kit's `ConfirmDialog` sets itself up as the document-wide
 * modal it was before #365: Radix's own overlay, its focus trap, `aria-modal`,
 * and Escape routed to it. Nothing here asks for that; it is what the kit
 * already does with no surface around it, which is why the mount point is the
 * whole of the decision.
 *
 * **It does not ask while the window is covered.** `useWorkspaceSealed` is
 * true while the vault is sealed AND for the launch check that has not answered
 * yet, and in both states this refuses the call instead of drawing a prompt.
 * Two reasons, in order:
 *
 * 1. A prompt over a locked window is a live control on a sealed session. The
 *    backend raises and focuses the window before it emits, so whoever is at
 *    the keyboard is who would be answering — not necessarily the reader who
 *    sealed it. §25's own argument for replacing the band rather than marking
 *    the vault closed is that a window which merely LOOKS sealed while its
 *    controls still act is worse than no lock; an Approve button is the
 *    strongest such control in the app.
 * 2. The gate fails closed either way, so refusing costs nothing that was not
 *    already lost. What it saves is a minute: the backend would deny on
 *    timeout regardless, and an agent told "no" now is better off than one
 *    waiting sixty seconds for the same answer.
 *
 * It is said out loud rather than left to that timeout, so the call does not
 * sit in a queue whose answer is already determined — and so an unlock at
 * second fifty-nine cannot raise a prompt whose approval could no longer land.
 * The refusal is the COVER's and not this component's: once it is down, the
 * next request is put to the reader exactly as before.
 *
 * The backend words its refusal `user declined` (`mcp_confirm.rs`), which is
 * the same sentence the Deny button produces. That is one degree less precise
 * than "the window was locked", and worth naming rather than hiding: the
 * decision is a refusal on the reader's behalf, and it is reported to the agent
 * as one.
 */

/**
 * The payload the backend emits, narrowed rather than cast.
 *
 * `on` types a payload as `unknown` — correctly, it crosses a process boundary
 * — so this is the one place that decides a message is a request. A shape that
 * does not match is ignored: there is no id to answer with, and drawing a card
 * headed `undefined` over a call that will time out anyway tells the reader
 * nothing they can act on.
 */
function asRequest(payload: unknown): ConfirmRequest | null {
  if (typeof payload !== "object" || payload === null) return null;
  const { id, tool, args } = payload as Partial<ConfirmRequest>;
  if (typeof id !== "string" || id === "" || typeof tool !== "string") return null;
  return { id, tool, args: typeof args === "object" && args !== null ? args : {} };
}

/** The id out of a `mcp://confirm-resolved` payload, or null. */
function resolvedId(payload: unknown): string | null {
  if (typeof payload !== "object" || payload === null) return null;
  const { id } = payload as { id?: unknown };
  return typeof id === "string" && id !== "" ? id : null;
}

export function AgentConsent() {
  const [queue, setQueue] = useState<ConfirmRequest[]>([]);
  const [busy, setBusy] = useState(false);
  // The cover, by either route — a raised lock or a launch check that has not
  // answered. See the file comment for what this component does about it.
  const covered = useWorkspaceSealed();
  const current = queue[0];

  useEffect(() => {
    // Every vault command and `mcp_confirm_respond` alike are Tauri commands,
    // and nothing in a browser emits this event — subscribing there would open
    // a channel for traffic that cannot arrive.
    if (!isTauri()) return;
    const offRequest = on("mcp://confirm-request", (payload) => {
      const request = asRequest(payload);
      if (request) setQueue((q) => [...q, request]);
    });
    // The backend announces every resolution, however it settled.
    const offResolved = on("mcp://confirm-resolved", (payload) => {
      const id = resolvedId(payload);
      if (id !== null) setQueue((q) => q.filter((r) => r.id !== id));
    });
    return () => {
      offRequest();
      offResolved();
    };
  }, []);

  // Refuse, rather than ask over a covered window. Everything waiting, not
  // just the head: the whole queue is unaskable for as long as the cover is up,
  // and a request left in it would surface the moment the cover came down, with
  // most of its sixty seconds already spent.
  useEffect(() => {
    if (!covered || queue.length === 0) return;
    const waiting = queue.map((r) => r.id);
    // Emptied before the answers are sent, not after: the state write is what
    // takes the prompt off screen, and this effect re-runs on the empty queue
    // and stops.
    setQueue([]);
    for (const id of waiting) {
      // Silent, deliberately. A refusal the reader did not ask for and cannot
      // act on has nothing to tell them, and what rejects here is a call that
      // was already settled — the refusal it would have duplicated has already
      // happened. It must still be caught: an unhandled rejection out of an
      // effect is a crash in a component whose whole job is to keep one from
      // mattering.
      //
      // One case is a genuine race rather than a duplicate, and it is worth
      // naming: the reader presses Approve and seals the window before the
      // approval's own round trip returns. Both answers are then in flight for
      // the same id, `Pending::resolve` (`mcp_confirm.rs`) accepts exactly the
      // first to arrive, and which one that is is not ordered here. Left as a
      // race on purpose — both answers are the reader's own, given a fraction
      // of a second apart, and the alternative (skipping the head while its
      // answer is in flight) would let an approval carry a window that is
      // already sealed, which is the thing this whole branch refuses.
      void respondToConfirm(id, false).catch(() => {});
    }
  }, [covered, queue]);

  async function answer(approved: boolean): Promise<void> {
    if (!current) return;
    const { id } = current;
    setBusy(true);
    try {
      await respondToConfirm(id, approved);
    } catch (e) {
      // The request timed out server-side, or was answered elsewhere — the
      // click did not take effect. Swallowing this would let the reader believe
      // they approved (or denied) a call that had in fact already been settled
      // without them.
      notify.error("Could not respond to that confirmation", describeError(e).detail);
    } finally {
      setBusy(false);
      // By id, not by position — see the file comment.
      setQueue((q) => q.filter((r) => r.id !== id));
    }
  }

  if (covered || !current) return null;

  return (
    <ConfirmDialog
      title="An agent wants to run a cluster action"
      message={
        <div className="flex flex-col gap-2">
          <p className="m-0">
            Tool: <code className="code rounded px-1.5 py-0.5">{current.tool}</code>
          </p>
          <pre className="code max-h-64 overflow-auto rounded p-3 text-[0.6875rem]">
            <code>{JSON.stringify(current.args, null, 2)}</code>
          </pre>
          {queue.length > 1 && (
            <p className="m-0 text-[0.6875rem] text-muted">
              {queue.length - 1} more request{queue.length - 1 === 1 ? "" : "s"} waiting
            </p>
          )}
        </div>
      }
      confirmLabel="Approve"
      cancelLabel="Deny"
      danger
      busy={busy}
      onConfirm={() => void answer(true)}
      onCancel={() => void answer(false)}
    />
  );
}
