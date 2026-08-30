import { describe, it, expect, vi, beforeEach } from "vitest";
import { describeError } from "@srelens/core";
import {
  askAgent,
  chooseAgent,
  clearAgentRun,
  getAgentRun,
  noteGate,
  resetAgentRun,
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

  it("clears the conversation but keeps its gates and the chosen agent", async () => {
    sendChat.mockImplementation(async () => null);
    await askAgent("q");
    noteGate({ id: "g1", tool: "k8s.scale", args: {}, outcome: "pending" });
    chooseAgent("codex");

    clearAgentRun();

    const run = getAgentRun();
    expect(run.turns).toEqual([]);
    expect(run.busy).toBe(false);
    expect(run.gates).toHaveLength(1);
    expect(run.agentKind).toBe("codex");
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

  it("a superseded turn's own busy/resume result never overwrites the current one", async () => {
    let resolveFirst!: (v: string | null) => void;
    let resolveSecond!: (v: string | null) => void;
    sendChat
      .mockImplementationOnce(() => new Promise<string | null>((resolve) => { resolveFirst = resolve; }))
      .mockImplementationOnce(() => new Promise<string | null>((resolve) => { resolveSecond = resolve; }))
      .mockImplementationOnce(async () => null);

    const first = askAgent("q1");
    const second = askAgent("q2");
    await untilSendChatCalledTimes(2);

    // The stale turn settles first, carrying a resume token that must not
    // stick — the second question is the one the reader is now waiting on.
    resolveFirst("stale-token");
    await first;
    expect(getAgentRun().busy).toBe(true); // the LIVE turn hasn't finished

    resolveSecond("live-token");
    await second;
    expect(getAgentRun().busy).toBe(false);

    // A third question resumes from the live conversation, not the stale one
    // the first turn tried to leave behind.
    await askAgent("q3");
    expect(sendChat).toHaveBeenLastCalledWith(
      "sess-1",
      "q3",
      "/c",
      expect.any(Function),
      undefined,
      "claude",
      3,
      "live-token",
    );
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
});
