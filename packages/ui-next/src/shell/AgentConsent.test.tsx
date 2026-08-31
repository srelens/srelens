import { describe, it, expect, vi, beforeEach, type Mock } from "vitest";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

/**
 * The event bus is core's `subscribe`, not `@tauri-apps/api/event` — this
 * package depends on `@srelens/core` and `@srelens/ui-kit` and nothing else.
 * Captured per channel so a test can emit exactly what the backend emits.
 *
 * **Registration is DEFERRED here, because it is deferred in the real one.**
 * Tauri's `listen` is an IPC round trip: core's `on` starts it and returns
 * while it is still pending, and core's `subscribe` resolves only once it has
 * landed. The fixture this file shipped with registered synchronously, and
 * under it `on` and `await subscribe` were indistinguishable — the mutation
 * "fetch before subscribe" only ever caught the order of the CALLS, and the
 * gap one level down (a request raised after the snapshot was taken but
 * before the listener's `listen()` had resolved, in neither the snapshot nor
 * any delivered event) could not be expressed at all. So a subscription here
 * is PENDING until the test lands it: {@link settle} lands every registration
 * started so far, the way the IPC acks would, and {@link mount} settles until
 * nothing is pending. A payload broadcast while a channel's registration is
 * still pending is DROPPED, as the backend's emit drops an event nobody is
 * listening for yet — that drop is the defect, and the fixture has to be able
 * to perform it.
 */
type Handler = (payload: unknown) => void;
type Off = Mock<() => void>;

const bus = vi.hoisted(() => {
  interface Registration {
    channel: string;
    handler: Handler;
    off: Off;
    landed: (off: Off) => void;
  }
  /** What has LANDED — the only handlers a broadcast can reach. */
  const handlers = new Map<string, Handler>();
  /** Every disposer handed out, by channel, landed or not. */
  const offs = new Map<string, Off>();
  /** Started and not yet landed, in the order they were started. */
  const pending: Registration[] = [];

  /** Start a registration; it lands — and its promise resolves — on `land`. */
  function register(channel: string, handler: Handler): Promise<Off> {
    return new Promise((landed) => {
      const off = vi.fn(() => {
        if (handlers.get(channel) === handler) handlers.delete(channel);
      });
      offs.set(channel, off);
      pending.push({ channel, handler, off, landed });
    });
  }

  /** Land everything started so far, as the IPC acks would. */
  function land(): void {
    for (const r of pending.splice(0)) {
      handlers.set(r.channel, r.handler);
      r.landed(r.off);
    }
  }

  return { handlers, offs, pending, register, land };
});

const core = vi.hoisted(() => ({
  isTauri: vi.fn(() => true),
  // Typed rather than bare `vi.fn()`: `mock.calls` on an untyped mock is a
  // zero-length tuple, so a test that reads an argument back cannot compile,
  // and `toHaveBeenCalledWith` checks nothing about its arguments either.
  respondToConfirm: vi.fn<(id: string, approved: boolean) => Promise<void>>(async () => {}),
  pendingConfirms: vi.fn<() => Promise<ConfirmRequest[]>>(async () => []),
  notify: { success: vi.fn(), error: vi.fn(), info: vi.fn() },
  on: vi.fn(),
  subscribe: vi.fn(),
  vaultStatus: vi.fn(),
  // For the "a turn is in flight" tests below (C1): `askAgent` is the store's
  // own way to get `busy` true, and these are the three backend calls it
  // makes on the way to `sendChat`. Not exercising streaming itself — only
  // getting, and holding, the store in a busy state — so `sendChat` is made
  // to hang rather than resolve.
  startChat: vi.fn<() => Promise<string>>(async () => "sess-1"),
  sendChat: vi.fn(),
  listAgents: vi.fn(async () => [
    { kind: "claude", label: "Claude", available: true, path: "/c", version: "1", installUrl: "", gated: false },
  ]),
}));

vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  isTauri: core.isTauri,
  respondToConfirm: core.respondToConfirm,
  pendingConfirms: core.pendingConfirms,
  notify: core.notify,
  vaultStatus: core.vaultStatus,
  startChat: core.startChat,
  sendChat: core.sendChat,
  listAgents: core.listAgents,
  // Resolves once the registration has landed — the real one's contract.
  subscribe: (channel: string, handler: (payload: unknown) => void) => {
    core.subscribe(channel, handler);
    return bus.register(channel, handler);
  },
  // Returns at once with the registration still pending, and disposes after
  // it lands — the real one's shape (`tauriTransport.ts`), kept so that a
  // component reaching for `on` here is seen doing so rather than quietly
  // handed a synchronous listener the real bus never gives it.
  on: (channel: string, handler: (payload: unknown) => void) => {
    core.on(channel, handler);
    const landing = bus.register(channel, handler);
    let disposed = false;
    void landing.then((off) => {
      if (disposed) off();
    });
    return () => {
      disposed = true;
      void landing.then((off) => off());
    };
  },
}));

import type { ConfirmRequest } from "@srelens/core";
import { AgentConsent } from "./AgentConsent";
import { lockWorkspace, resetLock, __setKnownVaultMode } from "./LockGate";
import { askAgent, getAgentRun, resetAgentRun } from "../lib/agentRun";

const REQUEST = "mcp://confirm-request";
const RESOLVED = "mcp://confirm-resolved";

/**
 * Land every registration started so far, inside `act`, and let what that
 * unblocks run to completion — the component's next `await`, and the fetch it
 * makes once its listeners are in. A macrotask tick rather than a counted
 * number of microtasks, so this does not depend on how many `await`s sit
 * between one landing and the next.
 */
async function settle(): Promise<void> {
  await act(async () => {
    bus.land();
    await new Promise<void>((r) => setTimeout(r, 0));
  });
}

/** Settle until nothing is pending: the component as it is once its acks are in. */
async function settleAll(): Promise<void> {
  while (bus.pending.length > 0) await settle();
}

/** Render and settle: the subscribed component, as the real one is once its acks land. */
async function mount(): Promise<ReturnType<typeof render>> {
  const view = render(<AgentConsent />);
  await settleAll();
  return view;
}

/** Deliver to a landed listener. Throws if there is none: a test's own mistake. */
function emit(channel: string, payload: unknown): void {
  const handler = bus.handlers.get(channel);
  if (!handler) throw new Error(`nothing subscribed to ${channel}`);
  act(() => handler(payload));
}

/**
 * Deliver as the backend does: to whoever has landed, and to nobody otherwise.
 * Returns whether it was heard. This is the drop the deferred fixture exists
 * to perform — see the file comment.
 */
function broadcast(channel: string, payload: unknown): boolean {
  const handler = bus.handlers.get(channel);
  if (!handler) return false;
  act(() => handler(payload));
  return true;
}

const ask = (id: string, tool: string, args: Record<string, unknown> = {}) =>
  emit(REQUEST, { id, tool, args });

/**
 * Put the run store into `busy` — the only state `AgentConsent` can use to
 * decide a confirm is srelens's own agent's doing (C1). `askAgent` commits
 * `busy: true` synchronously, before its first `await`, so this is true the
 * instant the call is made — no need to await anything here. `sendChat` is
 * left hanging (never resolves) so the run STAYS busy for the rest of the
 * test; `resetAgentRun()` in the next `beforeEach` is what cleans it up, not
 * anything this helper does.
 */
function startTurn(): void {
  core.sendChat.mockImplementation(() => new Promise(() => {}));
  void askAgent("investigate checkout-api");
}

beforeEach(() => {
  vi.clearAllMocks();
  bus.handlers.clear();
  bus.offs.clear();
  bus.pending.length = 0;
  core.isTauri.mockReturnValue(true);
  core.respondToConfirm.mockResolvedValue(undefined);
  // Nothing was waiting when this mounted, unless a test says otherwise.
  core.pendingConfirms.mockResolvedValue([]);
  core.startChat.mockResolvedValue("sess-1");
  core.listAgents.mockResolvedValue([
    { kind: "claude", label: "Claude", available: true, path: "/c", version: "1", installUrl: "", gated: false },
  ]);
  // Nothing here renders the gate itself; the store is what this component
  // reads, and a raise in one test must not still be up in the next.
  resetLock();
  // Same reason, one store over: a gate recorded in one test — or a turn left
  // busy by `startTurn()` — must not still be sitting in the run for the next.
  resetAgentRun();
  // And an OPEN vault, said out loud rather than left to the store's default.
  // A fresh store has read no vault at all, and "no read has landed" counts as
  // covered where a vault exists — so without this every test in this file
  // would be exercising the refusal path and none of them would be about the
  // prompt. This is the state `LockGate`'s launch read establishes on the way
  // in; the covered cases below take it away deliberately.
  __setKnownVaultMode("unlocked");
});

describe("AgentConsent", () => {
  it("renders nothing until a request arrives", async () => {
    const { container } = await mount();
    expect(container.textContent).toBe("");
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("shows the tool and its arguments, and approves", async () => {
    await mount();
    ask("r1", "k8s_deletePod", { name: "web-1", namespace: "prod" });
    expect(await screen.findByText(/k8s_deletePod/)).toBeTruthy();
    expect(screen.getByText(/web-1/)).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: /approve/i }));
    await waitFor(() => expect(core.respondToConfirm).toHaveBeenCalledWith("r1", true));
  });

  it("denies on the Deny button", async () => {
    await mount();
    ask("r2", "k8s_scale");
    await screen.findByText(/k8s_scale/);
    await userEvent.click(screen.getByRole("button", { name: /deny/i }));
    await waitFor(() => expect(core.respondToConfirm).toHaveBeenCalledWith("r2", false));
  });

  it("queues a second request rather than dropping it", async () => {
    await mount();
    ask("a", "toolA");
    ask("b", "toolB");
    await screen.findByText(/toolA/);
    expect(screen.getByText(/1 more request waiting/i)).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: /deny/i }));
    expect(await screen.findByText(/toolB/)).toBeTruthy();
  });

  it("drops a request resolved elsewhere instead of lingering over it", async () => {
    await mount();
    ask("a", "toolA");
    ask("b", "toolB");
    await screen.findByText(/toolA/);
    emit(RESOLVED, { id: "a" });
    expect(await screen.findByText(/toolB/)).toBeTruthy();
    emit(RESOLVED, { id: "b" });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    // Answered by nobody here: the backend already settled it.
    expect(core.respondToConfirm).not.toHaveBeenCalled();
  });

  /**
   * A failed answer is the one thing here the reader has to be told, and this
   * dialog is the only surface that can tell them. `notify.error` reaches
   * sonner, whose `<Toaster>` is mounted in classic's `App` — and `main.tsx`
   * mounts that tree or this one and never both, so a toast raised from this
   * package is created and rendered nowhere. (#374 item 2)
   *
   * So the pin is on the DOM. The version of this test that shipped asserted
   * the `notify` mock had been called, which was true for the whole time
   * nothing whatsoever reached the reader — a test that could not fail for the
   * reason its own name gave.
   */
  it("tells the reader, on screen, when their answer did not take effect", async () => {
    core.respondToConfirm.mockRejectedValue(
      new Error("that confirmation is no longer waiting (it timed out or was already answered)"),
    );
    await mount();
    ask("r3", "k8s_deleteResource");
    await screen.findByText(/k8s_deleteResource/);
    await userEvent.click(screen.getByRole("button", { name: /approve/i }));

    const said = await screen.findByRole("alert");
    // What srelens knows for certain, in srelens's own words.
    expect(said.textContent).toMatch(/not answered by you/i);
    // And the reason, from the backend rather than invented here.
    expect(said.textContent).toMatch(/no longer waiting/i);
    // Not into the invisible sink instead: a toast host this tree does not
    // have is not a report, and a second copy of the failure would be one
    // problem said twice the moment #374's sweep mounts one.
    expect(core.notify.error).not.toHaveBeenCalled();
  });

  /**
   * The prompt SURVIVES a failed answer. Dropping the request in `finally` —
   * which is what shipped — took the question off screen exactly as though it
   * had been answered, and with the queue entry went the only way to try again.
   */
  it("keeps the prompt up when the answer did not land, so it can be tried again", async () => {
    core.respondToConfirm.mockRejectedValue(new Error("already timed out"));
    await mount();
    ask("r3", "k8s_deleteResource");
    await screen.findByText(/k8s_deleteResource/);
    await userEvent.click(screen.getByRole("button", { name: /approve/i }));
    await screen.findByRole("alert");

    expect(screen.getByRole("dialog")).toBeTruthy();
    expect(screen.getByText(/k8s_deleteResource/)).toBeTruthy();
    expect(screen.getByRole("button", { name: /approve/i }).hasAttribute("disabled")).toBe(false);

    // And a second press that does land takes the failure down with the prompt.
    core.respondToConfirm.mockResolvedValue(undefined);
    await userEvent.click(screen.getByRole("button", { name: /approve/i }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(screen.queryByRole("alert")).toBeNull();
  });

  /**
   * The failure belongs to the request it happened on, and is held BY ID for
   * the same reason removal is: `mcp://confirm-resolved` can take the head out
   * from under this component at any moment — and it always eventually does,
   * since `ResolveOnDrop` broadcasts on every exit from `confirm`. A failure
   * kept as a bare string would then be painted under the NEXT agent's
   * question, telling the reader a call had been refused that never was.
   */
  it("does not carry a failure over onto the next request", async () => {
    core.respondToConfirm.mockRejectedValue(new Error("already timed out"));
    await mount();
    ask("a", "toolA");
    ask("b", "toolB");
    await screen.findByText(/toolA/);
    await userEvent.click(screen.getByRole("button", { name: /approve/i }));
    await screen.findByRole("alert");

    // The backend announces the resolution it refused the answer for.
    emit(RESOLVED, { id: "a" });
    expect(await screen.findByText(/toolB/)).toBeTruthy();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  /**
   * The window, not one tab. Mounted outside every portal scope, the kit's
   * `ConfirmDialog` is a document-wide modal — `aria-modal`, Radix's own
   * overlay and its focus trap — which is what an app-wide question needs.
   * A tab-scoped one (PR #365) would cover a single tab and leave the rest of
   * the window live, and the reader could switch away from a question the
   * backend is blocking on.
   */
  it("asks the whole window rather than one tab of it", async () => {
    await mount();
    ask("r4", "k8s_drainNode");
    const card = await screen.findByRole("dialog");
    expect(card.getAttribute("aria-modal")).toBe("true");
    expect(document.querySelector('[data-slot="dialog-overlay"]')?.className).toContain("fixed");
  });

  // ---- Requests raised before this mounted -----------------------------

  /**
   * The backend emits `mcp://confirm-request` exactly once (`mcp_confirm.rs`),
   * and this component's listener is an effect that runs only once the new
   * design's chunks have downloaded and `createRoot` has run (`main.tsx`). A
   * request raised in that window used to be denied on timeout with nothing
   * ever on screen — the third time the same gap surfaced, each fix having
   * moved the listener earlier and left an earlier window. So a subscriber is
   * HANDED what is already waiting: `pendingConfirms()` is the backend's live
   * set, and a request in it is put to the reader exactly as an event would
   * have been, and answered once.
   */
  describe("a request raised before it mounted", () => {
    const PRE: ConfirmRequest = { id: "pre", tool: "k8s_deletePod", args: { name: "web-1" } };

    it("is put to the reader after mount, and answered exactly once", async () => {
      core.pendingConfirms.mockResolvedValue([PRE]);
      await mount();
      expect(await screen.findByText(/k8s_deletePod/)).toBeTruthy();
      expect(screen.getByText(/web-1/)).toBeTruthy();
      await userEvent.click(screen.getByRole("button", { name: /approve/i }));
      await waitFor(() => expect(core.respondToConfirm).toHaveBeenCalledWith("pre", true));
      expect(core.respondToConfirm).toHaveBeenCalledTimes(1);
      await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    });

    /**
     * Subscribe FIRST, then fetch — the order is the whole point. Fetch-then-
     * subscribe reopens the exact gap for a request raised between the two.
     * And the two can overlap in the other direction: the backend registers
     * the request before it emits, so a request that lands while the fetch is
     * in flight can arrive BOTH as an event and in the snapshot. It is one
     * request with one `oneshot::Sender`; it is shown once and answered once.
     *
     * The fixture plays that overlap: the fetch's own implementation delivers
     * the event for the same id before resolving with it. With the order
     * reversed there is no listener to deliver to when the fetch runs; without
     * the merge by id there are two prompts for one call.
     */
    it("shows a request that arrived both by event and in the snapshot once", async () => {
      core.pendingConfirms.mockImplementation(async () => {
        const handler = bus.handlers.get(REQUEST);
        if (!handler) throw new Error("fetched before subscribing: nothing is listening yet");
        handler({ ...PRE });
        return [PRE];
      });
      await mount();
      expect(await screen.findByText(/k8s_deletePod/)).toBeTruthy();
      // `mount` settled the fetch, so its merge has landed: one prompt, not two.
      expect(screen.queryByText(/more request/i)).toBeNull();
      await userEvent.click(screen.getByRole("button", { name: /deny/i }));
      await waitFor(() => expect(core.respondToConfirm).toHaveBeenCalledWith("pre", false));
      expect(core.respondToConfirm).toHaveBeenCalledTimes(1);
      await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    });

    /**
     * The snapshot is taken on the backend when the command runs and read here
     * when the response lands; a call can settle between the two, and the
     * backend announces that on `mcp://confirm-resolved` — which is already
     * subscribed, that being the order. A resolution heard while the fetch was
     * in flight wins over the snapshot: replaying that request would draw a
     * prompt whose answer can no longer land, and nothing would ever take it
     * down.
     */
    it("does not replay a request the backend resolved while the snapshot was in flight", async () => {
      core.pendingConfirms.mockImplementation(async () => {
        const resolved = bus.handlers.get(RESOLVED);
        if (!resolved) throw new Error("fetched before subscribing: nothing is listening yet");
        resolved({ id: "pre" });
        return [PRE];
      });
      const { container } = await mount();
      expect(screen.queryByRole("dialog")).toBeNull();
      expect(screen.queryByText(/k8s_deletePod/)).toBeNull();
      expect(container.textContent).toBe("");
    });

    /**
     * The cover applies to a replayed request exactly as to a live one. A
     * request raised while the window booted meets, after mount, the same
     * refusal a request raised over a sealed window does — it is not put to
     * whoever is at the keyboard because it happened to be raised early.
     */
    it("is refused rather than shown when the window is covered", async () => {
      core.pendingConfirms.mockResolvedValue([PRE]);
      render(<AgentConsent />);
      // The cover goes up while the registrations are still landing, so the
      // snapshot arrives into a covered window — the boot-time interleaving.
      act(() => lockWorkspace());
      await settleAll();
      await waitFor(() => expect(core.respondToConfirm).toHaveBeenCalledWith("pre", false));
      expect(core.respondToConfirm).not.toHaveBeenCalledWith("pre", true);
      expect(screen.queryByRole("dialog")).toBeNull();
      expect(screen.queryByText(/k8s_deletePod/)).toBeNull();
    });

    /**
     * A snapshot that could not be read is not nothing waiting. Every request
     * raised before this mounted is then lost to its timeout with nothing
     * drawn — the very failure the replay exists for — and the reader is the
     * only one who could go and look (the agent's transcript, the audit
     * trail). So it is said, on screen, and it does not take the live path
     * down with it: a request that arrives by event afterwards is still asked.
     */
    it("says on screen when what was waiting could not be read, and keeps listening", async () => {
      core.pendingConfirms.mockRejectedValue(new Error("no such command: mcp_confirm_pending"));
      await mount();
      const said = await screen.findByRole("alert");
      expect(said.textContent).toMatch(/raised before/i);
      // The reason comes from the backend, through `describeError`, not invented here.
      expect(said.textContent).toMatch(/mcp_confirm_pending/);
      ask("live", "k8s_scale");
      expect(await screen.findByText(/k8s_scale/)).toBeTruthy();
      await userEvent.click(screen.getByRole("button", { name: /approve/i }));
      await waitFor(() => expect(core.respondToConfirm).toHaveBeenCalledWith("live", true));
    });

    it("lets the reader put that notice away", async () => {
      core.pendingConfirms.mockRejectedValue(new Error("ipc down"));
      await mount();
      await screen.findByRole("alert");
      await userEvent.click(screen.getByRole("button", { name: /dismiss/i }));
      await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
    });

    /** A malformed entry is dropped, as a malformed event is. */
    it("ignores a snapshot entry it cannot answer", async () => {
      core.pendingConfirms.mockResolvedValue([
        { id: "", tool: "k8s_scale", args: {} },
        PRE,
      ]);
      await mount();
      expect(await screen.findByText(/k8s_deletePod/)).toBeTruthy();
      expect(screen.queryByText(/more request/i)).toBeNull();
    });

    // ---- The registrations are AWAITED, not merely ordered ---------------

    /**
     * Subscribe-then-fetch ordered the CALLS. Core's `on` starts `listen()`
     * and returns while the registration is still in flight; the fetch that
     * followed it was issued with no listener installed yet. So a request
     * registered on the backend after `mcp_confirm_pending` took its
     * snapshot but before that `listen()` resolved was in neither the
     * snapshot nor any delivered event — the same gap, one level down, and
     * the call timed out unseen. The listener has to have LANDED before the
     * snapshot is asked for, which is what core's `subscribe` (and not `on`)
     * promises.
     *
     * The fixture plays the backend: the snapshot is taken with nothing in
     * it, then a request is registered and emitted — into whoever is
     * listening at that instant, and dropped if nobody is. Under `on` that
     * is nobody.
     */
    it("prompts a request raised after the snapshot was taken but before a started listener had landed", async () => {
      const LATE: ConfirmRequest = { id: "late", tool: "k8s_evictPod", args: { name: "web-2" } };
      core.pendingConfirms.mockImplementation(async () => {
        // Snapshot: empty. Then the backend registers LATE and emits it.
        broadcast(REQUEST, LATE);
        return [];
      });
      await mount();
      expect(await screen.findByText(/k8s_evictPod/)).toBeTruthy();
      expect(screen.getByText(/web-2/)).toBeTruthy();
    });

    /**
     * And the two listeners land in a fixed order: RESOLVED first, REQUEST
     * second. Between one landing and the next there is a gap that can lose
     * an event, and the two events are not equal in what losing them costs.
     * A request event lost in that gap is recovered — the snapshot, read once
     * both have landed, still holds it. A resolution lost in that gap is
     * recovered by nothing: `mcp://confirm-resolved` is the one thing that
     * takes a prompt down once its answer can no longer land, and it is
     * emitted once. So the listener whose loss is unrecoverable goes in
     * first.
     *
     * Request-first would let this in: a request heard live in the gap, its
     * resolution dropped in the same gap, and no snapshot to correct it —
     * a prompt over a settled call, with nothing left to take it down.
     *
     * The fixture plays that gap. After the FIRST landing a request is
     * raised and settled, and the snapshot holds only what is still waiting.
     */
    it("lands the resolution listener first, so a call settled while the request listener was landing is not left on screen", async () => {
      const GONE: ConfirmRequest = { id: "gone", tool: "k8s_drainNode", args: { name: "node-3" } };
      core.pendingConfirms.mockResolvedValue([PRE]);
      render(<AgentConsent />);
      await settle();
      // Exactly one has landed and one is still in flight — the registrations
      // are awaited one at a time, and this is the gap between them.
      expect(bus.handlers.size).toBe(1);
      expect(bus.pending).toHaveLength(1);
      // In the gap: GONE is raised and settles. Heard by whoever has landed.
      broadcast(REQUEST, GONE);
      broadcast(RESOLVED, { id: GONE.id });
      await settleAll();
      await screen.findByRole("dialog");
      // What settled is not on screen — and not queued behind the head either.
      // What is still waiting is asked.
      expect(screen.queryByText(/k8s_drainNode/)).toBeNull();
      expect(screen.queryByText(/more request/i)).toBeNull();
      expect(screen.getByText(/k8s_deletePod/)).toBeTruthy();
      await userEvent.click(screen.getByRole("button", { name: /deny/i }));
      await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
      expect(core.respondToConfirm).toHaveBeenCalledTimes(1);
      expect(core.respondToConfirm).toHaveBeenCalledWith("pre", false);
    });
  });

  it("stops listening when it goes away", async () => {
    const { unmount } = await mount();
    const offRequest = bus.offs.get(REQUEST);
    const offResolved = bus.offs.get(RESOLVED);
    unmount();
    expect(offRequest).toHaveBeenCalledTimes(1);
    expect(offResolved).toHaveBeenCalledTimes(1);
    expect(bus.handlers.size).toBe(0);
  });

  /**
   * Registration is awaited, so an unmount can land in the middle of it: the
   * effect's cleanup runs while a `subscribe` is still pending, with no
   * disposer to call yet. The ack still arrives, and the listener it installs
   * belongs to a component that is gone. It must be disposed WHEN it lands —
   * as core's own `on` disposes after its `listen` resolves — and the effect
   * must go no further: no second listener for the gone component, no
   * snapshot read for a queue nobody is drawing, and nothing written to its
   * state.
   */
  describe("unmounted before its registration had landed", () => {
    it("still unlistens, and does not go on to subscribe or fetch", async () => {
      const { unmount } = render(<AgentConsent />);
      // The effect has started its first registration and is awaiting it.
      expect(bus.pending.length).toBeGreaterThan(0);
      expect(core.pendingConfirms).not.toHaveBeenCalled();
      unmount();
      await settleAll();
      expect(bus.offs.size).toBeGreaterThan(0);
      for (const off of bus.offs.values()) expect(off).toHaveBeenCalledTimes(1);
      expect(bus.handlers.size).toBe(0);
      // It stopped at the listener that was in flight: only that one was ever
      // subscribed, and the snapshot was never asked for.
      expect(core.subscribe).toHaveBeenCalledTimes(1);
      expect(core.pendingConfirms).not.toHaveBeenCalled();
    });

    it("disposes the listener that had landed and the one that had not", async () => {
      const { unmount } = render(<AgentConsent />);
      await settle();
      // One in, one still landing.
      expect(bus.handlers.size).toBe(1);
      expect(bus.pending).toHaveLength(1);
      const landed = [...bus.handlers.keys()][0];
      const inFlight = bus.pending[0].channel;
      expect(inFlight).not.toBe(landed);
      unmount();
      // The landed one is let go at once, on cleanup.
      expect(bus.offs.get(landed)).toHaveBeenCalledTimes(1);
      expect(bus.handlers.size).toBe(0);
      // The in-flight one, when its ack arrives.
      await settleAll();
      expect(bus.offs.get(inFlight)).toHaveBeenCalledTimes(1);
      expect(bus.handlers.size).toBe(0);
      expect(core.pendingConfirms).not.toHaveBeenCalled();
    });
  });

  /**
   * Every vault command and `mcp_confirm_respond` alike are Tauri commands, so
   * in a browser there is no gate to answer and nothing that could emit for it.
   */
  it("subscribes to nothing in web mode, and asks the backend nothing", () => {
    core.isTauri.mockReturnValue(false);
    render(<AgentConsent />);
    expect(core.subscribe).not.toHaveBeenCalled();
    expect(core.on).not.toHaveBeenCalled();
    expect(bus.pending).toHaveLength(0);
    expect(core.pendingConfirms).not.toHaveBeenCalled();
  });

  // ---- While the window is covered ------------------------------------

  /**
   * A prompt over a sealed window would be a live control on a sealed session:
   * the backend raises and focuses the window before it emits, so the person
   * who answers is whoever is at the keyboard — not necessarily the reader who
   * sealed it. Refusing is the safe default, and it is said immediately rather
   * than left to the backend's 60-second timeout.
   */
  describe("while the workspace is covered", () => {
    it("refuses a request rather than putting it to a sealed window", async () => {
      await mount();
      act(() => lockWorkspace());
      ask("r5", "k8s_drainNode");
      await waitFor(() => expect(core.respondToConfirm).toHaveBeenCalledWith("r5", false));
      expect(screen.queryByRole("dialog")).toBeNull();
    });

    it("refuses what was already on screen when the cover went up", async () => {
      await mount();
      ask("r6", "k8s_deletePod");
      await screen.findByRole("dialog");
      await act(async () => {
        lockWorkspace();
      });
      await waitFor(() => expect(core.respondToConfirm).toHaveBeenCalledWith("r6", false));
      expect(screen.queryByRole("dialog")).toBeNull();
    });

    it("refuses every queued request, not only the one in front", async () => {
      await mount();
      ask("a", "toolA");
      ask("b", "toolB");
      await screen.findByRole("dialog");
      await act(async () => {
        lockWorkspace();
      });
      await waitFor(() => {
        expect(core.respondToConfirm).toHaveBeenCalledWith("a", false);
        expect(core.respondToConfirm).toHaveBeenCalledWith("b", false);
      });
    });

    /**
     * The refusal is the cover's, not this component's: an unlocked window puts
     * the next question to the reader as before. Without this the whole surface
     * could be a `respondToConfirm(id, false)` and every test above would still
     * pass on the sealed ones.
     */
    it("puts a later request to the reader once the cover is down", async () => {
      await mount();
      act(() => lockWorkspace());
      ask("r7", "k8s_scale");
      await waitFor(() => expect(core.respondToConfirm).toHaveBeenCalledWith("r7", false));
      await act(async () => {
        resetLock();
        // The unlock's own read, which is what actually lowers a cover: a
        // store with no mode in it is still covered, so `resetLock` alone
        // would leave this test asserting the refusal it is the control for.
        __setKnownVaultMode("unlocked");
      });
      ask("r8", "k8s_evictPod");
      expect(await screen.findByText(/k8s_evictPod/)).toBeTruthy();
      expect(core.respondToConfirm).not.toHaveBeenCalledWith("r8", false);
    });

    /**
     * The state the branch had left open, and it is the dangerous one: a vault
     * NOTHING has read.
     *
     * `sealed` and the launch check are both written by a mounted `LockGate`.
     * Before one mounts — a fresh module, a webview reload after `Lock now` —
     * neither is set, and the store used to answer "not covered" about a vault
     * it had never read. This component is mounted outside the boot gate on
     * purpose (a request is emitted once and never replayed), and the MCP
     * server is a backend process that survives a reload: so a confirm-gated
     * call could arrive in exactly that window and be put to whoever was at the
     * keyboard, with an Approve button, over a vault the backend had sealed.
     *
     * Not-yet-known counts as covered, so the answer is a refusal.
     */
    it("refuses over a vault whose state nothing has read yet", async () => {
      // No mode: the store as a fresh module has it, and as a reloaded webview
      // has it. Nothing has been sealed here — that is the point.
      act(() => __setKnownVaultMode(null));
      await mount();
      ask("r10", "k8s_scale", { name: "api", replicas: 0 });
      await waitFor(() => expect(core.respondToConfirm).toHaveBeenCalledWith("r10", false));
      expect(core.respondToConfirm).not.toHaveBeenCalledWith("r10", true);
      expect(screen.queryByRole("dialog")).toBeNull();
      expect(screen.queryByText(/k8s_scale/)).toBeNull();
    });

    /**
     * The browser is the other half of that rule, and it is not asserted here:
     * there is no vault in web mode, so `knownMode` stays `null` for the life
     * of the page — and this component subscribes to nothing there, so no
     * request can be delivered to observe the decision on. The store is where
     * that half is pinned (`LockGate.test.tsx`, "a vault whose state nothing
     * has read yet"), which is also where the `isTauri()` condition lives.
     */

    /**
     * A refusal that cannot be delivered — the call already timed out — has
     * nothing to tell the reader: they did not ask the question and cannot act
     * on the answer. It must not throw either, which an unhandled rejection
     * out of an effect would.
     */
    it("says nothing to the reader about a refusal it made on their behalf", async () => {
      core.respondToConfirm.mockRejectedValue(new Error("already timed out"));
      const { container } = await mount();
      act(() => lockWorkspace());
      ask("r9", "k8s_drainNode");
      await waitFor(() => expect(core.respondToConfirm).toHaveBeenCalledWith("r9", false));
      // On screen rather than on a spy — the same reason the failure test
      // above changed. A covered window is the cover's, and this component
      // draws nothing over it: no prompt, and no failure line either.
      expect(screen.queryByRole("dialog")).toBeNull();
      expect(screen.queryByRole("alert")).toBeNull();
      expect(container.textContent).toBe("");
    });
  });

  // ---- What the run's transcript is told ------------------------------------

  /**
   * The transcript DRAWS a gate; it does not answer one. This component stays
   * the only subscriber to `mcp://confirm-request` and the only caller of
   * `respondToConfirm`, and the tests below are about the record it leaves
   * behind for the transcript to render — never about a second way to reply.
   *
   * Classic listened twice, and showed a modal and an inline card for one
   * request, each with its own buttons. Answering one left the other stale,
   * which is the whole reason `mcp://confirm-resolved` had to exist.
   */
  describe("the record it leaves in the run", () => {
    it("still answers exactly once, with the transcript only recording it", async () => {
      await mount();
      startTurn();
      ask("r1", "k8s_scale", { name: "api" });
      await userEvent.click(await screen.findByRole("button", { name: /approve/i }));
      await waitFor(() => expect(core.respondToConfirm).toHaveBeenCalledWith("r1", true));
      expect(core.respondToConfirm.mock.calls.filter(([id]) => id === "r1")).toHaveLength(1);
    });

    it("puts the request in the run as pending, before anyone has answered it, while srelens's own agent has a turn in flight", async () => {
      await mount();
      startTurn();
      ask("r1", "k8s_scale", { replicas: 3 });
      await screen.findByRole("dialog");
      // Recorded at presentation — this is what the transcript draws as
      // pending, and it must exist while the answer is still the reader's to
      // give.
      await waitFor(() => expect(getAgentRun().gates.find((g) => g.id === "r1")?.outcome).toBe("pending"));
      expect(core.respondToConfirm).not.toHaveBeenCalled();
      // Pending means pending: no resolution time on a gate nobody resolved.
      expect(getAgentRun().gates.find((g) => g.id === "r1")?.at).toBeUndefined();
      // M10: `Transcript.tsx` renders `summarizeArgs(gate.args)` — a gate
      // recorded with `args: undefined` would leave the reader
      // `k8s_scale · Applied 14:06` with no replica count.
      expect(getAgentRun().gates.find((g) => g.id === "r1")?.args).toEqual({ replicas: 3 });
    });

    it("records the outcome in the run without owning the answer", async () => {
      await mount();
      startTurn();
      ask("r1", "k8s_scale", { replicas: 7 });
      await userEvent.click(await screen.findByRole("button", { name: /approve/i }));
      await waitFor(() => expect(getAgentRun().gates.find((g) => g.id === "r1")?.outcome).toBe("approved"));
      // One row, not two: `noteGate` merges by id, so the pending record is
      // replaced rather than left sitting above its own answer.
      expect(getAgentRun().gates.filter((g) => g.id === "r1")).toHaveLength(1);
      expect(getAgentRun().gates.find((g) => g.id === "r1")?.at).toEqual(expect.any(Number));
      // M10 (the resolution side of the same risk): the resolved record must
      // still carry the args the reader was actually shown.
      expect(getAgentRun().gates.find((g) => g.id === "r1")?.args).toEqual({ replicas: 7 });
    });

    it("records a denial as a denial", async () => {
      // `outcome` carries three states and a denial is not the absence of an
      // approval. Without this, a field that only ever writes "approved"
      // passes the test above forever.
      await mount();
      startTurn();
      ask("r2", "k8s_deletePod", {});
      await userEvent.click(await screen.findByRole("button", { name: /deny/i }));
      await waitFor(() => expect(getAgentRun().gates.find((g) => g.id === "r2")?.outcome).toBe("denied"));
    });

    it("leaves the run alone when an answer did not land", async () => {
      // The click did not take effect, so the gate is still the reader's to
      // give. A record saying "approved" here would report a decision the
      // backend never accepted — and this component already keeps the prompt
      // up for exactly that reason.
      await mount();
      startTurn();
      core.respondToConfirm.mockRejectedValueOnce(new Error("already settled"));
      ask("r4", "k8s_rolloutRestart", {});
      await userEvent.click(await screen.findByRole("button", { name: /approve/i }));
      await screen.findByRole("alert");
      expect(getAgentRun().gates.find((g) => g.id === "r4")?.outcome).toBe("pending");
      expect(getAgentRun().gates.find((g) => g.id === "r4")?.at).toBeUndefined();
    });

    it("does not put a gate in the run for a request the cover refused unseen", async () => {
      // A request raised over a sealed window is refused without ever being
      // put to the reader. A record of it in the transcript would draw a
      // decision nobody was asked to make. This component gained a P1 once
      // already by answering over a sealed vault; this is the next change to
      // touch it.
      await mount();
      startTurn();
      act(() => lockWorkspace());
      ask("r3", "k8s_drainNode");
      await waitFor(() => expect(core.respondToConfirm).toHaveBeenCalledWith("r3", false));
      expect(getAgentRun().gates.find((g) => g.id === "r3")).toBeUndefined();
    });

    /**
     * The other half of the cover case, and the one the brief did not settle.
     *
     * A request that WAS shown, and then the cover went up: the reader was
     * genuinely asked, and genuinely never answered. `pending` is the honest
     * record of that. Writing `denied` would put the reader's name on a
     * refusal the cover made, which is the same defect as recording an unseen
     * request — one level subtler. See the ruling in the ledger.
     */
    it("leaves a gate the reader saw but never answered as pending", async () => {
      await mount();
      startTurn();
      ask("r5", "k8s_deletePod", {});
      await screen.findByRole("dialog");
      await act(async () => {
        lockWorkspace();
      });
      await waitFor(() => expect(core.respondToConfirm).toHaveBeenCalledWith("r5", false));
      expect(getAgentRun().gates.find((g) => g.id === "r5")?.outcome).toBe("pending");
    });

    // ---- Ownership at presentation (C1) ---------------------------------
    //
    // `ConfirmRequest` carries no client identity. The confirm channel is
    // app-wide by design — an external MCP client (the loopback HTTP server)
    // raises the exact same event srelens's own agent does — so the only
    // honest signal this component has is whether the run store has a turn
    // actually in flight AT PRESENTATION. Every test above now runs with
    // `startTurn()` so it is testing the busy case on purpose, not by
    // accident; the two tests below are the idle case those tests deliberately
    // exclude.

    it("does not record a gate for a confirm raised while no srelens turn is in flight, but still presents and answers it", async () => {
      // No `startTurn()`: the store is idle, as it is for a confirm an
      // external MCP client raised — the scenario this fix exists for.
      await mount();
      ask("ext1", "k8s_scale", { replicas: 2 });
      await screen.findByRole("dialog");
      expect(getAgentRun().gates.find((g) => g.id === "ext1")).toBeUndefined();
      await userEvent.click(screen.getByRole("button", { name: /approve/i }));
      await waitFor(() => expect(core.respondToConfirm).toHaveBeenCalledWith("ext1", true));
    });

    it("does not create a gate on the way out, for an answer whose request was never recorded", async () => {
      // Same idle setup, but the assertion is about what `answer()` does
      // AFTER a landed response — it must look the id up rather than writing
      // one unconditionally, or an unowned confirm would grow a gate the
      // instant it was answered even though presentation recorded none.
      await mount();
      ask("ext2", "k8s_scale", { replicas: 5 });
      await userEvent.click(await screen.findByRole("button", { name: /deny/i }));
      await waitFor(() => expect(core.respondToConfirm).toHaveBeenCalledWith("ext2", false));
      expect(getAgentRun().gates.find((g) => g.id === "ext2")).toBeUndefined();
    });

    /**
     * The mutation the brief calls out by name: re-testing `busy` in
     * `answer()` instead of looking up the record already made at
     * presentation. A run that finishes mid-gate (srelens's own agent's
     * `sendChat` settles while the reader is still looking at the dialog) must
     * still stamp the outcome on the record presentation made — re-testing
     * `busy` here would find the store idle again and skip the stamp, leaving
     * an owned gate stuck `pending` forever.
     */
    it("still stamps the outcome when the run finishes while the gate is still on screen", async () => {
      await mount();
      let resolveSendChat!: (v: string | null) => void;
      core.sendChat.mockImplementation(
        () => new Promise<string | null>((resolve) => { resolveSendChat = resolve; }),
      );
      void askAgent("investigate checkout-api");
      ask("r6", "k8s_scale", { replicas: 4 });
      await waitFor(() => expect(getAgentRun().gates.find((g) => g.id === "r6")?.outcome).toBe("pending"));

      // The run finishes — `busy` goes back to false — before the reader answers.
      resolveSendChat(null);
      await waitFor(() => expect(getAgentRun().busy).toBe(false));

      await userEvent.click(await screen.findByRole("button", { name: /approve/i }));
      await waitFor(() => expect(getAgentRun().gates.find((g) => g.id === "r6")?.outcome).toBe("approved"));
      expect(getAgentRun().gates.find((g) => g.id === "r6")?.at).toEqual(expect.any(Number));
    });
  });

});
