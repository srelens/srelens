import React, { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Badge, Button, Spinner, TextInput } from "../ui";
import { listAgents, startChat, sendChat, type AgentEvent, type AgentInfo, type ToolStatus } from "../lib/chat";
import { respondToConfirm, type ConfirmRequest } from "../lib/mcpSecurity";
import {
  deleteSession as deleteSessionCmd,
  listSessions,
  loadSession,
  saveSession,
  type Session,
  type SessionMeta,
  type StoredMessage,
  type StoredToolCall,
} from "../lib/chatHistory";
import { AssistantMarkdown } from "./AssistantMarkdown";

export type AssistantContext = { context: string; namespace?: string; kind?: string; name?: string };

/** Chip text: `cluster[ / namespace][ / Kind name]`. */
function formatContext(c: AssistantContext): string {
  let text = c.context;
  if (c.namespace) text += ` / ${c.namespace}`;
  if (c.kind && c.name) text += ` / ${c.kind} ${c.name}`;
  return text;
}

/** One-line preface prepended to the prompt so the agent knows the target. */
function contextPreface(c: AssistantContext): string {
  let text = `Current context: cluster ${c.context}`;
  if (c.namespace) text += `, namespace ${c.namespace}`;
  if (c.kind && c.name) text += `, ${c.kind} ${c.name}`;
  return `${text}.`;
}

interface ToolCallState {
  tool: string;
  args: unknown;
  /** null while the call is in flight (no toolResult yet). */
  status: ToolStatus | null;
}

interface ChatMessage {
  id: number;
  /** `error` renders as its own red bubble — a transport failure or a stream
   * `error` event, neither of which belongs to either side of the exchange. */
  role: "user" | "assistant" | "error";
  text: string;
  /** Tool calls started during this (assistant) turn, in order. */
  toolCallIds?: string[];
}

const STATUS_LABEL: Record<ToolStatus, string> = { ok: "ok", error: "error", denied: "denied" };

/** Longest a derived session title is allowed to be before it's truncated
 * with an ellipsis — long enough to stay recognizable in a narrow list. */
const MAX_TITLE_LEN = 60;

/** `Session.title` = the first user message, trimmed and capped to a sane
 * length; "New chat" once none exists (a turn that errored before the user
 * ever sent anything, in practice never reached since a turn always starts
 * from a user message, but kept as a safe fallback). */
function deriveTitle(msgs: ChatMessage[]): string {
  const firstUser = msgs.find((m) => m.role === "user");
  const trimmed = firstUser?.text.trim() ?? "";
  if (!trimmed) return "New chat";
  return trimmed.length > MAX_TITLE_LEN ? `${trimmed.slice(0, MAX_TITLE_LEN).trimEnd()}…` : trimmed;
}

/** Flatten the live transcript into the disk shape: each message's tool
 * calls (referenced by id into the separate `toolCalls` record while live)
 * get embedded directly, since that record doesn't survive a reload. */
function toStoredMessages(msgs: ChatMessage[], calls: Record<string, ToolCallState>): StoredMessage[] {
  return msgs.map((m) => {
    const toolCalls = (m.toolCallIds ?? [])
      .map((id) => (calls[id] ? { id, tool: calls[id].tool, args: calls[id].args, status: calls[id].status } : null))
      .filter((tc): tc is StoredToolCall => tc !== null);
    return toolCalls.length > 0 ? { id: m.id, role: m.role, text: m.text, toolCalls } : { id: m.id, role: m.role, text: m.text };
  });
}

/** The inverse of `toStoredMessages` — rebuilds the live `ChatMessage[]` plus
 * a `toolCalls` record from a loaded session's opaque `messages`. */
function fromStoredMessages(stored: StoredMessage[]): { msgs: ChatMessage[]; calls: Record<string, ToolCallState> } {
  const calls: Record<string, ToolCallState> = {};
  const msgs: ChatMessage[] = stored.map((m) => {
    for (const tc of m.toolCalls ?? []) calls[tc.id] = { tool: tc.tool, args: tc.args, status: tc.status };
    const toolCallIds = m.toolCalls?.map((tc) => tc.id);
    return toolCallIds?.length ? { id: m.id, role: m.role, text: m.text, toolCallIds } : { id: m.id, role: m.role, text: m.text };
  });
  return { msgs, calls };
}

function summarizeArgs(args: unknown): string {
  if (args == null) return "";
  if (typeof args === "object" && Object.keys(args as Record<string, unknown>).length === 0) return "";
  try {
    return JSON.stringify(args);
  } catch {
    return "";
  }
}

/**
 * A single tool invocation: name and a status badge (spinner while running)
 * are always visible; the args summary is behind a disclosure toggle,
 * collapsed by default so a busy turn doesn't flood the transcript.
 */
function ToolCallCard({ tool, args, status }: ToolCallState) {
  const [expanded, setExpanded] = useState(false);
  const summary = summarizeArgs(args);
  return (
    <div className="mt-2 rounded-md border border-border bg-muted/40 px-3 py-2 text-xs">
      <button
        type="button"
        className="flex w-full items-center justify-between gap-2 text-left"
        aria-expanded={expanded}
        onClick={() => setExpanded((e) => !e)}
      >
        <span className="font-mono font-medium">{tool}</span>
        {status ? (
          <Badge variant={status === "ok" ? "success" : status === "denied" ? "warning" : "danger"}>
            {STATUS_LABEL[status]}
          </Badge>
        ) : (
          <Spinner className="size-3" label="Running" />
        )}
      </button>
      {expanded && summary && <div className="mt-1 truncate font-mono text-muted-foreground">{summary}</div>}
    </div>
  );
}

/**
 * The same `mcp://confirm-request` the modal (`McpConfirmDialog`) answers,
 * rendered inline in the transcript so the approval is visible next to the
 * turn that triggered it. Both views call `respondToConfirm` with the same
 * `id` — the backend resolves whichever answers first and errors harmlessly
 * on the second, which this view swallows since the modal already surfaces
 * that failure.
 */
function ConfirmCard({
  request,
  onAnswer,
}: {
  request: ConfirmRequest;
  onAnswer: (id: string, approved: boolean) => void;
}) {
  const summary = summarizeArgs(request.args);
  return (
    <div className="mt-2 rounded-md border border-border bg-muted/40 px-3 py-2 text-xs">
      <div className="flex items-center justify-between gap-2">
        <span className="font-mono font-medium">{request.tool}</span>
        <span className="text-muted-foreground">wants to run</span>
      </div>
      {summary && <div className="mt-1 truncate font-mono text-muted-foreground">{summary}</div>}
      <div className="mt-2 flex justify-end gap-2">
        <Button variant="secondary" size="xs" onClick={() => onAnswer(request.id, false)}>
          Deny
        </Button>
        <Button size="xs" onClick={() => onAnswer(request.id, true)}>
          Approve
        </Button>
      </div>
    </div>
  );
}

/**
 * The assistant conversation: a streamed exchange with the configured coding
 * agent, plus collapsible tool-call cards for anything it invokes. `context`
 * (the resource/namespace the caller had active) is rendered as a removable
 * chip and, while attached, prefaced onto the prompt sent to the agent — it
 * is entirely optional, so this same component serves both a resource-scoped
 * host (the `AssistantDrawer`) and a global one with no resource in scope
 * (the `AssistantTab`). The agent picker is sourced from `listAgents()`; an
 * unavailable agent shows its install link, a gated one (Codex/Cursor, whose
 * sandbox isn't solved yet) shows a "coming soon" note instead — either way
 * Send stays disabled.
 */
export function AssistantConversation({
  context,
  className,
}: {
  context?: AssistantContext;
  className?: string;
}) {
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [toolCalls, setToolCalls] = useState<Record<string, ToolCallState>>({});
  const [input, setInput] = useState("");
  const [sending, setSending] = useState(false);
  const [agents, setAgents] = useState<AgentInfo[]>([]);
  const [selectedKind, setSelectedKind] = useState("");
  const [attachedContext, setAttachedContext] = useState<AssistantContext | undefined>(context);
  const [pendingConfirms, setPendingConfirms] = useState<ConfirmRequest[]>([]);
  // The saved-session picker (Task 19 builds the real rail; this is a
  // minimal control so New chat / reopen / delete work in the meantime).
  const [sessions, setSessions] = useState<SessionMeta[]>([]);

  const sessionRef = useRef<string | null>(null);
  const nextId = useRef(0);
  // Mirrors of `messages`/`toolCalls` kept in lockstep with every update (see
  // `setMessagesTracked`/`setToolCallsTracked`) so the `turnDone`/`error`
  // auto-save can read the value a completed turn just produced without
  // depending on a React re-render having landed yet — `applyEvent` fires
  // from a channel callback, not a DOM event handler, so React may still
  // batch several of this turn's `setState` calls together.
  const messagesRef = useRef<ChatMessage[]>([]);
  const toolCallsRef = useRef<Record<string, ToolCallState>>({});
  // Set once per disk session, on its first save; cleared by New chat and
  // restored from the loaded value when reopening a session, so re-saving
  // never stomps the original `createdAt`.
  const createdAtRef = useRef<number | null>(null);

  function setMessagesTracked(updater: (msgs: ChatMessage[]) => ChatMessage[]) {
    const next = updater(messagesRef.current);
    messagesRef.current = next;
    setMessages(next);
  }

  function setToolCallsTracked(updater: (calls: Record<string, ToolCallState>) => Record<string, ToolCallState>) {
    const next = updater(toolCallsRef.current);
    toolCallsRef.current = next;
    setToolCalls(next);
  }
  // A single Claude turn streams several `textDelta`s onto the same assistant
  // message, split up by tool calls in between. Left alone they'd concatenate
  // raw ("cluster.This is a"); this tracks whether a tool event has landed
  // since the last delta so the next delta can start a fresh paragraph.
  const toolEventSincePendingDelta = useRef(false);

  // This component only exists while its host (the drawer or the tab) is
  // showing it, so a subscription made on mount already covers "each time
  // this view becomes visible" — no visibility flag to gate on here.
  useEffect(() => {
    const unlisten = listen<ConfirmRequest>("mcp://confirm-request", (event) => {
      setPendingConfirms((q) => [...q, event.payload]);
    });
    return () => {
      void unlisten.then((f) => f());
    };
  }, []);

  async function answerConfirm(id: string, approved: boolean) {
    setPendingConfirms((q) => q.filter((r) => r.id !== id));
    try {
      await respondToConfirm(id, approved);
    } catch {
      // Already answered elsewhere (the modal, or a server-side timeout) —
      // that failure is surfaced there; nothing left here to retry.
    }
  }

  // Re-sync the locally-editable context if the caller's target changes while
  // this view stays mounted (e.g. "Ask assistant" clicked on a different
  // resource while the drawer is already open on this component instance).
  useEffect(() => {
    setAttachedContext(context);
  }, [context]);

  useEffect(() => {
    listAgents()
      .then((list) => {
        setAgents(list);
        const firstAvailable = list.find((a) => a.available && !a.gated) ?? list.find((a) => a.available) ?? list[0];
        setSelectedKind(firstAvailable?.kind ?? "");
      })
      .catch(() => {
        setAgents([]);
        setSelectedKind("");
      });
  }, []);

  // Load the saved-session picker once on mount — `listSessions` already
  // returns newest-first, so this renders in that order verbatim.
  useEffect(() => {
    listSessions()
      .then(setSessions)
      .catch(() => setSessions([]));
  }, []);

  const selectedAgent = agents.find((a) => a.kind === selectedKind);
  const agentPath = selectedAgent?.path ?? "";
  const canSend = !!selectedAgent?.available && !selectedAgent?.gated;

  /** Serialize the current transcript and save it to disk. Best-effort: a
   * failed save is swallowed rather than surfaced, since it must never break
   * the live chat the user is actually looking at. */
  async function persistSession(msgs: ChatMessage[], calls: Record<string, ToolCallState>) {
    const id = sessionRef.current;
    if (!id) return; // nothing sent yet this conversation — no id to save under
    const now = Date.now();
    if (createdAtRef.current === null) createdAtRef.current = now;
    const session: Session = {
      id,
      title: deriveTitle(msgs),
      createdAt: createdAtRef.current,
      updatedAt: now,
      // Task 17 replaces this single-context derivation with a real
      // multi-select; for now the session just remembers the one chip
      // (if any) that was attached when the turn ran.
      contexts: attachedContext?.context ? [attachedContext.context] : [],
      skills: [],
      // Not wired yet — the `AgentEvent` stream doesn't carry the agent
      // CLI's own session id, and threading it through needs a backend
      // change. `null` here means a reopened session is continued
      // best-effort (replayed transcript only, no CLI `--resume`), which
      // the spec (§2) allows.
      cliSessionId: null,
      messages: toStoredMessages(msgs, calls),
    };
    try {
      await saveSession(session);
      setSessions((prev) => {
        const meta: SessionMeta = { id: session.id, title: session.title, createdAt: session.createdAt, updatedAt: session.updatedAt };
        return [meta, ...prev.filter((s) => s.id !== id)].sort((a, b) => b.updatedAt - a.updatedAt);
      });
    } catch {
      // Disk persistence is a convenience, not the primary function — the
      // live conversation just keeps going.
    }
  }

  /** Clear the transcript and drop the channel/disk session id so the next
   * `handleSend` mints a fresh one via `startChat()`. */
  function onNewChat() {
    setMessagesTracked(() => []);
    setToolCallsTracked(() => ({}));
    setInput("");
    sessionRef.current = null;
    createdAtRef.current = null;
  }

  /** Load a saved session and replay it into state read-only — no event
   * stream involved, just the transcript as it was last saved. Continuing
   * to type and send afterward appends further turns onto the same disk
   * session (see the deferred-`cliSessionId` note on `persistSession`). */
  async function onSelectSession(id: string) {
    try {
      const session = await loadSession(id);
      const { msgs, calls } = fromStoredMessages(session.messages as unknown as StoredMessage[]);
      setMessagesTracked(() => msgs);
      setToolCallsTracked(() => calls);
      nextId.current = msgs.reduce((max, m) => Math.max(max, m.id + 1), 0);
      sessionRef.current = session.id;
      createdAtRef.current = session.createdAt;
      // `session.contexts` is deliberately not restored into
      // `attachedContext` yet — Task 17 owns the real multi-context
      // select this would need to round-trip through.
    } catch {
      // Leave the current conversation untouched on a bad load.
    }
  }

  /** Delete a saved session from disk and drop it from the picker. If it was
   * the one currently open, this behaves like New chat. */
  async function onDeleteSession(id: string) {
    try {
      await deleteSessionCmd(id);
    } catch {
      // Best-effort — still drop it from the visible list below.
    }
    setSessions((prev) => prev.filter((s) => s.id !== id));
    if (sessionRef.current === id) onNewChat();
  }

  function applyEvent(e: AgentEvent) {
    switch (e.type) {
      case "toolCallStart":
        toolEventSincePendingDelta.current = true;
        setToolCallsTracked((tc) => ({ ...tc, [e.id]: { tool: e.tool, args: e.args, status: null } }));
        setMessagesTracked((msgs) => {
          const last = msgs[msgs.length - 1];
          if (!last || last.role !== "assistant") return msgs;
          return [...msgs.slice(0, -1), { ...last, toolCallIds: [...(last.toolCallIds ?? []), e.id] }];
        });
        break;
      case "toolResult":
        toolEventSincePendingDelta.current = true;
        setToolCallsTracked((tc) => (tc[e.id] ? { ...tc, [e.id]: { ...tc[e.id], status: e.status } } : tc));
        break;
      case "textDelta": {
        // Read + reset the flag synchronously, here, rather than inside the
        // `setMessages` updater below: React batches updates from this
        // (non-event-handler) callback, so the updater functions for
        // several `textDelta`s can all run together during one flush, by
        // which point a ref mutation made *inside* an updater would already
        // reflect every event that happened after it, not just the ones
        // before it — reading the flag here pins it to this event's turn.
        const toolEventPending = toolEventSincePendingDelta.current;
        toolEventSincePendingDelta.current = false;
        setMessagesTracked((msgs) => {
          const last = msgs[msgs.length - 1];
          if (!last || last.role !== "assistant") return msgs;
          const needsBreak =
            toolEventPending && last.text.length > 0 && !/\s$/.test(last.text) && !/^\s/.test(e.text);
          const separator = needsBreak ? "\n\n" : "";
          return [...msgs.slice(0, -1), { ...last, text: last.text + separator + e.text }];
        });
        break;
      }
      case "error":
        // Don't persist here: the backend always follows a stream `error`
        // with a terminal `turnDone` on the same channel once it's live
        // (crash-recovery in `finish_turn`, and the bad-image-attachment
        // path both do this) — saving on both would double-save the turn.
        setMessagesTracked((msgs) => [...msgs, { id: nextId.current++, role: "error", text: e.message }]);
        break;
      case "turnDone":
        void persistSession(messagesRef.current, toolCallsRef.current);
        break;
    }
  }

  async function handleSend() {
    const prompt = input.trim();
    if (!prompt || sending || !canSend) return;
    const outgoing = attachedContext ? `${contextPreface(attachedContext)}\n\n${prompt}` : prompt;
    setInput("");
    setMessagesTracked((msgs) => [
      ...msgs,
      { id: nextId.current++, role: "user", text: prompt },
      { id: nextId.current++, role: "assistant", text: "" },
    ]);
    setSending(true);
    try {
      let session = sessionRef.current;
      if (!session) {
        session = await startChat();
        sessionRef.current = session;
      }
      await sendChat(session, outgoing, agentPath, applyEvent);
    } catch (e) {
      // A rejection here means the transport itself failed before any
      // `error` event could stream (e.g. `chat_send` rejects outright when
      // the MCP server isn't running) — without this it would be a silent
      // unhandled rejection and the user would just see the turn hang.
      const text = e instanceof Error ? e.message : String(e);
      setMessagesTracked((msgs) => [...msgs, { id: nextId.current++, role: "error", text }]);
      void persistSession(messagesRef.current, toolCallsRef.current);
    } finally {
      setSending(false);
    }
  }

  const agentPicker = agents.length > 0 && (
    <select
      aria-label="Agent"
      className="fl-select text-xs"
      value={selectedKind}
      onChange={(e) => setSelectedKind(e.target.value)}
    >
      {agents.map((a) => (
        <option key={a.kind} value={a.kind} disabled={!a.available || a.gated}>
          {a.label}
          {!a.available ? " (not installed)" : a.gated ? " (not yet available)" : ""}
        </option>
      ))}
    </select>
  );

  // Minimal session picker: a New chat button plus one chip per saved
  // session (title + delete). Task 19 replaces this with the real history
  // rail; `sessions`/`onNewChat`/`onSelectSession`/`onDeleteSession` above
  // are already shaped for that to just wire in.
  const sessionBar = (
    <div className="flex shrink-0 flex-wrap items-center justify-between gap-2">
      <div className="flex flex-wrap items-center gap-1">
        <Button variant="secondary" size="xs" onClick={onNewChat}>
          New chat
        </Button>
        {sessions.map((s) => (
          <span
            key={s.id}
            className="inline-flex max-w-[10rem] items-center gap-1 rounded-md border border-border px-2 py-1 text-xs"
          >
            <button type="button" className="truncate" title={s.title} onClick={() => void onSelectSession(s.id)}>
              {s.title}
            </button>
            <button
              type="button"
              aria-label={`Delete ${s.title}`}
              onClick={() => void onDeleteSession(s.id)}
              className="text-muted-foreground hover:text-foreground"
            >
              ✕
            </button>
          </span>
        ))}
      </div>
      {agentPicker}
    </div>
  );

  return (
    <div className={`flex h-full flex-col gap-3${className ? ` ${className}` : ""}`}>
      {sessionBar}
      <div className="flex-1 space-y-3 overflow-y-auto">
        {messages.length === 0 && (
          <p className="text-sm text-muted-foreground">Ask about this cluster to get started.</p>
        )}
        {messages.map((m) => (
          <div key={m.id} className={m.role === "user" ? "text-right" : "text-left"}>
            <div
              className={
                m.role === "user"
                  ? "inline-block whitespace-pre-wrap rounded-md bg-primary/10 px-3 py-2 text-left text-sm"
                  : m.role === "error"
                    ? "inline-block whitespace-pre-wrap rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-left text-sm text-destructive"
                    : "inline-block max-w-full rounded-md bg-muted px-3 py-2 text-left text-sm"
              }
            >
              {m.role === "assistant" ? (
                m.text ? <AssistantMarkdown text={m.text} /> : sending ? "…" : ""
              ) : (
                m.text
              )}
            </div>
            {m.toolCallIds?.map((id) => {
              const tc = toolCalls[id];
              return tc ? <ToolCallCard key={id} tool={tc.tool} args={tc.args} status={tc.status} /> : null;
            })}
          </div>
        ))}
        {pendingConfirms.map((req) => (
          <ConfirmCard key={req.id} request={req} onAnswer={answerConfirm} />
        ))}
      </div>
      {!canSend && (
        <p className="shrink-0 text-xs text-muted-foreground">
          {selectedAgent
            ? selectedAgent.gated
              ? "Codex/Cursor support is coming — use Claude for now."
              : selectedAgent.installUrl
                ? (
                  <>
                    {selectedAgent.label} isn&apos;t installed.{" "}
                    <a href={selectedAgent.installUrl} target="_blank" rel="noreferrer" className="underline">
                      {selectedAgent.installUrl}
                    </a>
                  </>
                )
                : `${selectedAgent.label} isn't installed.`
            : "No coding agent available. Install one to use the assistant."}
        </p>
      )}
      {attachedContext && (
        <div className="flex shrink-0 items-center gap-2 rounded-md border border-border bg-muted/40 px-3 py-1.5 text-xs">
          <span className="min-w-0 flex-1 truncate">{formatContext(attachedContext)}</span>
          <button
            type="button"
            aria-label="Remove context"
            onClick={() => setAttachedContext(undefined)}
            className="text-muted-foreground hover:text-foreground"
          >
            ✕
          </button>
        </div>
      )}
      <div className="flex shrink-0 items-end gap-2 border-t border-border pt-3">
        <TextInput
          value={input}
          onValueChange={setInput}
          onEnter={handleSend}
          placeholder="Ask about this cluster..."
          disabled={sending}
          className="flex-1"
        />
        <Button onClick={handleSend} disabled={sending || !input.trim() || !canSend}>
          Send
        </Button>
      </div>
    </div>
  );
}
