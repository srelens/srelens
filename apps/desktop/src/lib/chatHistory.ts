// Typed wrappers for the four chat-history commands (backend: `assistant_history.rs`,
// Task 14) — disk persistence for the in-app AI assistant's chat sessions.
// Field names are camelCase to mirror the Rust `Session`/`SessionMeta`
// structs exactly (`#[serde(rename_all = "camelCase")]`) — no translation
// happens at this boundary.
import { invokeCommand } from "../transport/transport";
import type { ToolStatus } from "./chat";

/** Picker metadata only — no `messages`, so listing sessions stays cheap
 * even once a session's transcript grows large. */
export interface SessionMeta {
  id: string;
  title: string;
  createdAt: number;
  updatedAt: number;
}

/**
 * A full chat session, including its message transcript. `messages` is
 * opaque JSON to the backend — the frontend owns its shape; see
 * `StoredMessage` below for what `AssistantConversation` actually stores.
 */
export interface Session extends SessionMeta {
  contexts: string[];
  /** Active skill names (Task 23) — always empty for now. */
  skills: string[];
  /**
   * The agent CLI's own session id, captured from the stream by `chat_send`
   * and passed back to the CLI's `--resume` on the conversation's next turn
   * so follow-ups keep their context (Claude and Cursor; the native agent
   * and Codex have no resumable id and store `null`). `null` also covers
   * sessions saved before this was wired — those reopen fresh, best-effort.
   */
  cliSessionId: string | null;
  /** The agent CLI this conversation used ("claude"/"codex"), restored into
   * the picker on reopen. Defaulted (optional) for sessions saved before this
   * field existed. */
  agentKind?: string | null;
  messages: unknown[];
}

/** A frozen copy of a `ToolCallState` (see `AssistantConversation`),
 * embedded directly on the stored message it belongs to rather than
 * referenced by id into a separate live record — that record doesn't
 * survive a reload. */
export interface StoredToolCall {
  id: string;
  tool: string;
  args: unknown;
  status: ToolStatus | null;
}

/** Mirrors `AssistantConversation`'s `ChatMessage`, with its tool calls
 * embedded (see `StoredToolCall`) instead of referenced by id. */
export interface StoredMessage {
  id: number;
  role: "user" | "assistant" | "error";
  text: string;
  toolCalls?: StoredToolCall[];
  /** Data URIs (`data:image/...;base64,...`) attached to a user message
   * (Task 18) — only ever set for `role: "user"`. */
  images?: string[];
  /** Reasoning streamed before the answer (the collapsible "Thoughts" row) —
   * only for `role: "assistant"`, and only when the agent surfaced any. */
  thoughts?: string;
  /** Seconds spent thinking, when the agent streams reasoning live (absent
   * for Codex, whose summaries arrive already completed and untimeable). */
  thoughtSecs?: number;
}

/** Saved sessions, newest first. */
export function listSessions(): Promise<SessionMeta[]> {
  return invokeCommand("chat_history_list");
}

/** Load one full session (including its message transcript) by id. */
export function loadSession(id: string): Promise<Session> {
  return invokeCommand("chat_history_load", { id });
}

/** Persist a session, creating or updating both its file and index entry. */
export function saveSession(session: Session): Promise<void> {
  return invokeCommand("chat_history_save", { session });
}

/** Delete a session's file and its index entry. */
export function deleteSession(id: string): Promise<void> {
  return invokeCommand("chat_history_delete", { id });
}
