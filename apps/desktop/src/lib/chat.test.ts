import { describe, it, expect, vi } from "vitest";
import { parseAgentEvent } from "./chat";

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
