import { useEffect, useState } from "react";
import { isTauri, pendingConfirms, respondToConfirm, subscribe, type ConfirmRequest } from "@srelens/core";
import { getAgentRun, noteGate } from "../lib/agentRun";
import { Alert, ConfirmDialog } from "@srelens/ui-kit";
import { FailureLine } from "../lib/errorCopy";
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
 * **A port, not a redesign.** The queue, the resolution listener and the two
 * labels are classic's. Requests QUEUE rather than replace
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
 * - The bus is core's `subscribe` rather than `listen` from
 *   `@tauri-apps/api/event`. This package depends on `@srelens/core` and
 *   `@srelens/ui-kit` and nothing else, and core's bus is the abstraction every
 *   other backend event in srelens goes through. It hands the payload as
 *   `unknown`, so {@link asRequest} narrows it instead of casting — a malformed
 *   payload is ignored rather than rendered as a question with `undefined` in
 *   it. It is `subscribe` and not `on` for a reason the replay paragraph below
 *   gives.
 * - The failure detail goes through `describeError`, as everything in this
 *   package does, rather than `String(e)`.
 *
 * **A failed answer is REPORTED IN THE DIALOG, and the dialog stays up.** The
 * port carried classic's `notify.error` across, and in this tree that reports
 * nothing: `notify`'s sink is installed at `main.tsx` for both designs, but it
 * calls sonner, and sonner's `<Toaster>` is mounted in classic's `App` — which
 * `main.tsx` mounts instead of this tree, never beside it. The toast was
 * created and rendered nowhere, and the `finally` below then dropped the
 * request, so the reader watched their approval or denial vanish and was left
 * believing they had answered a call that in fact nobody answered. That is
 * #374 item 2; the surface here is the same one `index.tsx` and
 * `ResourceMenu`'s confirmation already use for the same reason, and the
 * `notify.error` call is gone rather than kept, so there is one report of this
 * failure and not one visible plus one invisible.
 *
 * Two consequences worth stating, because they are the point:
 *
 * - **The queue entry survives a failed answer.** A rejection means the call
 *   was NOT answered as the reader asked, so removing it would take the
 *   question away exactly as if it had been — and take the retry with it. It
 *   cannot strand them: `ResolveOnDrop` (`mcp_confirm.rs`) broadcasts
 *   `mcp://confirm-resolved` on every exit from `confirm` — answered, timed
 *   out at sixty seconds, or the future dropped — so a request that can no
 *   longer be answered is always taken out of this queue by the backend,
 *   whether or not the reader presses anything again.
 * - **The failure is held BY ID**, for the same reason removal is. That
 *   broadcast can take the head out from under this component at any moment;
 *   a failure kept as a bare string would then be drawn under the NEXT
 *   agent's question, telling the reader a call had been refused that never
 *   was.
 *
 * The copy keeps the distinction the catch block already drew and claims
 * nothing more. srelens knows one thing for certain — the click did not take
 * effect — and says exactly that in its own words; WHY comes from the backend
 * underneath, through `FailureLine`, which is `describeError` plus the folded
 * original. It is not srelens's sentence, and it is not printed as one.
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
 *
 * **A subscriber is handed what is already waiting.** The backend emits
 * `mcp://confirm-request` exactly once and waits sixty seconds; nothing
 * replays it. This component's listener is an effect, and an effect runs only
 * once `main.tsx` has downloaded the new design's two chunks, called
 * `createRoot`, and this tree has committed — so a request raised while any of
 * that was still happening met no listener and was denied on timeout with
 * nothing ever drawn. That gap surfaced three times (the boot spinner, the
 * unmounted gate, the pre-`createRoot` bootstrap), and each fix moved this
 * listener earlier and left an earlier window in front of it. There is no
 * earliest listener; the class is fixed instead. `Pending` (`mcp_confirm.rs`)
 * now holds each request beside its answer channel, `mcp_confirm_pending`
 * returns that set, and the subscribe effect below reads it — AFTER its
 * listeners have LANDED, and that is the whole point. The backend registers a
 * request before it emits it, so with the listener installed first a request
 * is seen in the snapshot, or as an event, or as both, whatever the
 * interleaving; fetch-then-subscribe would reopen the exact gap for a request
 * raised between the two.
 *
 * "Installed" means registered, not requested. Tauri's `listen` is an IPC
 * round trip, and core's `on` starts it and RETURNS while it is still in
 * flight; a fetch issued after `on` was ordered after the call and not after
 * the registration, and a request the backend registered after the snapshot
 * was taken but before that `listen()` resolved was in neither the snapshot
 * nor any delivered event — the same gap, one level down. So the effect uses
 * core's `subscribe`, which resolves only once the registration has landed,
 * and AWAITS each one before asking for the snapshot.
 *
 * The two listeners land in a fixed order, resolution first. Between one
 * landing and the next an event can be lost, and the two events are not equal
 * in what that costs: a request event lost there is recovered by the snapshot,
 * read once both are in; a resolution lost there is recovered by nothing —
 * `mcp://confirm-resolved` is emitted once and is the one thing that takes a
 * prompt down once its answer can no longer land. Request-first would let a
 * call heard live in that gap settle unheard in the same gap, and leave its
 * prompt on screen with nothing to take it down.
 *
 * Awaiting means this can be unmounted mid-registration. The cleanup then has
 * no disposer to call yet, and the ack still arrives, for a component that is
 * gone; the listener it installs is disposed as it lands — the way core's `on`
 * disposes after its own `listen` resolves — and the effect goes no further:
 * no next listener, no snapshot, no state written to an unmounted component.
 *
 * "Or as both" is why the snapshot is MERGED BY ID and not appended: one
 * request has one `oneshot::Sender`, and two queue entries for it would be two
 * prompts and a second answer that rejects. And a resolution heard while the
 * fetch was in flight wins over the snapshot — the set is read on the backend
 * when the command runs and here when the response lands, a call can settle in
 * between, and replaying it would draw a prompt whose answer can no longer land
 * with nothing left to take it down. A replayed request then takes exactly the
 * live path: into the queue, where the cover effect refuses it if the window is
 * sealed and the prompt asks it if not. The sixty seconds are the backend's
 * and untouched; a request replayed after boot has whatever remains of them.
 *
 * **A snapshot that could not be read is said on screen.** Web mode never runs
 * the effect. On desktop a rejection means every request raised before this
 * mounted is lost to its timeout unseen — the very failure the replay is for —
 * and the reader is the only one who could go and look (the agent's
 * transcript, the Audit pane). Swallowing it would be this component's own
 * gap again, one layer down; throwing would take the live listener down with
 * it. So it is an `Alert` the reader can put away, through `describeError`
 * like every failure in this package, and the subscriptions stand.
 */

/**
 * The payload the backend emits, narrowed rather than cast.
 *
 * `subscribe` types a payload as `unknown` — correctly, it crosses a process
 * boundary — so this is the one place that decides a message is a request. A shape that
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

/**
 * A failed answer, and the request it was an answer TO.
 *
 * The id is not decoration: see the file comment. The head can be taken out
 * from under this by `mcp://confirm-resolved` between the rejection and the
 * next render, so the failure is only drawn while the request it belongs to is
 * still the one being asked.
 */
interface FailedAnswer {
  id: string;
  error: unknown;
}

/**
 * The queue with `incoming` merged in BY ID — an entry already present, by
 * event or by an earlier replay, is kept once. See the file comment on why the
 * snapshot and the event stream can carry the same request.
 */
function mergeById(queue: ConfirmRequest[], incoming: ConfirmRequest[]): ConfirmRequest[] {
  const seen = new Set(queue.map((r) => r.id));
  const fresh: ConfirmRequest[] = [];
  for (const r of incoming) {
    if (seen.has(r.id)) continue;
    seen.add(r.id);
    fresh.push(r);
  }
  return fresh.length === 0 ? queue : [...queue, ...fresh];
}

export function AgentConsent() {
  const [queue, setQueue] = useState<ConfirmRequest[]>([]);
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState<FailedAnswer | null>(null);
  /** Why what was waiting at mount could not be read, until the reader puts
   *  it away. See the file comment on why this is said rather than swallowed. */
  const [replayFailed, setReplayFailed] = useState<unknown>(null);
  // The cover, by either route — a raised lock or a launch check that has not
  // answered. See the file comment for what this component does about it.
  const covered = useWorkspaceSealed();
  const current = queue[0];

  useEffect(() => {
    // Every vault command and `mcp_confirm_respond` alike are Tauri commands,
    // and nothing in a browser emits this event — subscribing there would open
    // a channel for traffic that cannot arrive.
    if (!isTauri()) return;
    // Set by the cleanup. Every step below is on the far side of an `await`,
    // so each checks it: a registration that lands for an unmounted component
    // is disposed on the spot, and nothing past it runs.
    let cancelled = false;
    const offs: Array<() => void> = [];
    // Resolutions heard while the snapshot below is in flight. It is read on
    // the backend when the command runs and applied here when the response
    // lands; a call that settled in between is in the snapshot and must not be
    // replayed. Cleared once the snapshot has been applied — after that the
    // queue filter alone is the whole story, as it always was.
    let resolvedMeanwhile: Set<string> | null = new Set();

    /**
     * Register, and resolve only once the registration has LANDED — `subscribe`
     * and not `on`; see the file comment. False if this was unmounted while
     * the registration was in flight, in which case the listener it installed
     * has already been let go.
     */
    async function listen(channel: string, handler: (payload: unknown) => void): Promise<boolean> {
      const off = await subscribe(channel, handler);
      if (cancelled) {
        off();
        return false;
      }
      offs.push(off);
      return true;
    }

    void (async () => {
      // RESOLUTION FIRST — the listener whose loss nothing recovers. See the
      // file comment. The backend announces every resolution, however it
      // settled.
      const hearingResolutions = await listen("mcp://confirm-resolved", (payload) => {
        const id = resolvedId(payload);
        if (id === null) return;
        resolvedMeanwhile?.add(id);
        setQueue((q) => q.filter((r) => r.id !== id));
        // Taking it off screen is not the whole job: a gate this run already
        // owns is still drawn in the transcript, and `pending` there is a claim
        // that the request is the reader's to answer. Once the backend says it
        // stopped waiting, that claim is false — however it settled.
        //
        // Only a gate still `pending` is touched. The backend announces EVERY
        // resolution, including the reader's own answer, so clobbering here
        // would overwrite an `approved` with a vaguer word. Whichever of the
        // two lands second, the decided outcome is the one that survives.
        const owned = getAgentRun().gates.find((g) => g.id === id);
        if (owned && owned.outcome === "pending") {
          noteGate({ ...owned, outcome: "settled", at: Date.now() });
        }
      });
      if (!hearingResolutions) return;
      const hearingRequests = await listen("mcp://confirm-request", (payload) => {
        const request = asRequest(payload);
        if (request) setQueue((q) => [...q, request]);
      });
      if (!hearingRequests) return;
      // AFTER both have landed, and that is the whole point — see the file
      // comment. What was raised before this mounted is handed over here.
      let waiting: ConfirmRequest[];
      try {
        waiting = await pendingConfirms();
      } catch (e) {
        if (!cancelled) setReplayFailed(e);
        return;
      }
      if (cancelled) return;
      const settled = resolvedMeanwhile ?? new Set<string>();
      resolvedMeanwhile = null;
      const requests = waiting
        .map(asRequest)
        .filter((r): r is ConfirmRequest => r !== null && !settled.has(r.id));
      if (requests.length > 0) setQueue((q) => mergeById(q, requests));
    })();
    // A registration that REJECTS is left to reject, as it was under `on`
    // (whose own `listen()` rejection had no handler either): a bus that
    // cannot install a listener is not something this component can report
    // in a way the reader could act on, and hiding it would be worse.

    return () => {
      cancelled = true;
      // What has landed is let go now; what is still in flight is let go by
      // `listen` as it lands.
      for (const off of offs) off();
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

  // The transcript's record of a gate begins when the reader is actually SHOWN
  // it — `!covered && current` is exactly the condition under which this
  // component renders the prompt, so the record and the question appear
  // together.
  //
  // Not on arrival. The refusals this component makes on its own — a covered
  // window, a request the backend settled elsewhere, a snapshot entry it cannot
  // answer — are not decisions, and a record of one would draw a decision in
  // the transcript that nobody was ever asked to make. The reader would read
  // their own name on a call they never saw.
  //
  // **Ownership is decided ONCE, here, at presentation.** `ConfirmRequest` is
  // `{ id, tool, args }` — it carries no client identity, so this component
  // cannot know whose call raised it. The confirm channel is app-wide by
  // design: an external MCP client (the loopback HTTP server, bearer-token
  // authenticated) raises the exact same `mcp://confirm-request` srelens's own
  // agent does. The honest predicate is "does THIS store have a turn actually
  // in flight right now" — that is the only moment srelens's own agent could
  // be the caller. A confirm presented while the store is idle is recorded as
  // nothing: it is still shown and still answered below, just not attributed
  // to a conversation it may have no part in.
  //
  // Known limit, stated rather than hidden: a confirm raised by ANOTHER
  // client WHILE srelens's own agent happens to be mid-turn is still
  // misattributed — this predicate cannot tell the two apart without client
  // identity in the payload, which `ConfirmRequest` does not carry. Fixing
  // that needs a payload change on the backend side; filed separately.
  useEffect(() => {
    if (covered || !current) return;
    if (!getAgentRun().busy) return;
    noteGate({ id: current.id, tool: current.tool, args: current.args, outcome: "pending" });
  }, [covered, current]);

  async function answer(approved: boolean): Promise<void> {
    if (!current) return;
    const { id } = current;
    setBusy(true);
    // A previous attempt's failure is no longer what is happening. Cleared as
    // this one starts rather than when it succeeds, so the line does not sit
    // under a disabled Approve button describing an answer already superseded.
    setFailed(null);
    try {
      await respondToConfirm(id, approved);
      // Stamped only once the answer has LANDED, and from the same `current`
      // the reader was shown. A rejection below means the click did not take
      // effect, so the gate is still genuinely pending and the record stays
      // that way — the transcript must not report a decision the backend never
      // accepted.
      //
      // Updates ONLY a record that already exists. Ownership was decided once,
      // at presentation (see the effect above) — not re-tested here, on
      // purpose: a run can finish (or start) between presentation and the
      // reader's click, and re-testing `busy` at THIS moment would leave a
      // gate this component did own stuck `pending` forever (the run finished
      // first) or invent one for a call it never owned (a run started after
      // presentation but before the click). Looking the id up is the only
      // check that agrees with the presentation-time decision either way.
      if (getAgentRun().gates.some((g) => g.id === id)) {
        noteGate({
          id,
          tool: current.tool,
          args: current.args,
          outcome: approved ? "approved" : "denied",
          at: Date.now(),
        });
      }
      // Only a landed answer takes the question away. By id, not by position —
      // see the file comment.
      setQueue((q) => q.filter((r) => r.id !== id));
    } catch (e) {
      // The request timed out server-side, or was answered elsewhere — the
      // click did not take effect. Swallowing this would let the reader believe
      // they approved (or denied) a call that had in fact already been settled
      // without them; dropping the request would do the same thing more
      // convincingly. So the prompt stays, carrying this, and the buttons stay
      // live — and the backend's own `mcp://confirm-resolved` is what takes it
      // down once the call really is settled.
      setFailed({ id, error: e });
    } finally {
      setBusy(false);
    }
  }

  if (covered) return null;

  // Not a live control on a covered window — it renders after the cover check
  // above, like the prompt — and not a modal: nothing here is blocking on it.
  // Fixed above the status bar so it sits in no tab and moves no layout, the
  // same reason the prompt is mounted where it is.
  const replayNotice = replayFailed !== null && (
    <Alert
      tone="sev"
      title="Agent requests raised before this window was ready could not be checked"
      onDismiss={() => setReplayFailed(null)}
      className="fixed bottom-10 right-3 z-40 max-w-md shadow-md"
    >
      <p className="m-0">
        Any agent call waiting on your approval from before srelens finished loading is not shown here,
        and is refused once its minute is up. The agent's transcript and the Audit pane say what was
        asked.
      </p>
      <FailureLine error={replayFailed} className="mt-1" />
    </Alert>
  );

  if (!current) return replayNotice || null;

  return (
    <>
      {replayNotice}
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
            {/*
              Only for the request it happened on — see {@link FailedAnswer}.
              `role="alert"` for the reason `NextApp`'s own inline failure has
              one: the reader pressed a button and the visible result is that
              nothing happened, so this has to be announced rather than merely
              drawn. It is safe to announce inside the card because the card is
              where focus already is.
            */}
            {failed?.id === current.id && (
              <div role="alert" className="text-sev">
                <p className="m-0">
                  This request was not answered by you: your answer did not take effect. Try
                  again — if the call is no longer waiting, this prompt goes away on its own.
                </p>
                <FailureLine error={failed.error} className="mt-1" />
              </div>
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
    </>
  );
}
