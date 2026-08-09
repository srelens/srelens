import { invokeCommand, subscribe } from "../transport/transport";

export type ToolStatus = "ok" | "error" | "denied";

export type AgentEvent =
  | { type: "textDelta"; text: string }
  | { type: "toolCallStart"; id: string; tool: string; args: unknown }
  | { type: "toolResult"; id: string; status: ToolStatus }
  | { type: "turnDone" }
  | { type: "error"; message: string };

export interface AgentInfo {
  kind: "claude" | "codex" | "cursor";
  label: string;
  available: boolean;
  path: string | null;
  version: string | null;
  installUrl: string;
  /** Installed but not yet selectable — sandbox story pending (Codex/Cursor). */
  gated: boolean;
}

/** Validate a raw channel payload into a typed event, or null if unknown. */
export function parseAgentEvent(raw: unknown): AgentEvent | null {
  if (typeof raw !== "object" || raw === null) return null;
  const t = (raw as { type?: unknown }).type;
  switch (t) {
    case "textDelta":
    case "toolCallStart":
    case "toolResult":
    case "turnDone":
    case "error":
      return raw as AgentEvent;
    default:
      return null;
  }
}

export function listAgents(): Promise<AgentInfo[]> {
  return invokeCommand("agent_list");
}

export function startChat(): Promise<string> {
  return invokeCommand("chat_start");
}

/**
 * Send one user turn. Subscribes to the session channel BEFORE invoking so the
 * first streamed event can't race the listener (the logs pattern).
 */
export async function sendChat(
  session: string,
  prompt: string,
  agentPath: string,
  onEvent: (e: AgentEvent) => void,
  images?: string[],
): Promise<void> {
  const unsub = await subscribe(`chat://${session}`, (payload: unknown) => {
    const e = parseAgentEvent(payload);
    if (e) onEvent(e);
  });
  try {
    await invokeCommand("chat_send", { session, prompt, images: images ?? [], agentPath });
  } finally {
    unsub();
  }
}

export function cancelChat(session: string): Promise<void> {
  return invokeCommand("chat_cancel", { session });
}
