import { describe, it, expect, vi, beforeEach } from "vitest";
import { parseAgentEvent } from "./chat";

const { invokeCommandMock, subscribeMock } = vi.hoisted(() => ({
  invokeCommandMock: vi.fn(),
  subscribeMock: vi.fn(),
}));
vi.mock("../transport/transport", async () => {
  const actual = await vi.importActual<typeof import("../transport/transport")>("../transport/transport");
  return { ...actual, invokeCommand: invokeCommandMock, subscribe: subscribeMock };
});

describe("sendChat", () => {
  beforeEach(() => {
    invokeCommandMock.mockReset().mockResolvedValue(undefined);
    subscribeMock.mockReset().mockResolvedValue(() => {});
  });

  it("defaults agentKind to claude and turn to 0 when the caller doesn't pass them", async () => {
    const { sendChat } = await import("./chat");
    await sendChat("s1", "hi", "/usr/bin/claude", () => {});
    expect(invokeCommandMock).toHaveBeenCalledWith("chat_send", {
      session: "s1",
      prompt: "hi",
      images: [],
      agentPath: "/usr/bin/claude",
      agentKind: "claude",
      turn: 0,
      resume: null,
    });
  });

  it("threads an explicit agentKind and turn through to chat_send's invoke args", async () => {
    const { sendChat } = await import("./chat");
    await sendChat("s1", "hi", "/usr/bin/codex", () => {}, ["AAAA"], "codex", 3);
    expect(invokeCommandMock).toHaveBeenCalledWith("chat_send", {
      session: "s1",
      prompt: "hi",
      images: ["AAAA"],
      agentPath: "/usr/bin/codex",
      agentKind: "codex",
      turn: 3,
      resume: null,
    });
  });

  it("cancelChat passes the session and turn generation to chat_cancel", async () => {
    const { cancelChat } = await import("./chat");
    await cancelChat("s1", 2);
    expect(invokeCommandMock).toHaveBeenCalledWith("chat_cancel", { session: "s1", turn: 2 });
  });
});

describe("parseAgentEvent", () => {
  it("passes through a well-formed textDelta", () => {
    const e = parseAgentEvent({ type: "textDelta", text: "hi" });
    expect(e).toEqual({ type: "textDelta", text: "hi" });
  });

  it("keeps tool-call fields", () => {
    const e = parseAgentEvent({ type: "toolCallStart", id: "t1", tool: "k8s.scale", args: { replicas: 3 } });
    expect(e?.type).toBe("toolCallStart");
    if (e?.type === "toolCallStart") expect(e.tool).toBe("k8s.scale");
  });

  it("rejects an unknown type as null", () => {
    expect(parseAgentEvent({ type: "wat" })).toBeNull();
  });
});
