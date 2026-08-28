import { describe, it, expect, vi, beforeEach } from "vitest";
import { getAgentRun, askAgent, resetAgentRun, subscribeAgentRun, noteGate } from "./agentRun";

const { startChat, sendChat, listAgents } = vi.hoisted(() => ({
  startChat: vi.fn(), sendChat: vi.fn(), listAgents: vi.fn(),
}));
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  startChat, sendChat, listAgents,
}));

beforeEach(() => {
  resetAgentRun();
  startChat.mockResolvedValue("sess-1");
  listAgents.mockResolvedValue([{ kind: "claude", label: "Claude", available: true, path: "/c", version: "1", installUrl: "", gated: false }]);
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

  it("keeps its snapshot identity when nothing changed, so subscribers stay asleep", async () => {
    const before = getAgentRun();
    expect(getAgentRun()).toBe(before);
  });

  it("tells subscribers once per change", async () => {
    const seen = vi.fn();
    const off = subscribeAgentRun(seen);
    sendChat.mockImplementation(async () => null);
    await askAgent("q");
    expect(seen).toHaveBeenCalled();
    off();
  });

  it("merges a gate by id, so a second outcome for the same request replaces the first", () => {
    noteGate({ id: "g1", tool: "k8s.scale", args: { replicas: 3 }, outcome: "pending" });
    noteGate({ id: "g1", tool: "k8s.scale", args: { replicas: 3 }, outcome: "approved" });
    const gates = getAgentRun().gates;
    expect(gates).toHaveLength(1);
    expect(gates[0].outcome).toBe("approved");
  });
});
