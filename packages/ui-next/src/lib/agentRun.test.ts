import { describe, it, expect, vi, beforeEach } from "vitest";
import { describeError } from "@srelens/core";
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
} from "./agentRun";

const { startChat, sendChat, listAgents, cancelChat, loadSkill } = vi.hoisted(() => ({
  startChat: vi.fn(), sendChat: vi.fn(), listAgents: vi.fn(), cancelChat: vi.fn(), loadSkill: vi.fn(),
}));
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  startChat, sendChat, listAgents, cancelChat, loadSkill,
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
  startChat.mockResolvedValue("sess-1");
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

  it("merges a gate by id, so a second outcome for the same request replaces the first", () => {
    noteGate({ id: "g1", tool: "k8s.scale", args: { replicas: 3 }, outcome: "pending" });
    noteGate({ id: "g1", tool: "k8s.scale", args: { replicas: 3 }, outcome: "approved" });
    const gates = getAgentRun().gates;
    expect(gates).toHaveLength(1);
    expect(gates[0].outcome).toBe("approved");
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
    // Unlike the chosen agent, a skill picked for the run that just ended is
    // not "still active" for whatever run comes next.
    expect(run.activeSkills).toEqual([]);
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
});
