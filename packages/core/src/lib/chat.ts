import { invokeCommand, subscribe } from "../transport/transport";

export type ToolStatus = "ok" | "error" | "denied";

export type AgentEvent =
  | { type: "textDelta"; text: string }
  | { type: "thinking"; text: string }
  | { type: "toolCallStart"; id: string; tool: string; args: unknown }
  | { type: "toolResult"; id: string; status: ToolStatus }
  | { type: "turnDone" }
  | { type: "error"; message: string };

/** Agent kinds: the three vendor CLIs plus srelens's own native in-process
 * agent (`srelens`), which talks directly to a provider API with the user's
 * key and needs no installed binary. */
export type AgentKind = "claude" | "codex" | "cursor" | "srelens";

export interface AgentInfo {
  kind: AgentKind;
  label: string;
  available: boolean;
  path: string | null;
  version: string | null;
  installUrl: string;
  /** Installed but not yet selectable — currently always `false` for every
   * kind (Claude, Codex, and Cursor are all boxed to srelens's MCP tools and
   * shipped); kept so a future agent whose sandbox story isn't solved yet has
   * a place to gate from without a UI/wire-format change. */
  gated: boolean;
}

/** Validate a raw channel payload into a typed event, or null if unknown. */
export function parseAgentEvent(raw: unknown): AgentEvent | null {
  if (typeof raw !== "object" || raw === null) return null;
  const t = (raw as { type?: unknown }).type;
  switch (t) {
    case "textDelta":
    case "thinking":
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
 *
 * `agentKind` selects which CLI + MCP wiring `chat_send` uses — "claude",
 * "codex", or "cursor", all accepted server-side. Defaults to `"claude"` so
 * an existing caller that hasn't been updated to pass an
 * agent's kind (e.g. `SkillsPanel`'s one-shot generation turn) keeps its prior
 * behavior rather than failing to compile or silently misrouting.
 *
 * `turn` is the caller's turn generation, matched against the `turn` of a
 * `cancelChat` so the backend can honor a Stop that reaches it before this
 * send does (the `subscribe` await below opens exactly that window) while
 * dropping a stale Stop left over from an earlier turn. Callers that never
 * cancel can omit it.
 *
 * `resume` is the agent CLI's own session id returned by a previous call for
 * this conversation; the backend passes it to the CLI's `--resume` so
 * follow-up turns keep their context. Resolves to what the caller should now
 * store as the conversation's id: the id captured from this turn's stream, the
 * echoed `resume` when the turn was cancelled or ran clean without yielding
 * one, or `null` — meaning CLEAR — for agents with no id (native srelens
 * agent, Codex) and for a crashed resume (the CLI lost the session; retrying
 * the same id would fail forever). A REJECTION, by contrast, says nothing
 * about the stored id — keep it.
 */
export async function sendChat(
  session: string,
  prompt: string,
  agentPath: string,
  onEvent: (e: AgentEvent) => void,
  images?: string[],
  agentKind: string = "claude",
  turn: number = 0,
  resume: string | null = null,
): Promise<string | null> {
  const unsub = await subscribe(`chat://${session}`, (payload: unknown) => {
    const e = parseAgentEvent(payload);
    if (e) onEvent(e);
  });
  try {
    return await invokeCommand("chat_send", { session, prompt, images: images ?? [], agentPath, agentKind, turn, resume });
  } finally {
    unsub();
  }
}

/** Stop the turn `turn` (the generation passed to its `sendChat`) — running,
 * mid-preparation, or not yet arrived at the backend. */
export function cancelChat(session: string, turn: number = 0): Promise<void> {
  return invokeCommand("chat_cancel", { session, turn });
}
