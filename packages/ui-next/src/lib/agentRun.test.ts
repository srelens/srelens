import { describe, it, expect, vi, beforeEach } from "vitest";
import { describeError } from "@srelens/core";
import { runKeyFor } from "./askContext";
import {
  askAgent,
  chooseAgent,
  clearAgentRun,
  getAgentRun,
  noteGate,
  resetAgentRun,
  setSkillActive,
  stopAgentRun,
  subscribeAgentRun,
  restoreRuns,
  openSavedRun,
  getRun,
  getRunSummaries,
  getActiveRunKey,
  getRunSubject,
  selectRun,
  forgetRun,
} from "./agentRun";

const { startChat, sendChat, listAgents, cancelChat, loadSkill, listSessions, loadSession, saveSession, deleteSession } =
  vi.hoisted(() => ({
    startChat: vi.fn(), sendChat: vi.fn(), listAgents: vi.fn(), cancelChat: vi.fn(), loadSkill: vi.fn(),
    listSessions: vi.fn(), loadSession: vi.fn(), saveSession: vi.fn(), deleteSession: vi.fn(),
  }));
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  startChat, sendChat, listAgents, cancelChat, loadSkill,
  listSessions, loadSession, saveSession, deleteSession,
}));

/** Let queued microtasks run until `sendChat` has been called `n` times, or
 *  give up after a generous number of ticks — used to let two overlapping
 *  `askAgent` calls each reach their own `sendChat` before the test drives
 *  either one's resolution, without guessing an exact tick count. */
/** Let queued microtasks run until `listAgents` has been called — i.e. the
 *  turn is inside the preparation window, past `startChat` and short of
 *  `sendChat`, which is the window round 3's P1 was about. */
async function untilListAgentsCalled(): Promise<void> {
  for (let i = 0; i < 50 && listAgents.mock.calls.length === 0; i++) await Promise.resolve();
}

async function untilSendChatCalledTimes(n: number): Promise<void> {
  for (let i = 0; i < 50 && sendChat.mock.calls.length < n; i++) {
    await Promise.resolve();
  }
}

beforeEach(() => {
  resetAgentRun();
  // Full reset, not just a clear: each test's own `mock.calls.length` — the
  // "superseded turn" test's `untilSendChatCalledTimes` in particular — must
  // count only what THAT test did, not every `sendChat` call the whole file
  // has made so far.
  startChat.mockReset();
  sendChat.mockReset();
  listAgents.mockReset();
  cancelChat.mockReset();
  loadSkill.mockReset();
  // Reset, not merely re-stubbed: `mockResolvedValue` leaves call history in
  // place, so without these the persistence tests below counted every
  // `saveSession` the whole file had made — 115 of them.
  listSessions.mockReset();
  loadSession.mockReset();
  saveSession.mockReset();
  deleteSession.mockReset();
  startChat.mockResolvedValue("sess-1");
  listSessions.mockResolvedValue([]);
  saveSession.mockResolvedValue(undefined);
  deleteSession.mockResolvedValue(undefined);
  listAgents.mockResolvedValue([{ kind: "claude", label: "Claude", available: true, path: "/c", version: "1", installUrl: "", gated: false }]);
  cancelChat.mockResolvedValue(undefined);
});

describe("the run store", () => {
  it("records the reader's question before the agent has said anything", async () => {
    sendChat.mockImplementation(async () => null);
    await askAgent("why is checkout-api 5xx?");
    const turns = getAgentRun().turns;
    expect(turns[0].role).toBe("user");
    expect(turns[0].text).toBe("why is checkout-api 5xx?");
  });

  it("times a tool call from its start to its result, and reports nothing until then", async () => {
    let emit: (e: unknown) => void = () => {};
    sendChat.mockImplementation(async (_s: string, _p: string, _a: string, onEvent: (e: unknown) => void) => {
      emit = onEvent;
      emit({ type: "toolCallStart", id: "t1", tool: "k8s.listPods", args: { ns: "checkout" } });
      // No result yet: the duration is not known, so it is not drawn.
      expect(getAgentRun().turns.at(-1)?.calls[0].ms).toBeUndefined();
      emit({ type: "toolResult", id: "t1", status: "ok" });
      emit({ type: "turnDone" });
      return null;
    });
    await askAgent("q");
    const call = getAgentRun().turns.at(-1)?.calls[0];
    expect(call?.status).toBe("ok");
    expect(typeof call?.ms).toBe("number");
  });

  // Replaces a version that only called `getAgentRun()` twice with no action
  // in between — true for any store that doesn't spontaneously mutate on
  // read, guard or no guard. This drives an actual write that changes
  // nothing a reader could see, through the public API, and checks the
  // snapshot survives it.
  it("keeps its snapshot identity across a write that changes nothing", () => {
    const before = getAgentRun();
    chooseAgent(before.agentKind); // already the current kind
    expect(getAgentRun()).toBe(before);
  });

  // Replaces a version that asserted only `toHaveBeenCalled()`, which cannot
  // tell one notification apart from several. This drives a known sequence —
  // the turn's own append, a real text delta, a no-op delta that must NOT
  // notify, a tool call starting, its result landing, an event with no state
  // at all, and finally `busy` coming down — and checks the count lands on
  // exactly the number of real changes: 5.
  it("notifies once per real change, and not at all for one that changes nothing", async () => {
    const seen = vi.fn();
    const off = subscribeAgentRun(seen);
    sendChat.mockImplementation(async (_s: string, _p: string, _a: string, onEvent: (e: unknown) => void) => {
      onEvent({ type: "textDelta", text: "checkout-api is failing" }); // 1 real change
      onEvent({ type: "textDelta", text: "" }); // no-op: same text after appending nothing
      onEvent({ type: "toolCallStart", id: "t1", tool: "k8s.listPods", args: {} }); // 1 real change
      onEvent({ type: "toolResult", id: "t1", status: "ok" }); // 1 real change
      onEvent({ type: "turnDone" }); // no state touched at all
      return null;
    });
    await askAgent("q"); // the turn's own append (+1), and busy:false in `finally` (+1)
    off();
    expect(seen).toHaveBeenCalledTimes(5);
  });

  it("merges a gate by id, so a second outcome for the same request replaces the first", async () => {
    // A gate belongs to the run whose agent called the tool, so there has to
    // BE one with a turn in flight — with runs keyed by subject, "the store"
    // is no longer a single conversation that exists by default.
    sendChat.mockImplementation(() => new Promise<string | null>(() => {}));
    void askAgent("q", { about: { cluster: "prod-eu" }, route: "/overview" });
    await untilSendChatCalledTimes(1);

    noteGate({ id: "g1", tool: "k8s.scale", args: { replicas: 3 }, outcome: "pending" });
    noteGate({ id: "g1", tool: "k8s.scale", args: { replicas: 3 }, outcome: "approved" });
    const gates = getAgentRun().gates;
    expect(gates).toHaveLength(1);
    expect(gates[0].outcome).toBe("approved");
  });

  it("records no gate when no run has a turn in flight — an external client's confirm", () => {
    // The #393 case, now structural rather than a heuristic: with no busy run
    // there is no conversation to attribute a mutation to.
    noteGate({ id: "external", tool: "k8s.deletePod", args: {}, outcome: "pending" });
    expect(getAgentRun().gates).toEqual([]);
  });

  it("chooseAgent switches which agent the next question goes to, without touching the conversation", () => {
    const before = getAgentRun();
    chooseAgent("cursor");
    const after = getAgentRun();
    expect(after.agentKind).toBe("cursor");
    expect(after).not.toBe(before);
    expect(after.turns).toBe(before.turns);
  });

  // C1: a gate is a row in THIS conversation's transcript, not a fact that
  // outlives it — `Transcript` draws `gates` whenever it is non-empty, with
  // no check that a gate's turn is still among `turns`. The version of this
  // test that shipped was named for the defect it was pinning: it asserted
  // `gates` survives a clear, which is exactly the bug (approve a mutation,
  // start a fresh conversation, and the old gate is still drawn under it).
  it("clears the conversation and its gates, but keeps the chosen agent", async () => {
    sendChat.mockImplementation(async () => null);
    await askAgent("q");
    noteGate({ id: "g1", tool: "k8s.scale", args: {}, outcome: "pending" });
    chooseAgent("codex");
    setSkillActive("rollout", true);

    clearAgentRun();

    const run = getAgentRun();
    expect(run.turns).toEqual([]);
    expect(run.busy).toBe(false);
    // Gates belong to the conversation that raised them, not to the window.
    expect(run.gates).toEqual([]);
    // The chosen agent genuinely does survive a clear — only `gates` changes.
    expect(run.agentKind).toBe("codex");
    // `activeSkills` SURVIVES a clear, and that is a deliberate change
    // (ruling AD). It used to be per-run and to be dropped here, on the
    // reasoning that a skill picked for a run that ended is not still active.
    // With runs keyed by subject that broke: there is no run to hold the set
    // before the first question, which is exactly when the rail offers the
    // switch, and the rail can be showing a different run than the dock. So
    // it is window-wide now, beside `agentKind`, and the reader's pick applies
    // to whatever they ask next.
    expect(run.activeSkills).toEqual(["rollout"]);
  });

  // I2: `session` is created lazily inside `askAgent`, AFTER `busy` and
  // `generation` are already committed — so a Stop pressed on a cold start,
  // before `startChat()` has even resolved, used to see `!session`, return,
  // and let the question go out anyway once the CLI finished spawning.
  it("a Stop pressed before the first sendChat drops the question rather than sending it anyway", async () => {
    let resolveStartChat!: (v: string) => void;
    startChat.mockImplementation(() => new Promise<string>((resolve) => { resolveStartChat = resolve; }));

    const asked = askAgent("q");
    expect(getAgentRun().busy).toBe(true);
    stopAgentRun();
    // The stop takes effect immediately, before the CLI has even started.
    expect(getAgentRun().busy).toBe(false);

    resolveStartChat("sess-1");
    await asked;

    expect(sendChat).not.toHaveBeenCalled();
    expect(getAgentRun().busy).toBe(false);
  });

  // P1 round 3 (#392): the first fix guarded only the `startChat` window.
  // `loadSkillsGuidance` and `listAgents` are two more awaits before dispatch.
  it("does not dispatch a turn the reader discarded while its agent list was loading", async () => {
    let resolveAgents!: (v: unknown) => void;
    listAgents.mockImplementation(() => new Promise((resolve) => { resolveAgents = resolve; }));

    const asked = askAgent("q");
    // Past `startChat`, inside the preparation window the old guard missed.
    await untilListAgentsCalled();
    clearAgentRun();
    resolveAgents([{ kind: "claude", label: "Claude", available: true, gated: false, path: "/c" }]);
    await asked;

    expect(sendChat).not.toHaveBeenCalled();
  });

  it("dispatches against the session it started, not whatever the global holds by then", async () => {
    // The other half: without capturing, `sendChat` read the mutable global —
    // so a discarded turn went out with `null`, or under a later question's
    // session entirely.
    let resolveAgents!: (v: unknown) => void;
    listAgents.mockImplementation(() => new Promise((resolve) => { resolveAgents = resolve; }));
    const asked = askAgent("q");
    await untilListAgentsCalled();
    resolveAgents([{ kind: "claude", label: "Claude", available: true, gated: false, path: "/c" }]);
    await asked;
    expect(sendChat.mock.calls.at(-1)?.[0]).toBe("sess-1");
  });

  // P2 round 3 (#392): some `error` events are not terminal — the backend
  // emits them and carries on.
  describe("a non-terminal error event", () => {
    it("does not turn a successful answer into an error turn", async () => {
      sendChat.mockImplementationOnce(async (_s, _t, _p, onEvent) => {
        onEvent({ type: "error", message: "image attachments are only supported with the Codex agent" });
        onEvent({ type: "textDelta", text: "## Findings\n\nThe rollout is healthy." });
        return null;
      });

      await askAgent("look at this", { images: ["data:image/png;base64,AAA"] });

      const turn = getAgentRun().turns.at(-1)!;
      // Still the agent's answer — markdown and tool rows intact, not red.
      expect(turn.role).toBe("agent");
      expect(turn.text).toContain("The rollout is healthy.");
      // And the warning is not swallowed: the attachment really did not arrive.
      expect(turn.notes?.some((n) => /image attachments/i.test(n))).toBe(true);
    });

    it("is the whole story when nothing else arrived, and then the turn IS the error", async () => {
      sendChat.mockImplementationOnce(async (_s, _t, _p, onEvent) => {
        onEvent({ type: "error", message: "could not decode attached image 0: bad padding" });
        return null;
      });

      await askAgent("look at this");

      const turn = getAgentRun().turns.at(-1)!;
      expect(turn.role).toBe("error");
      expect(turn.text).toMatch(/bad padding/);
      // Folded into the turn rather than left duplicated beside it.
      expect(turn.notes).toBeUndefined();
    });
  });

  // P1 round 5 (#392): classic prepends a context preface; the new design
  // sent nothing, so a question like "what is unhealthy right now?" gave the
  // agent no way to name the cluster on screen — and every MCP tool call
  // takes an explicit context, so it would have to guess one.
  describe("what a question is about", () => {
    it("names the cluster to the agent, without putting it in the reader's transcript", async () => {
      sendChat.mockResolvedValue(null);
      await askAgent("what is unhealthy right now?", { about: { cluster: "prod-eu" } });

      // Sent.
      expect(sendChat.mock.calls.at(-1)?.[1]).toBe(
        "Current context: cluster prod-eu.\n\nwhat is unhealthy right now?",
      );
      // Not recorded — the transcript keeps the reader's own words, the same
      // split the skills guidance follows.
      expect(getAgentRun().turns.find((t) => t.role === "user")?.text).toBe("what is unhealthy right now?");
    });

    it("sends no preface at all when there is no cluster to name", async () => {
      sendChat.mockResolvedValue(null);
      await askAgent("hello", { about: { cluster: "   " } });
      // Not an empty "Current context: cluster ." line — a preface naming
      // nothing is worse than none.
      expect(sendChat.mock.calls.at(-1)?.[1]).toBe("hello");
    });

    it("puts the cluster ahead of the skills guidance, not inside it", async () => {
      sendChat.mockResolvedValue(null);
      loadSkill.mockResolvedValue({ name: "oom", body: "oom body" });
      setSkillActive("oom", true);
      await askAgent("why", { about: { cluster: "prod-eu" } });
      expect(sendChat.mock.calls.at(-1)?.[1]).toBe(
        "Current context: cluster prod-eu.\n\nApply these skills:\n\noom body\n\nwhy",
      );
    });

    // The defect a screenshot caught: "Summarise this stream" reached the agent
    // with the cluster alone, so it had no target and went searching four
    // namespaces for a pod to read.
    it("names the namespace and the resource, not just the cluster", async () => {
      sendChat.mockResolvedValue(null);
      await askAgent("Summarise the last 500 log lines and group errors by cause", {
        about: {
          cluster: "m01-1786968575165/kubernetes-admin@cluster.local",
          namespace: "m01-cnips-01-services",
          kind: "Pod",
          name: "ai-editor",
        },
      });
      const sent = sendChat.mock.calls.at(-1)?.[1] as string;
      expect(sent).toContain("namespace m01-cnips-01-services");
      expect(sent).toContain("Pod ai-editor");
    });

    it("says the reader is looking at logs, so 'this stream' has a referent", async () => {
      sendChat.mockResolvedValue(null);
      await askAgent("Summarise the last 500 log lines and group errors by cause", {
        about: { cluster: "prod-eu", namespace: "ns", kind: "Pod", name: "ai-editor", surface: "logs" },
      });
      const sent = sendChat.mock.calls.at(-1)?.[1] as string;
      expect(sent).toMatch(/looking at ai-editor's logs/);
      expect(sent).toMatch(/"this stream" means that pod's logs/);
    });

    it("tells the agent the scope the reader set with the namespace picker", async () => {
      sendChat.mockResolvedValue(null);
      await askAgent("which MongoDB replica set spiked?", {
        about: { cluster: "prod-eu", kind: "StatefulSet", namespaces: ["m01-prod-04-dataservices"] },
      });
      const sent = sendChat.mock.calls.at(-1)?.[1] as string;
      expect(sent).toMatch(/narrowed to namespace m01-prod-04-dataservices/);
      expect(sent).toMatch(/that is the scope of the question/);
    });

    /**
     * "Pass kind type like which tab is opened." A list has a kind and no
     * name, and the agent cannot see the tab — so which list is open is
     * something only srelens can say.
     */
    it("says which list the reader is looking at, when that is all there is", async () => {
      sendChat.mockResolvedValue(null);
      await askAgent("what is unhealthy", { about: { cluster: "prod-eu", kind: "StatefulSet" } });
      const sent = sendChat.mock.calls.at(-1)?.[1] as string;
      expect(sent).toMatch(/looking at the StatefulSet list/);
    });

    it("says list of nothing when the route named a resource", async () => {
      sendChat.mockResolvedValue(null);
      await askAgent("why is it restarting", {
        about: { cluster: "prod-eu", namespace: "ns", kind: "Pod", name: "mongodb-0" },
      });
      const sent = sendChat.mock.calls.at(-1)?.[1] as string;
      // A kind WITH a name is the subject itself, already stated as
      // `Pod mongodb-0`. Adding "looking at the Pod list" beside it would be
      // srelens describing a screen the reader is not on.
      expect(sent).toMatch(/namespace ns, Pod mongodb-0/);
      expect(sent).not.toMatch(/looking at the Pod list/);
    });

    it("says nothing about scope when the reader narrowed to nothing", async () => {
      sendChat.mockResolvedValue(null);
      await askAgent("what is unhealthy", { about: { cluster: "prod-eu", namespaces: [] } });
      expect(sendChat.mock.calls.at(-1)?.[1]).toBe("Current context: cluster prod-eu.\n\nwhat is unhealthy");
    });

    it("omits a namespace a cluster-scoped resource does not have", async () => {
      sendChat.mockResolvedValue(null);
      await askAgent("why is it not ready", {
        about: { cluster: "prod-eu", kind: "Node", name: "worker-3" },
      });
      const sent = sendChat.mock.calls.at(-1)?.[1] as string;
      expect(sent).toBe("Current context: cluster prod-eu, Node worker-3.\n\nwhy is it not ready");
      expect(sent).not.toMatch(/namespace/);
    });
  });

  // P2 round 7 (#392): `chooseAgent` closed the MANUAL switch. `askAgent`'s
  // own fallback makes the same switch automatically — the previous agent left
  // PATH — and was still handing the old agent's conversation id to the new
  // one.
  it("does not hand the old agent's resume token to the one it falls back to", async () => {
    sendChat.mockResolvedValue("claude-conversation-id");
    await askAgent("q1");
    expect(sendChat.mock.calls.at(-1)?.[5]).toBe("claude");

    // Claude is gone; only Codex remains. `agentKind` still says "claude".
    listAgents.mockResolvedValue([
      { kind: "codex", label: "Codex", available: true, gated: false, path: "/codex" },
    ]);
    await askAgent("q2");

    expect(sendChat.mock.calls.at(-1)?.[5]).toBe("codex");
    // Codex is asked to start, not to resume a Claude conversation.
    expect(sendChat.mock.calls.at(-1)?.[7]).toBeNull();
  });

  it("keeps the conversation when the fallback lands on the same kind", async () => {
    sendChat.mockResolvedValue("claude-conversation-id");
    await askAgent("q1");
    // Claude still installed: no switch, so nothing to invalidate.
    await askAgent("q2");
    expect(sendChat.mock.calls.at(-1)?.[7]).toBe("claude-conversation-id");
  });

  // P2 (#392 review): `resume` is the PREVIOUS CLI's own conversation id.
  describe("switching agent CLIs", () => {
    it("does not hand one CLI's resume token to another", async () => {
      sendChat.mockResolvedValue("claude-conversation-id");
      await askAgent("q1");
      expect(sendChat.mock.calls.at(-1)?.[7]).toBeNull();

      chooseAgent("cursor");
      listAgents.mockResolvedValue([
        { kind: "cursor", label: "Cursor", available: true, gated: false, path: "/cursor" },
      ]);
      await askAgent("q2");

      // Cursor is asked to start something, not to resume a conversation it
      // has never heard of.
      expect(sendChat.mock.calls.at(-1)?.[7]).toBeNull();
      expect(sendChat.mock.calls.at(-1)?.[5]).toBe("cursor");
    });

    it("keeps the conversation when the chosen kind has not actually changed", async () => {
      sendChat.mockResolvedValue("claude-conversation-id");
      await askAgent("q1");

      // `Composer`'s reconciliation effect calls this with whatever it can
      // offer; a no-op call must not end the run the reader is in.
      chooseAgent("claude");
      await askAgent("q2");

      expect(sendChat.mock.calls.at(-1)?.[7]).toBe("claude-conversation-id");
    });

    it("keeps the transcript across the switch — those questions were asked", async () => {
      sendChat.mockResolvedValue(null);
      await askAgent("q1");
      const before = getAgentRun().turns.length;
      chooseAgent("cursor");
      expect(getAgentRun().turns).toHaveLength(before);
    });
  });

  // P1 (#392 review): `clearAgentRun` preserved `run.generation`, so every
  // post-await guard in `askAgent` still read a discarded turn as current.
  describe("New question, with a turn still in flight", () => {
    it("does not send a question the reader threw away before its CLI had spawned", async () => {
      let resolveStartChat!: (v: string) => void;
      startChat.mockImplementation(() => new Promise<string>((resolve) => { resolveStartChat = resolve; }));

      const asked = askAgent("q");
      expect(getAgentRun().busy).toBe(true);
      clearAgentRun();

      resolveStartChat("sess-1");
      await asked;

      // The discarded turn must not reassign the session the clear nulled,
      // and must not go on to ask its question.
      expect(sendChat).not.toHaveBeenCalled();
      expect(getAgentRun().turns).toEqual([]);
    });

    it("does not let a discarded turn's resume token survive the clear", async () => {
      let resolveSend!: (v: string | null) => void;
      sendChat.mockImplementationOnce(() => new Promise<string | null>((resolve) => { resolveSend = resolve; }));

      const asked = askAgent("q1");
      await untilSendChatCalledTimes(1);
      clearAgentRun();
      resolveSend("stale-token");
      await asked;

      // The next question starts a conversation, rather than resuming the one
      // the reader cleared. Pinned on the resume slot alone: coupling this to
      // all eight arguments would make it fail for reasons that have nothing
      // to do with the token, which is the whole subject of the test.
      sendChat.mockImplementationOnce(async () => null);
      await askAgent("q2");
      expect(sendChat.mock.calls.at(-1)?.[1]).toBe("q2");
      // `null` is the store's "no conversation to resume" — the point is that
      // the discarded turn's "stale-token" is not sitting here.
      expect(sendChat.mock.calls.at(-1)?.[7]).toBeNull();
    });

    it("cancels the in-flight turn with the generation it was sent with, not the bumped one", async () => {
      sendChat.mockImplementationOnce(() => new Promise<string | null>(() => {}));
      void askAgent("q");
      await untilSendChatCalledTimes(1);
      const sent = getAgentRun().generation;

      clearAgentRun();

      expect(cancelChat).toHaveBeenCalledWith("sess-1", sent);
      // And the run has moved on, so the abandoned turn's own guards all fail.
      expect(getAgentRun().generation).toBe(sent + 1);
    });

    /**
     * Codex P1. Stop before `startChat` resolves leaves the generation where it
     * is (there is no session to cancel), so the marker is the abandoned turn's
     * only evidence — and `askAgent` clears that marker for the turn it is
     * starting. The discarded question then passed BOTH guards and went out
     * beside the replacement, on a session Stop could no longer reach.
     */
    /**
     * Codex P2. A `cancelChat` rejection landing after the reader had cleared
     * the run or asked something else spread whatever `run` was current at
     * rejection time — so a NEW conversation was told "That question was not
     * sent" for a Stop belonging to the one before it.
     */
    /**
     * Codex P2. `askAgent` has to create a run to have somewhere to put the
     * refusal, and it was left behind: a newest "0 questions" row in Recent
     * runs, and `/agent` opening on a blank transcript for a question that was
     * never sent.
     */
    /**
     * Codex P2. The agent turn was stamped when the reader pressed Enter, and
     * the transcript draws that clock beneath the FINISHED answer — so an
     * answer that streamed for minutes was labelled with the submission time.
     */
    it("stamps the answer when it settles, and the question when it was asked", async () => {
      // Fake timers, and REAL ones restored in the `finally`. The first draft
      // moved the system clock and never put it back, which left every later
      // test in this file running 90 seconds into the future — and the full
      // suite failed once, in this file, in a way that would not reproduce on
      // its own.
      vi.useFakeTimers();
      try {
        const asked = Date.now();
        let finish: ((s: string | null) => void) | undefined;
        sendChat.mockImplementationOnce(() => new Promise<string | null>((res) => (finish = res)));
        void askAgent("why is it restarting");
        await untilSendChatCalledTimes(1);

        const askedTurn = getAgentRun().turns[0];
        // Time passes while the answer streams.
        vi.setSystemTime(asked + 90_000);
        finish?.(null);
        for (let i = 0; i < 50; i++) await Promise.resolve();

        const turns = getAgentRun().turns;
        // The question keeps its own submission time — that is what it means.
        expect(turns[0].at).toBe(askedTurn.at);
        // The answer carries when it actually appeared.
        expect(turns[1].at).toBe(asked + 90_000);
      } finally {
        vi.useRealTimers();
      }
    });

    it("leaves no conversation behind for a question it refused", async () => {
      const pod = {
        about: { cluster: "prod-eu", namespace: "ns", kind: "Pod", name: "mongodb-0" },
        route: "/k/pods/ns/mongodb-0",
      };
      // What a subscriber SEES at each notification, not merely that it was
      // notified: `commitTo(busy:false)` emits in the same flush, so "was
      // called" is true with the removal's own emit deleted.
      const snapshots: { rows: number; key: string | null }[] = [];
      const list = { about: { cluster: "prod-eu" }, route: "/k/statefulsets" };
      let finish: ((s: string | null) => void) | undefined;
      sendChat.mockImplementationOnce(() => new Promise<string | null>((res) => (finish = res)));
      void askAgent("the one in flight", pod);
      await untilSendChatCalledTimes(1);

      // A question about a DIFFERENT subject, refused because that turn is busy.
      await askAgent("refused", list);
      expect(getAgentRun().error).toMatch(/still answering/);

      // Subscribed only now, so the count below is about the removal rather
      // than about everything the two questions already emitted.
      const off = subscribeAgentRun(() =>
        snapshots.push({ rows: getRunSummaries().length, key: getActiveRunKey() }),
      );
      finish?.(null);
      for (let i = 0; i < 50; i++) await Promise.resolve();
      off();

      // The LAST thing a subscriber was told already has the refusal gone. A
      // delete does not emit and neither does reassigning `activeKey`, so
      // without the removal's own emit the final notification a hook received
      // still described two conversations — the alert and the blank run stayed
      // on screen until some unrelated update woke it.
      expect(snapshots.at(-1)).toEqual({ rows: 1, key: "prod-eu|Pod|ns|mongodb-0" });
      // One conversation — the one that was actually asked.
      const rows = getRunSummaries();
      expect(rows).toHaveLength(1);
      expect(rows[0].turns).toBeGreaterThan(0);
      // And the reader is left on it, not on a key that no longer resolves.
      expect(getAgentRun().turns.length).toBeGreaterThan(0);
    });

    /**
     * Codex P2, round 7: nothing bumps the generation when a turn simply
     * FINISHES, so the generation guard alone let a `cancelChat` rejecting
     * after the answer arrived put "That question was not sent" above a
     * question that visibly completed.
     */
    it("says nothing when a Stop loses the race with the answer it was stopping", async () => {
      let reject: ((e: unknown) => void) | undefined;
      cancelChat.mockImplementationOnce(() => new Promise<void>((_, rej) => (reject = rej)));
      // The answer lands while the cancel is still outstanding.
      sendChat.mockImplementation(async () => {
        stopAgentRun();
        return null;
      });

      await askAgent("why is it restarting");
      await vi.waitFor(() => {
        expect(reject).toBeDefined();
      });
      // The turn is over and its generation never moved.
      expect(getAgentRun().busy).toBe(false);

      reject?.(new Error("no such turn"));
      for (let i = 0; i < 50; i++) await Promise.resolve();

      expect(getAgentRun().error).toBeUndefined();
    });

    it("does not blame a new question for the previous Stop's failure", async () => {
      let reject: ((e: unknown) => void) | undefined;
      cancelChat.mockImplementationOnce(() => new Promise<void>((_, rej) => (reject = rej)));
      sendChat.mockImplementation(() => new Promise<string | null>(() => {}));

      void askAgent("the first one");
      await untilSendChatCalledTimes(1);
      stopAgentRun();
      await vi.waitFor(() => {
        expect(reject).toBeDefined();
      });

      // The reader moves on before the cancel comes back.
      clearAgentRun();
      sendChat.mockResolvedValue(null);
      await askAgent("a fresh question");

      reject?.(new Error("cancel failed"));
      for (let i = 0; i < 50; i++) await Promise.resolve();

      // The new conversation is not carrying the old Stop's failure.
      expect(getAgentRun().error).toBeUndefined();
    });

    it("does not send a question stopped before its session existed, even after a replacement", async () => {
      let release: ((s: string) => void) | undefined;
      startChat.mockImplementationOnce(() => new Promise<string>((res) => (release = res)));

      void askAgent("the abandoned one");
      await vi.waitFor(() => {
        expect(release).toBeDefined();
      });

      // Stop with no session yet: recorded as a stop, generation untouched.
      stopAgentRun();
      // ...and immediately a replacement, which resets the marker.
      startChat.mockResolvedValue("sess-2");
      await askAgent("the replacement");

      // Now the first `startChat` comes back.
      release?.("sess-1");
      // Flushed HARD. The abandoned continuation has three more awaits to get
      // through (`loadSkillsGuidance`, `listAgents`, then the send), and the
      // first draft of this test asserted after two microtasks — so it passed
      // with the whole fix reverted, which is the one thing a regression test
      // must not do.
      await untilSendChatCalledTimes(2);
      for (let i = 0; i < 50; i++) await Promise.resolve();

      const asked = sendChat.mock.calls.map((c) => c[1] as string);
      expect(asked).toContain("the replacement");
      expect(asked).not.toContain("the abandoned one");
    });

    it("leaves the generation alone when there was nothing in flight to abandon", async () => {
      const before = getAgentRun().generation;
      clearAgentRun();
      expect(getAgentRun().generation).toBe(before);
      expect(cancelChat).not.toHaveBeenCalled();
    });
  });

  // I3: `cancelChat`'s arguments had zero coverage anywhere in the package —
  // every existing test only ever `mockReset`/`mockResolvedValue`d it. The
  // reviewer's own mutation, `run.generation` → `run.generation - 1`, passed
  // every store and Composer test there is.
  it("stopAgentRun asks cancelChat to stop the session and generation the run is actually on", async () => {
    let sawStop = false;
    sendChat.mockImplementation(async () => {
      stopAgentRun();
      sawStop = true;
      return null;
    });

    await askAgent("q");

    expect(sawStop).toBe(true);
    expect(cancelChat).toHaveBeenCalledWith("sess-1", 1);
  });

  it("stopAgentRun turns a failed cancel into a top-level error, described rather than raw", async () => {
    cancelChat.mockRejectedValueOnce(new Error("boom"));
    sendChat.mockImplementation(async () => {
      stopAgentRun();
      // Give the rejected `cancelChat`'s `.catch` a turn to land before the
      // question itself finishes.
      await Promise.resolve();
      await Promise.resolve();
      return null;
    });

    await askAgent("q");

    expect(getAgentRun().error).toBe(describeError(new Error("boom")).detail);
    expect(getAgentRun().turns.every((t) => t.role !== "error")).toBe(true);
  });

  it("a failure answering a question marks its own turn, and never touches the top-level error", async () => {
    sendChat.mockRejectedValueOnce(new Error("kaboom"));

    await askAgent("q");

    const run = getAgentRun();
    expect(run.error).toBeUndefined();
    expect(run.turns.at(-1)?.role).toBe("error");
    expect(run.turns.at(-1)?.text).toBe(describeError(new Error("kaboom")).detail);
  });

  /**
   * This test used to assert the opposite — that two overlapping `askAgent`
   * calls BOTH reach `sendChat`, and only the stale one's result is dropped.
   * The bot review on #392 showed that behaviour is not merely untidy but
   * unsupported: the backend keys child processes by session in a `HashMap`
   * and `insert`s (`assistant.rs:727`), so a second send on one session
   * replaces the first child handle and drops it without `kill_and_reap`,
   * which that file's own doc says leaves a zombie. The first CLI is then
   * untracked and uncancellable, because `chat_cancel` removes by session and
   * finds only the newer child.
   *
   * So the store refuses instead. The generation guards still matter — a
   * clear or a stop abandons a turn whose result can still land — and the
   * "New question, with a turn still in flight" block above is what covers
   * them now.
   */
  it("refuses a second question while one is still being answered, and says so", async () => {
    sendChat.mockImplementationOnce(() => new Promise<string | null>(() => {}));

    void askAgent("q1");
    await untilSendChatCalledTimes(1);
    expect(getAgentRun().busy).toBe(true);

    await askAgent("q2");

    // Not sent — one child per session, and the first is still using it.
    expect(sendChat).toHaveBeenCalledTimes(1);
    expect(getAgentRun().error).toMatch(/still answering/i);
    // And it did not land as a turn either: the reader's transcript must not
    // show a question srelens never asked.
    expect(getAgentRun().turns.filter((t) => t.text === "q2")).toEqual([]);
  });

  it("takes the next question once the turn in flight has finished", async () => {
    let resolveFirst!: (v: string | null) => void;
    sendChat
      .mockImplementationOnce(() => new Promise<string | null>((resolve) => { resolveFirst = resolve; }))
      .mockImplementationOnce(async () => null);

    const first = askAgent("q1");
    await untilSendChatCalledTimes(1);
    resolveFirst("live-token");
    await first;
    expect(getAgentRun().busy).toBe(false);

    // The refusal is about the turn in flight, not a latch that stays shut —
    // and the second question resumes the conversation the first established.
    await askAgent("q2");
    expect(sendChat).toHaveBeenCalledTimes(2);
    expect(sendChat.mock.calls.at(-1)?.[7]).toBe("live-token");
  });

  // I4: `agentKind` defaults to "claude" and `Composer` is the ONLY code that
  // reconciles it to something installed — but the dock never mounts
  // `Composer` (§F), so a reader who has only Codex installed (or configured
  // only the native srelens agent) and never opens `/agent` used to have
  // `askAgent` hand `sendChat` an empty path (`agents.find(...) ?? ""`),
  // which fails to spawn on every question until `/agent` is opened once.
  describe("agentKind reconciliation (askAgent resolves its own agent)", () => {
    it("falls back to the first available agent when agentKind names nothing installed, and records the choice", async () => {
      listAgents.mockResolvedValue([
        { kind: "codex", label: "Codex", available: true, path: "/codex", version: "1", installUrl: "", gated: false },
      ]);
      sendChat.mockImplementation(async () => null);

      await askAgent("q"); // store still defaults agentKind to "claude"

      expect(sendChat).toHaveBeenCalledWith(
        "sess-1",
        "q",
        "/codex",
        expect.any(Function),
        undefined,
        "codex",
        1,
        null,
      );
      // The picker agrees with what actually ran.
      expect(getAgentRun().agentKind).toBe("codex");
    });

    it("fails the turn, without calling sendChat at all, when no agent is available", async () => {
      listAgents.mockResolvedValue([]);

      await askAgent("q");

      expect(sendChat).not.toHaveBeenCalled();
      const run = getAgentRun();
      expect(run.busy).toBe(false);
      expect(run.turns.at(-1)?.role).toBe("error");
      expect(run.turns.at(-1)?.text).toMatch(/no agent is available/i);
    });

    it("does not offer a gated agent, even if it is the one agentKind already names", async () => {
      listAgents.mockResolvedValue([
        { kind: "claude", label: "Claude", available: true, path: "/c", version: "1", installUrl: "", gated: true },
        { kind: "codex", label: "Codex", available: true, path: "/codex", version: "1", installUrl: "", gated: false },
      ]);
      sendChat.mockImplementation(async () => null);

      await askAgent("q");

      expect(sendChat.mock.calls[0][2]).toBe("/codex");
      expect(getAgentRun().agentKind).toBe("codex");
    });
  });

  // G2: `opts.skills`'s transmitted-vs-recorded split, and its per-skill
  // fault tolerance, had no coverage of their own — every existing test
  // either omits `skills` or (in `Composer.test.tsx`) mocks `askAgent`
  // itself, which pins the call boundary but nothing behind it.
  describe("skill guidance (opts.skills)", () => {
    it("sends the bare question when no skill is active — never an empty guidance preamble", async () => {
      sendChat.mockImplementation(async () => null);
      await askAgent("q");
      expect(sendChat.mock.calls[0][1]).toBe("q");
      expect(loadSkill).not.toHaveBeenCalled();
    });

    it("prepends active skills' guidance to what sendChat sends, but never into the recorded turn's text", async () => {
      loadSkill.mockResolvedValue({ name: "rollout", description: "d", body: "Check the rollout history." });
      sendChat.mockImplementation(async () => null);

      await askAgent("why is checkout-api 5xx?", { skills: ["rollout"] });

      expect(sendChat).toHaveBeenCalledWith(
        "sess-1",
        "Apply these skills:\n\nCheck the rollout history.\n\nwhy is checkout-api 5xx?",
        "/c",
        expect.any(Function),
        undefined,
        "claude",
        1,
        null,
      );
      // The reader's own turn holds exactly what they typed — the guidance
      // block is TRANSMITTED, never RECORDED.
      expect(getAgentRun().turns[0].text).toBe("why is checkout-api 5xx?");
    });

    it("drops one skill's guidance on a loadSkill rejection, and still sends the turn with the other skills' guidance intact", async () => {
      loadSkill.mockImplementation(async (name: string) => {
        if (name === "broken") throw new Error("deleted from disk");
        return { name, description: "d", body: `${name} body` };
      });
      sendChat.mockImplementation(async () => null);

      await askAgent("q", { skills: ["broken", "ok-skill"] });

      expect(sendChat.mock.calls[0][1]).toBe("Apply these skills:\n\nok-skill body\n\nq");
      const run = getAgentRun();
      expect(run.busy).toBe(false);
      expect(run.turns.at(-1)?.role).not.toBe("error");
    });
  });

  // C1/Ruling S: one set, two writers (the composer's `/` menu, the rail's
  // switch — both call `setSkillActive` and nothing else), one reader
  // (`askAgent`, which falls back to it whenever a call omits `opts.skills`).
  describe("skill activation (setSkillActive)", () => {
    it("starts with no skill active", () => {
      expect(getAgentRun().activeSkills).toEqual([]);
    });

    it("activates a skill, idempotently", () => {
      const before = getAgentRun();
      setSkillActive("rollout", true);
      expect(getAgentRun().activeSkills).toEqual(["rollout"]);
      expect(getAgentRun()).not.toBe(before);

      const after = getAgentRun();
      setSkillActive("rollout", true);
      // Already active: no new snapshot, per `commit`'s own no-op guard.
      expect(getAgentRun()).toBe(after);
    });

    it("deactivates a skill, idempotently", () => {
      setSkillActive("rollout", true);
      const before = getAgentRun();
      setSkillActive("rollout", false);
      expect(getAgentRun().activeSkills).toEqual([]);
      expect(getAgentRun()).not.toBe(before);

      const after = getAgentRun();
      setSkillActive("rollout", false);
      // Already inactive: no new snapshot.
      expect(getAgentRun()).toBe(after);
    });

    it("is the ONE set two different writers reach — a skill set active by one call reads back active from either", () => {
      // Stands in for "the composer's pick" and "the rail's switch": neither
      // this module nor the test cares which caller made the call, which is
      // the entire point — there is exactly one place this fact lives.
      setSkillActive("rollout", true);
      expect(getAgentRun().activeSkills).toContain("rollout");
      setSkillActive("oom-triage", true);
      expect(getAgentRun().activeSkills).toEqual(["rollout", "oom-triage"]);
    });

    it("askAgent folds the store's active skills into its guidance when the caller passes none of its own", async () => {
      loadSkill.mockResolvedValue({ name: "rollout", description: "d", body: "Check the rollout history." });
      sendChat.mockImplementation(async () => null);
      setSkillActive("rollout", true);

      await askAgent("why is checkout-api 5xx?");

      expect(sendChat.mock.calls[0][1]).toBe(
        "Apply these skills:\n\nCheck the rollout history.\n\nwhy is checkout-api 5xx?",
      );
    });

    it("an explicit opts.skills still overrides the store, for a caller that has its own list in hand", async () => {
      loadSkill.mockImplementation(async (name: string) => ({ name, description: "d", body: `${name} body` }));
      sendChat.mockImplementation(async () => null);
      setSkillActive("rollout", true);

      await askAgent("q", { skills: ["oom-triage"] });

      expect(sendChat.mock.calls[0][1]).toBe("Apply these skills:\n\noom-triage body\n\nq");
    });
  });

  /**
   * The change the reader asked for after using it: "it opens the same agent
   * view from everywhere, same chat". A run is keyed by its SUBJECT now, so
   * asking from a StatefulSets list and from a pod's logs gives two
   * conversations rather than one that inherits whatever the last question
   * was about.
   */
  describe("one conversation per subject", () => {
    const LIST = { about: { cluster: "prod-eu", namespaces: ["m01-prod-04-dataservices"] }, route: "/k/statefulsets" };
    const POD = {
      about: { cluster: "prod-eu", namespace: "m01-cnips-01-services", kind: "Pod", name: "ai-editor", surface: "logs" as const },
      route: "/logs/Pod/m01-cnips-01-services/ai-editor",
    };

    it("keeps two subjects in two conversations", async () => {
      sendChat.mockResolvedValue(null);
      await askAgent("which replica set spiked?", LIST);
      await askAgent("summarise this stream", POD);

      const summaries = getRunSummaries();
      expect(summaries).toHaveLength(2);
      // Each holds only its own question.
      // Rows are labelled by the QUESTION now, with the subject beside it —
      // one naming scheme for live and saved rows alike.
      const list = summaries.find((r) => r.subject === "statefulsets")!;
      const pod = summaries.find((r) => r.subject === "Pod/ai-editor")!;
      expect(getRun(list.key).turns.map((t) => t.text)).toContain("which replica set spiked?");
      expect(getRun(list.key).turns.map((t) => t.text)).not.toContain("summarise this stream");
      expect(getRun(pod.key).turns.map((t) => t.text)).toContain("summarise this stream");
    });

    it("continues the same conversation when the subject is the same", async () => {
      sendChat.mockResolvedValue(null);
      await askAgent("first", LIST);
      await askAgent("second", LIST);
      expect(getRunSummaries()).toHaveLength(1);
      expect(getRunSummaries()[0].turns).toBe(2);
    });

    it("treats a pod's logs and that pod's detail as ONE subject", async () => {
      sendChat.mockResolvedValue(null);
      await askAgent("from logs", POD);
      await askAgent("from detail", {
        about: { cluster: "prod-eu", namespace: "m01-cnips-01-services", kind: "Pod", name: "ai-editor" },
        route: "/k/Pod/m01-cnips-01-services/ai-editor",
      });
      // Same subject, different lens — forking would split a conversation the
      // reader experiences as one.
      expect(getRunSummaries()).toHaveLength(1);
    });

    /**
     * Codex P1, round 3. The full view holds keys that cannot be recomputed —
     * a conversation aliased to `saved|<id>` while still carrying its original
     * subject, and a dock with no stored subject at all — so it names the run
     * outright. This is the seam: the console test pins that the key is
     * PASSED, and nothing pinned that it is used.
     */
    it("asks into the conversation the caller named, not one derived from the route", async () => {
      sendChat.mockResolvedValue(null);
      await askAgent("a follow-up", {
        key: "saved|older-file",
        about: { cluster: "prod-eu", namespace: "ns", kind: "Pod", name: "mongodb-0" },
        route: "/k/pods/ns/mongodb-0",
      });

      expect(getActiveRunKey()).toBe("saved|older-file");
      expect(getRun("saved|older-file").turns.map((t) => t.text)).toContain("a follow-up");
      // And NOT under the key its subject would have produced, which is the
      // run the derivation reached.
      expect(getRun("prod-eu|Pod|ns|mongodb-0").turns).toEqual([]);
      // The preface still carries the resource, so naming the key does not
      // cost the conversation its context.
      expect(sendChat.mock.calls.at(-1)?.[1]).toMatch(/namespace ns, Pod mongodb-0/);
    });

    it("does not fork when the reader only re-narrows the namespace picker", async () => {
      sendChat.mockResolvedValue(null);
      await askAgent("first", LIST);
      await askAgent("second", {
        about: { cluster: "prod-eu", namespaces: ["m01-prod-05-dataservices"] },
        route: "/k/statefulsets",
      });
      // Narrowing is a filter adjusted while thinking, not a new subject.
      expect(getRunSummaries()).toHaveLength(1);
    });

    it("keeps the same subject in two clusters apart", async () => {
      sendChat.mockResolvedValue(null);
      await askAgent("here", POD);
      await askAgent("there", { ...POD, about: { ...POD.about, cluster: "staging" } });
      expect(getRunSummaries()).toHaveLength(2);
    });

    it("creates a run by ASKING, never by navigating", () => {
      // Browsing thirty screens must not leave thirty empty chats in the rail.
      expect(getRunSummaries()).toEqual([]);
      expect(getActiveRunKey()).toBeNull();
    });

    it("refuses a second question while another subject is still answering, and says which", async () => {
      sendChat.mockImplementation(() => new Promise<string | null>(() => {}));
      void askAgent("long one", POD);
      await untilSendChatCalledTimes(1);

      await askAgent("another", LIST);
      // Named, so the reader knows what to go and stop.
      expect(getAgentRun().error).toMatch(/Pod\/ai-editor/);
      expect(sendChat).toHaveBeenCalledTimes(1);
    });

    it("switches which conversation the surfaces show", async () => {
      sendChat.mockResolvedValue(null);
      await askAgent("a", LIST);
      await askAgent("b", POD);
      const [newest, older] = getRunSummaries();
      expect(getActiveRunKey()).toBe(newest.key);
      selectRun(older.key);
      expect(getActiveRunKey()).toBe(older.key);
      expect(getAgentRun().turns.map((t) => t.text)).toContain("a");
    });

    it("forgets a conversation, but never one that is still answering", async () => {
      sendChat.mockImplementation(() => new Promise<string | null>(() => {}));
      void askAgent("streaming", POD);
      await untilSendChatCalledTimes(1);
      const key = getRunSummaries()[0].key;

      forgetRun(key);
      // Dropping the state a turn is writing into would leave its sendChat
      // with nowhere to land and its child untracked.
      expect(getRunSummaries()).toHaveLength(1);
    });
  });

  /**
   * "Conversations are still not showing up" — because nothing was persisted
   * (#395). Every conversation now survives a restart.
   */
  it("sends raw base64, not the data URI it records", async () => {
    // `chat_send` hands images to `decode_base64_image`, which is
    // `STANDARD.decode` (`assistant.rs:245-249`) — it does not strip a
    // `data:image/png;base64,` prefix, it fails on it. So the URI is what
    // srelens SHOWS and the payload is what srelens SENDS.
    sendChat.mockResolvedValue(null);
    await askAgent("what is this", {
      about: { cluster: "prod-eu" },
      route: "/",
      images: ["data:image/png;base64,AAAB"],
    });
    expect(sendChat.mock.calls.at(-1)?.[4]).toEqual(["AAAB"]);
    // The turn keeps the displayable form, so the transcript can draw it.
    expect(getAgentRun().turns.find((t) => t.role === "user")?.images).toEqual([
      "data:image/png;base64,AAAB",
    ]);
  });

  describe("conversations survive the window", () => {
    const POD = {
      about: { cluster: "prod-eu", namespace: "ns", kind: "Pod", name: "mongodb-0" },
      route: "/k/Pod/ns/mongodb-0",
    };

    it("saves the reader's question BEFORE the answer, so an interrupted turn keeps it", async () => {
      // Never resolves: the turn is still in flight when the assertion runs.
      sendChat.mockImplementation(() => new Promise<string | null>(() => {}));
      void askAgent("why is it restarting?", POD);
      await untilSendChatCalledTimes(1);

      expect(saveSession).toHaveBeenCalled();
      const saved = saveSession.mock.calls.at(-1)?.[0] as { messages: unknown[]; title: string };
      const envelope = saved.messages[0] as { v: number; key: string; turns: { text: string }[] };
      expect(envelope.v).toBe(1);
      expect(envelope.turns.map((t) => t.text)).toContain("why is it restarting?");
      // The title is the question TIDIED — same derivation the rail uses, so a
      // conversation reads the same live or restored. Raised first letter,
      // opener dropped, cut at a word boundary.
      expect(saved.title).toBe("Why is it restarting?");
    });

    it("saves again once the answer has landed", async () => {
      sendChat.mockResolvedValue("cli-conversation-id");
      await askAgent("q", POD);
      // At least twice: once with the question, once with the settled turn.
      expect(saveSession.mock.calls.length).toBeGreaterThanOrEqual(2);
      const last = saveSession.mock.calls.at(-1)?.[0] as { cliSessionId: string | null };
      // The CLI's own conversation id travels with it, so a reopened run can
      // resume rather than start over.
      expect(last.cliSessionId).toBe("cli-conversation-id");
    });

    it("lists a conversation once, not twice, when it is both live and on disk", async () => {
      // A run asked in this window is written immediately, so the next
      // `listSessions` sees it. Without deduping by file id it appeared twice
      // — once live, once as its own saved copy, under two different names.
      sendChat.mockResolvedValue(null);
      await askAgent("what is mongodb using?", POD);
      const id = (saveSession.mock.calls.at(-1)?.[0] as { id: string }).id;

      listSessions.mockResolvedValue([
        { id, title: "what is mongodb using?", createdAt: 1, updatedAt: 2 },
      ]);
      await restoreRuns();

      expect(getRunSummaries()).toHaveLength(1);
      expect(getRunSummaries()[0].savedId).toBeUndefined();
    });

    it("writes nothing for a run nobody asked anything in", async () => {
      sendChat.mockResolvedValue(null);
      // Selecting a subject is not a conversation.
      selectRun("prod-eu|/overview");
      expect(saveSession).not.toHaveBeenCalled();
    });

    it("lists what is on disk, and opens one into a real run", async () => {
      listSessions.mockResolvedValue([{ id: "s1", title: "check mongodb", createdAt: 1, updatedAt: 2 }]);
      loadSession.mockResolvedValue({
        id: "s1",
        title: "check mongodb",
        createdAt: 1,
        updatedAt: 2,
        contexts: [],
        skills: [],
        cliSessionId: "cli-1",
        agentKind: "claude",
        messages: [
          {
            v: 1,
            key: "prod-eu|Pod|ns|mongodb-0",
            label: "Pod/mongodb-0",
            turns: [{ id: 1, role: "user", text: "check mongodb", calls: [], at: 1 }],
            gates: [],
          },
        ],
      });

      await restoreRuns();
      const row = getRunSummaries().find((r) => r.savedId === "s1")!;
      expect(row.label).toBe("check mongodb");

      await openSavedRun("s1");
      expect(getAgentRun().turns.map((t) => t.text)).toContain("check mongodb");
      // Under its own subject key, so the next question about that pod
      // continues this conversation rather than starting a second one.
      expect(getActiveRunKey()).toBe("prod-eu|Pod|ns|mongodb-0");
      // And no longer listed as merely on disk.
      expect(getRunSummaries().filter((r) => r.savedId === "s1")).toEqual([]);
    });

    /**
     * "Shows empty for old ones." Those sessions were written by CLASSIC, in
     * its own `StoredMessage` shape, and `openSavedRun` looked only for this
     * build's envelope — so the reader's own conversations opened blank.
     */
    it("reads a conversation classic saved, rather than opening it empty", async () => {
      listSessions.mockResolvedValue([{ id: "old", title: "check cluster health", createdAt: 1, updatedAt: 5 }]);
      loadSession.mockResolvedValue({
        id: "old",
        title: "check cluster health",
        createdAt: 1,
        updatedAt: 5,
        contexts: ["prod-eu"],
        skills: [],
        cliSessionId: null,
        agentKind: "claude",
        // Classic's shape: `assistant`, not `agent`, and tool calls embedded.
        messages: [
          { id: 1, role: "user", text: "check cluster health" },
          {
            id: 2,
            role: "assistant",
            text: "Three nodes are under memory pressure.",
            toolCalls: [{ id: "c1", tool: "k8s.listNodes", args: {}, status: "ok" }],
            thoughts: "looking at allocatable",
          },
        ],
      });

      await restoreRuns();
      await openSavedRun("old");

      const turns = getAgentRun().turns;
      expect(turns.map((t) => t.text)).toEqual([
        "check cluster health",
        "Three nodes are under memory pressure.",
      ]);
      // `assistant` is this design's `agent`.
      expect(turns[1].role).toBe("agent");
      expect(turns[1].calls[0]?.tool).toBe("k8s.listNodes");
      expect(turns[1].thoughts).toBe("looking at allocatable");
    });

    it("does not claim a time classic never stored", async () => {
      listSessions.mockResolvedValue([{ id: "old", title: "t", createdAt: 1, updatedAt: 5 }]);
      loadSession.mockResolvedValue({
        id: "old", title: "t", createdAt: 1, updatedAt: 5, contexts: [], skills: [],
        cliSessionId: null, agentKind: "claude",
        messages: [{ id: 1, role: "user", text: "q" }],
      });
      await restoreRuns();
      await openSavedRun("old");
      // `StoredMessage` carries no timestamp, so the turn is marked as having
      // none and the transcript withholds its clock — rather than printing the
      // session's own stamp under every turn as if it were theirs.
      expect(getAgentRun().turns[0].atRecorded).toBe(false);
      // `createdAt`, not `updatedAt`: `/agent`'s head reads the first turn's
      // stamp as `started <time>`, and 5 is when this conversation was last
      // touched — printing it as the start would be false.
      expect(getAgentRun().turns[0].at).toBe(1);
    });

    it("ignores an entry in a shape it does not recognise, without losing the rest", async () => {
      listSessions.mockResolvedValue([{ id: "old", title: "t", createdAt: 1, updatedAt: 5 }]);
      loadSession.mockResolvedValue({
        id: "old", title: "t", createdAt: 1, updatedAt: 5, contexts: [], skills: [],
        cliSessionId: null, agentKind: "claude",
        messages: [{ nonsense: true }, { id: 1, role: "user", text: "the real one" }],
      });
      await restoreRuns();
      await openSavedRun("old");
      expect(getAgentRun().turns.map((t) => t.text)).toEqual(["the real one"]);
    });

    /**
     * Codex P2, round 5: `isSavedRun` checked only that `turns` was an array,
     * so a truncated or hand-edited file whose turn lacked `calls` was assigned
     * straight to the run — and `Transcript` reads `turn.calls.length` the
     * moment it renders, taking the agent screen down.
     */
    it("does not hydrate an envelope whose turns are malformed", async () => {
      listSessions.mockResolvedValue([{ id: "bad", title: "t", createdAt: 1, updatedAt: 2 }]);
      loadSession.mockResolvedValue({
        id: "bad", title: "t", createdAt: 1, updatedAt: 2, contexts: [], skills: [],
        cliSessionId: null, agentKind: "claude",
        messages: [{
          v: 1,
          key: "prod-eu|/k/pods",
          label: "pods",
          // A turn with no `calls` — what a write truncated mid-flush leaves.
          turns: [{ id: 1, role: "user", text: "q", at: 1 }],
          gates: [],
        }],
      });
      await restoreRuns();
      await openSavedRun("bad");
      // Opened empty rather than crashing the screen: every turn the transcript
      // is handed can be rendered.
      expect(getAgentRun().turns).toEqual([]);
    });

    /**
     * Codex P2, round 8: the required fields were checked last round and the
     * OPTIONAL ones were not, so `images: "broken"` passed and `UserTurn` then
     * called `turn.images.map`.
     */
    /**
     * Codex P1, round 9: the classic branch restored the transcript and the CLI
     * resume token and IGNORED `session.contexts`, which is where classic put
     * the cluster. Opening a `prod-eu` conversation while the workspace was on
     * `staging` therefore displayed staging, sent a staging preface, and
     * resumed the prod CLI conversation underneath.
     */
    it("restores the cluster a classic conversation was had on", async () => {
      listSessions.mockResolvedValue([{ id: "old", title: "t", createdAt: 1, updatedAt: 2 }]);
      loadSession.mockResolvedValue({
        id: "old", title: "t", createdAt: 1, updatedAt: 2,
        // Classic's own field, one entry per attached context.
        contexts: ["prod-eu"],
        skills: [], cliSessionId: "claude-token", agentKind: "claude",
        messages: [{ id: 1, role: "user", text: "check the mongodb replica set" }],
      });
      await restoreRuns();
      await openSavedRun("old");

      const key = getActiveRunKey();
      expect(getRunSubject(key)?.about.cluster).toBe("prod-eu");
    });

    it("names no cluster for a classic conversation that spanned several", async () => {
      // Classic's multi-context mode wrote several, and `AskContext.cluster` is
      // one string — there is no honest single answer, so nothing is invented.
      // A real remaining gap rather than a handled case.
      listSessions.mockResolvedValue([{ id: "multi", title: "t", createdAt: 1, updatedAt: 2 }]);
      loadSession.mockResolvedValue({
        id: "multi", title: "t", createdAt: 1, updatedAt: 2,
        contexts: ["prod-eu", "staging-eu"],
        skills: [], cliSessionId: null, agentKind: "claude",
        messages: [{ id: 1, role: "user", text: "compare them" }],
      });
      await restoreRuns();
      await openSavedRun("multi");
      expect(getRunSubject(getActiveRunKey())).toBeUndefined();
    });

    it("does not hydrate an envelope whose subject is malformed", async () => {
      // `shown.about.cluster` is read DURING RENDER in the full view, so a
      // `subject: {}` took the agent screen down.
      listSessions.mockResolvedValue([{ id: "bad", title: "t", createdAt: 1, updatedAt: 2 }]);
      loadSession.mockResolvedValue({
        id: "bad", title: "t", createdAt: 1, updatedAt: 2, contexts: [], skills: [],
        cliSessionId: null, agentKind: "claude",
        messages: [{
          v: 1, key: "prod-eu|/k/pods", label: "pods",
          turns: [{ id: 1, role: "user", text: "q", calls: [], at: 1 }],
          gates: [],
          subject: {},
        }],
      });
      await restoreRuns();
      await openSavedRun("bad");
      expect(getAgentRun().turns).toEqual([]);
    });

    it("hydrates an envelope whose subject is well formed", async () => {
      listSessions.mockResolvedValue([{ id: "good", title: "t", createdAt: 1, updatedAt: 2 }]);
      loadSession.mockResolvedValue({
        id: "good", title: "t", createdAt: 1, updatedAt: 2, contexts: [], skills: [],
        cliSessionId: null, agentKind: "claude",
        messages: [{
          v: 1, key: "prod-eu|Pod|ns|mongodb-0", label: "Pod/mongodb-0",
          turns: [{ id: 1, role: "user", text: "q", calls: [], at: 1 }],
          gates: [],
          subject: {
            about: { cluster: "prod-eu", namespace: "ns", kind: "Pod", name: "mongodb-0" },
            route: "/k/pods/ns/mongodb-0",
          },
        }],
      });
      await restoreRuns();
      await openSavedRun("good");
      expect(getAgentRun().turns.map((t) => t.text)).toEqual(["q"]);
      expect(getRunSubject(getActiveRunKey())?.about.cluster).toBe("prod-eu");
    });

    it("does not hydrate an envelope whose optional fields are the wrong shape", async () => {
      for (const bad of [
        { images: "broken" },
        { notes: 3 },
        { thoughts: { why: "no" } },
        { images: [1, 2] },
      ]) {
        resetAgentRun();
        listSessions.mockResolvedValue([{ id: "bad", title: "t", createdAt: 1, updatedAt: 2 }]);
        loadSession.mockResolvedValue({
          id: "bad", title: "t", createdAt: 1, updatedAt: 2, contexts: [], skills: [],
          cliSessionId: null, agentKind: "claude",
          messages: [{
            v: 1,
            key: "prod-eu|/k/pods",
            label: "pods",
            turns: [{ id: 1, role: "user", text: "q", calls: [], at: 1, ...bad }],
            gates: [],
          }],
        });
        await restoreRuns();
        await openSavedRun("bad");
        expect(getAgentRun().turns, `accepted ${JSON.stringify(bad)}`).toEqual([]);
      }
    });

    it("hydrates an envelope whose optional fields are well formed", async () => {
      // The presence half, so tightening this until it accepts nothing would
      // not pass: absent and well-formed are both fine.
      listSessions.mockResolvedValue([{ id: "good", title: "t", createdAt: 1, updatedAt: 2 }]);
      loadSession.mockResolvedValue({
        id: "good", title: "t", createdAt: 1, updatedAt: 2, contexts: [], skills: [],
        cliSessionId: null, agentKind: "claude",
        messages: [{
          v: 1,
          key: "prod-eu|/k/pods",
          label: "pods",
          turns: [
            { id: 1, role: "user", text: "q", calls: [], at: 1, images: ["data:image/png;base64,AA"] },
            { id: 2, role: "agent", text: "a", calls: [], at: 2, notes: ["a warning"], thoughts: "hmm" },
          ],
          gates: [],
        }],
      });
      await restoreRuns();
      await openSavedRun("good");
      expect(getAgentRun().turns.map((t) => t.text)).toEqual(["q", "a"]);
      expect(getAgentRun().turns[0].images).toEqual(["data:image/png;base64,AA"]);
    });

    it("does not hydrate an envelope whose gates are not a list", async () => {
      listSessions.mockResolvedValue([{ id: "bad", title: "t", createdAt: 1, updatedAt: 2 }]);
      loadSession.mockResolvedValue({
        id: "bad", title: "t", createdAt: 1, updatedAt: 2, contexts: [], skills: [],
        cliSessionId: null, agentKind: "claude",
        messages: [{
          v: 1,
          key: "prod-eu|/k/pods",
          label: "pods",
          turns: [{ id: 1, role: "user", text: "q", calls: [], at: 1 }],
          // `Transcript` calls `gates.map`.
          gates: "not a list",
        }],
      });
      await restoreRuns();
      await openSavedRun("bad");
      expect(getAgentRun().turns).toEqual([]);
      expect(getAgentRun().gates).toEqual([]);
    });

    it("hydrates an envelope whose turns are all well formed", async () => {
      // The presence half. Without it, tightening the predicate until it
      // accepts nothing would pass both tests above.
      listSessions.mockResolvedValue([{ id: "good", title: "t", createdAt: 1, updatedAt: 2 }]);
      loadSession.mockResolvedValue({
        id: "good", title: "t", createdAt: 1, updatedAt: 2, contexts: [], skills: [],
        cliSessionId: null, agentKind: "claude",
        messages: [{
          v: 1,
          key: "prod-eu|/k/pods",
          label: "pods",
          turns: [
            { id: 1, role: "user", text: "q", calls: [], at: 1 },
            { id: 2, role: "agent", text: "a", calls: [{ id: "c", tool: "k8s.listPods" }], at: 2 },
          ],
          gates: [{ id: "g", tool: "k8s.scale" }],
        }],
      });
      await restoreRuns();
      await openSavedRun("good");
      expect(getAgentRun().turns.map((t) => t.text)).toEqual(["q", "a"]);
      expect(getAgentRun().gates).toHaveLength(1);
    });

    it("drops a stored tool call it cannot name, keeping the ones it can", async () => {
      listSessions.mockResolvedValue([{ id: "old", title: "t", createdAt: 1, updatedAt: 5 }]);
      loadSession.mockResolvedValue({
        id: "old", title: "t", createdAt: 1, updatedAt: 5, contexts: [], skills: [],
        cliSessionId: null, agentKind: "claude",
        messages: [
          {
            id: 1,
            role: "assistant",
            text: "done",
            toolCalls: [
              { id: "a", tool: "", status: "ok" },
              { id: "b" },
              { id: "c", tool: "k8s.listPods", args: {}, status: "ok" },
            ],
          },
        ],
      });
      await restoreRuns();
      await openSavedRun("old");
      // A row with no capability name reads as a call srelens made and cannot
      // name — worse than not drawing it.
      expect(getAgentRun().turns[0].calls.map((c) => c.tool)).toEqual(["k8s.listPods"]);
    });

    /**
     * Codex P1. The reader asks about a subject after a restart, then opens the
     * OLDER saved conversation about that same subject. Reusing the live run
     * replaced its turns and gates while keeping its id, resume token, save
     * chain and busy state — so a streaming answer's deltas could no longer
     * find their turn, and the saved transcript was persisted under the live
     * conversation's file.
     */
    it("opens a saved conversation beside a live one about the same subject, not into it", async () => {
      sendChat.mockResolvedValue(null);
      // A live conversation about the pod, asked in this window.
      await askAgent("what is it doing now", POD);
      const liveKey = getActiveRunKey();
      const liveTurns = getRun(liveKey).turns.map((t) => t.text);

      // An OLDER saved conversation about the same pod, under its own file.
      listSessions.mockResolvedValue([{ id: "older", title: "t", createdAt: 1, updatedAt: 2 }]);
      loadSession.mockResolvedValue({
        id: "older", title: "t", createdAt: 1, updatedAt: 2, contexts: [], skills: [],
        cliSessionId: null, agentKind: "claude",
        messages: [{ v: 1, key: liveKey, label: "Pod/mongodb-0", turns: [
          { id: 99, role: "user", text: "the older question", calls: [], at: 1 },
        ], gates: [] }],
      });
      await restoreRuns();
      await openSavedRun("older");

      // The saved one is what is open...
      expect(getAgentRun().turns.map((t) => t.text)).toEqual(["the older question"]);
      // ...and the live conversation is untouched, still under its own key.
      expect(getRun(liveKey).turns.map((t) => t.text)).toEqual(liveTurns);
      expect(getActiveRunKey()).not.toBe(liveKey);
    });

    /**
     * Codex P2. `openSavedRun` restored the file's resume token and set the
     * RUN's agent, but `askAgent` read the module-level kind — so a Codex
     * conversation reopened after restart handed its Codex token to Claude
     * while the picker, which shows `run.agentKind`, still said Codex.
     */
    /**
     * Codex P2. Two clicks before the first `loadSession` returned and both
     * continuations assigned `activeKey`, so the slower one won — the
     * transcript switched back after the reader had already opened the other.
     */
    it("opens the conversation clicked last, whichever load finishes first", async () => {
      listSessions.mockResolvedValue([
        { id: "slow", title: "slow", createdAt: 1, updatedAt: 2 },
        { id: "quick", title: "quick", createdAt: 1, updatedAt: 3 },
      ]);
      await restoreRuns();

      let releaseSlow: ((v: unknown) => void) | undefined;
      loadSession.mockImplementationOnce(
        () => new Promise((res) => (releaseSlow = res)),
      );
      const slow = openSavedRun("slow");

      loadSession.mockResolvedValueOnce({
        id: "quick", title: "quick", createdAt: 1, updatedAt: 3, contexts: [], skills: [],
        cliSessionId: null, agentKind: "claude",
        messages: [{ v: 1, key: "k|quick", label: "quick", turns: [
          { id: 1, role: "user", text: "the one clicked second", calls: [], at: 1 },
        ], gates: [] }],
      });
      await openSavedRun("quick");

      // Now the first click's load finally comes back.
      releaseSlow?.({
        id: "slow", title: "slow", createdAt: 1, updatedAt: 2, contexts: [], skills: [],
        cliSessionId: null, agentKind: "claude",
        messages: [{ v: 1, key: "k|slow", label: "slow", turns: [
          { id: 2, role: "user", text: "the one clicked first", calls: [], at: 1 },
        ], gates: [] }],
      });
      await slow;

      expect(getAgentRun().turns.map((t) => t.text)).toEqual(["the one clicked second"]);
    });

    /**
     * Codex P2, round 3. After `askAgent` began reading the RUN's agent, a
     * reopened conversation could show Codex while the module preference was
     * still Claude — so picking Claude in the picker compared against the
     * module value, returned early, and left the Codex resume token in place.
     * A reader whose restored CLI is no longer installed had no way off it.
     */
    it("switches a reopened conversation off the CLI it was saved with", async () => {
      listSessions.mockResolvedValue([{ id: "cdx", title: "t", createdAt: 1, updatedAt: 2 }]);
      loadSession.mockResolvedValue({
        id: "cdx", title: "t", createdAt: 1, updatedAt: 2, contexts: [], skills: [],
        cliSessionId: "codex-token", agentKind: "codex",
        messages: [{ v: 1, key: "prod-eu|/k/pods", label: "pods", turns: [
          { id: 1, role: "user", text: "earlier", calls: [], at: 1 },
        ], gates: [] }],
      });
      await restoreRuns();
      await openSavedRun("cdx");
      // The module preference never moved; the run shows the file's CLI.
      expect(getAgentRun().agentKind).toBe("codex");

      chooseAgent("claude");

      // Not a no-op: the conversation is on Claude now...
      expect(getAgentRun().agentKind).toBe("claude");
      // ...and the other CLI's resume token went with the switch, which is the
      // whole reason `chooseAgent` drops them (ruling AC).
      listAgents.mockResolvedValue([
        { kind: "claude", label: "Claude", available: true, gated: false, path: "/claude", version: "1", installUrl: "" },
      ]);
      sendChat.mockResolvedValue(null);
      await askAgent("carry on", { about: { cluster: "prod-eu" }, route: "/k/pods" });
      expect(sendChat.mock.calls.at(-1)?.[7]).toBeNull();
    });

    /**
     * Codex P2, round 4: `openSeq` was only ever advanced by another saved-row
     * click, so selecting an already-loaded conversation while a load was in
     * flight left that load free to switch the transcript back when it landed.
     */
    it("lets an explicit selection outrank a load still in flight", async () => {
      sendChat.mockResolvedValue(null);
      // A live conversation to switch to.
      await askAgent("the live one", { about: { cluster: "prod-eu" }, route: "/k/pods" });
      const liveKey = getActiveRunKey();

      listSessions.mockResolvedValue([{ id: "slow", title: "slow", createdAt: 1, updatedAt: 2 }]);
      await restoreRuns();
      let release: ((v: unknown) => void) | undefined;
      loadSession.mockImplementationOnce(() => new Promise((res) => (release = res)));
      const slow = openSavedRun("slow");
      await vi.waitFor(() => {
        expect(release).toBeDefined();
      });

      // The reader gives up waiting and picks the conversation already loaded.
      selectRun(liveKey);

      release?.({
        id: "slow", title: "slow", createdAt: 1, updatedAt: 2, contexts: [], skills: [],
        cliSessionId: null, agentKind: "claude",
        messages: [{ v: 1, key: "k|slow", label: "slow", turns: [
          { id: 9, role: "user", text: "the slow one", calls: [], at: 1 },
        ], gates: [] }],
      });
      await slow;

      expect(getActiveRunKey()).toBe(liveKey);
      expect(getAgentRun().turns.map((t) => t.text)).toContain("the live one");
    });

    /**
     * Codex P2, round 5: the dock is keyed by its own route, which off `/agent`
     * need not be the ACTIVE run — so the picker can render a restored Codex
     * conversation while `activeKey` points at a Claude one, and comparing the
     * active run made the pick a no-op.
     */
    it("switches the run whose picker was used, not whichever is active", async () => {
      sendChat.mockResolvedValue(null);
      // A Claude conversation, and it is the active one.
      await askAgent("the active one", { about: { cluster: "prod-eu" }, route: "/k/pods" });
      const activeKeyNow = getActiveRunKey();

      // A second conversation restored from a Codex file, NOT selected: its
      // load leaves `activeKey` on itself, so put the first back in front.
      listSessions.mockResolvedValue([{ id: "cdx", title: "t", createdAt: 1, updatedAt: 2 }]);
      loadSession.mockResolvedValue({
        id: "cdx", title: "t", createdAt: 1, updatedAt: 2, contexts: [], skills: [],
        cliSessionId: "codex-token", agentKind: "codex",
        messages: [{ v: 1, key: "prod-eu|/k/statefulsets", label: "statefulsets", turns: [
          { id: 1, role: "user", text: "earlier", calls: [], at: 1 },
        ], gates: [] }],
      });
      await restoreRuns();
      await openSavedRun("cdx");
      selectRun(activeKeyNow);
      expect(getRun("prod-eu|/k/statefulsets").agentKind).toBe("codex");

      // The reader is on another tab, whose dock shows the Codex conversation,
      // and picks Claude there.
      chooseAgent("claude", "prod-eu|/k/statefulsets");

      // Not a no-op, even though both the module and the ACTIVE run already
      // said Claude.
      expect(getRun("prod-eu|/k/statefulsets").agentKind).toBe("claude");
    });

    it("asks the agent the reopened conversation belongs to", async () => {
      listSessions.mockResolvedValue([{ id: "cdx", title: "t", createdAt: 1, updatedAt: 2 }]);
      loadSession.mockResolvedValue({
        id: "cdx", title: "t", createdAt: 1, updatedAt: 2, contexts: [], skills: [],
        cliSessionId: "codex-token", agentKind: "codex",
        messages: [{ v: 1, key: "prod-eu|/k/pods", label: "pods", turns: [
          { id: 1, role: "user", text: "earlier", calls: [], at: 1 },
        ], gates: [] }],
      });
      listAgents.mockResolvedValue([
        { kind: "claude", label: "Claude", available: true, gated: false, path: "/claude", version: "1", installUrl: "" },
        { kind: "codex", label: "Codex", available: true, gated: false, path: "/codex", version: "1", installUrl: "" },
      ]);
      sendChat.mockResolvedValue(null);

      await restoreRuns();
      await openSavedRun("cdx");
      expect(getAgentRun().agentKind).toBe("codex");

      await askAgent("carry on", { about: { cluster: "prod-eu" }, route: "/k/pods" });
      // The path of the agent this conversation belongs to — not the window's
      // default, which would have received the other CLI's resume token.
      expect(sendChat.mock.calls.at(-1)?.[2]).toBe("/codex");
    });

    /**
     * Codex P2, round 6: `getRunSummaries` hides a persisted file only while
     * its id belongs to a live run, so rotating the live id on clear while
     * `saved` still held the old one made the conversation the reader just
     * cleared reappear immediately as a saved row.
     */
    it("does not list a cleared conversation as one still on disk", async () => {
      sendChat.mockResolvedValue(null);
      await askAgent("something to clear", { about: { cluster: "prod-eu" }, route: "/k/pods" });
      // The file this window wrote is now on the index, as it is after a
      // restart or any other `listSessions`.
      const id = getRunSummaries()[0]?.key;
      expect(id).toBeDefined();
      listSessions.mockResolvedValue([
        { id: (await saveSession.mock.calls.at(-1)?.[0])?.id ?? "", title: "t", createdAt: 1, updatedAt: 2 },
      ]);
      await restoreRuns();

      clearAgentRun();

      // Gone, not moved to the saved list under its dead id.
      expect(getRunSummaries()).toEqual([]);
    });

    /**
     * Codex P2, round 8: `listSessions` can be issued BEFORE a clear and
     * answered after it, so assigning the response wholesale put the cleared
     * conversation back as a saved row — which then opened with a load error
     * once the delete landed.
     */
    it("does not let a listing in flight reinstate a conversation just cleared", async () => {
      sendChat.mockResolvedValue(null);
      await askAgent("something to clear", { about: { cluster: "prod-eu" }, route: "/k/pods" });
      const fileId = (await saveSession.mock.calls.at(-1)?.[0])?.id ?? "";

      // A listing issued now, answered later — with the file still in it.
      let release: ((v: unknown) => void) | undefined;
      listSessions.mockImplementationOnce(() => new Promise((res) => (release = res)));
      const listing = restoreRuns();
      await vi.waitFor(() => {
        expect(release).toBeDefined();
      });

      clearAgentRun();

      release?.([{ id: fileId, title: "t", createdAt: 1, updatedAt: 2 }]);
      await listing;

      // Still gone. The response named a file this window had already deleted.
      expect(getRunSummaries()).toEqual([]);
    });

    it("does not list a forgotten conversation as one still on disk either", async () => {
      // The same class as the clear above, at the other site that deletes a
      // file. Fixed together rather than at the one that was reported.
      sendChat.mockResolvedValue(null);
      await askAgent("something to forget", { about: { cluster: "prod-eu" }, route: "/k/pods" });
      const key = getActiveRunKey() ?? "";
      listSessions.mockResolvedValue([
        { id: (await saveSession.mock.calls.at(-1)?.[0])?.id ?? "", title: "t", createdAt: 1, updatedAt: 2 },
      ]);
      await restoreRuns();

      forgetRun(key);

      expect(getRunSummaries()).toEqual([]);
    });

    it("resumes the CLI conversation a reopened run came with", async () => {
      listSessions.mockResolvedValue([{ id: "s1", title: "t", createdAt: 1, updatedAt: 2 }]);
      loadSession.mockResolvedValue({
        id: "s1", title: "t", createdAt: 1, updatedAt: 2, contexts: [], skills: [],
        cliSessionId: "cli-1", agentKind: "claude",
        messages: [{ v: 1, key: "prod-eu|Pod|ns|mongodb-0", label: "Pod/mongodb-0", turns: [], gates: [] }],
      });
      await restoreRuns();
      await openSavedRun("s1");

      sendChat.mockResolvedValue(null);
      await askAgent("follow up", POD);
      // `resume` came off disk, so the CLI picks the conversation back up.
      expect(sendChat.mock.calls.at(-1)?.[7]).toBe("cli-1");
    });

    it("deletes the file when the reader clears the conversation, after the write drains", async () => {
      sendChat.mockResolvedValue(null);
      await askAgent("q", POD);
      const id = (saveSession.mock.calls.at(-1)?.[0] as { id: string }).id;

      clearAgentRun();
      await vi.waitFor(() => expect(deleteSession).toHaveBeenCalledWith(id));
    });

    it("does not delete a file while a write to it is still in flight", async () => {
      // Classic's own lesson, in its `persistChainRef` comment: a save still
      // flushing when the delete lands recreates the file and its index entry,
      // so the reader's clear silently un-deletes itself. One chain per run is
      // what orders them.
      let finishWrite!: () => void;
      // ONLY the first write hangs. Every later one resolves, or the chain
      // would be waiting on write #2 and this test would pass for the wrong
      // reason — a delete that never fires because a save never finished.
      saveSession
        .mockImplementationOnce(() => new Promise<void>((resolve) => { finishWrite = () => resolve(); }))
        .mockResolvedValue(undefined);
      sendChat.mockResolvedValue(null);
      void askAgent("q", POD);
      await vi.waitFor(() => expect(saveSession).toHaveBeenCalled());

      clearAgentRun();
      // The write has not landed, so neither may the delete.
      await Promise.resolve();
      expect(deleteSession).not.toHaveBeenCalled();

      finishWrite();
      await vi.waitFor(() => expect(deleteSession).toHaveBeenCalled());
    });

    it("does not lose a later conversation to an earlier delete", async () => {
      sendChat.mockResolvedValue(null);
      await askAgent("first", POD);
      const first = (saveSession.mock.calls.at(-1)?.[0] as { id: string }).id;
      clearAgentRun();
      await vi.waitFor(() => expect(deleteSession).toHaveBeenCalledWith(first));

      // A new question in the same run writes a NEW file, not the one just
      // removed — otherwise the delete and the save race for the same path.
      await askAgent("second", POD);
      const second = (saveSession.mock.calls.at(-1)?.[0] as { id: string }).id;
      expect(second).not.toBe(first);
    });
  });

  /**
   * The refusal said "srelens is still answering the last question" and stayed
   * there after the answer had visibly arrived — a current-sounding message
   * about a condition that was over.
   */
  describe("a refusal stops being shown when what it was about is over", () => {
    const A = { about: { cluster: "prod-eu" }, route: "/k/statefulsets" };
    const B = { about: { cluster: "prod-eu", namespace: "ns", kind: "Pod", name: "p" }, route: "/k/Pod/ns/p" };

    it("clears the refusal once the turn it named has finished", async () => {
      let finish!: (v: string | null) => void;
      sendChat.mockImplementationOnce(() => new Promise<string | null>((r) => { finish = r; }));
      void askAgent("long one", A);
      await untilSendChatCalledTimes(1);

      await askAgent("second", B);
      expect(getAgentRun().error).toMatch(/still answering/i);

      sendChat.mockResolvedValue(null);
      finish(null);
      await vi.waitFor(() => {
        expect(getRun(runKeyFor(B.about, B.route)).error).toBeUndefined();
      });
    });

    it("does NOT clear a real failure the same way", async () => {
      // A `cancelChat` that did not land is a different thing: it says
      // something went wrong, and it is still true after the turn ends.
      sendChat.mockImplementation(() => new Promise<string | null>(() => {}));
      void askAgent("q", A);
      await untilSendChatCalledTimes(1);
      cancelChat.mockRejectedValueOnce(new Error("cancel refused"));
      stopAgentRun();
      await vi.waitFor(() => expect(getAgentRun().error).toBeDefined());
      const failure = getAgentRun().error;
      expect(failure).not.toMatch(/still answering/i);
    });
  });
});