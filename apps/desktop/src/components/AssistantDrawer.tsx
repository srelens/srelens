import React, { useEffect, useRef, useState } from "react";
import { Drawer } from "../ui/Drawer";
import { Badge, Button, Spinner, TextInput } from "../ui";
import { listAgents, startChat, sendChat, type AgentEvent, type AgentInfo, type ToolStatus } from "../lib/chat";

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
  role: "user" | "assistant";
  text: string;
  /** Tool calls started during this (assistant) turn, in order. */
  toolCallIds?: string[];
}

const STATUS_LABEL: Record<ToolStatus, string> = { ok: "ok", error: "error", denied: "denied" };

function summarizeArgs(args: unknown): string {
  if (args == null) return "";
  if (typeof args === "object" && Object.keys(args as Record<string, unknown>).length === 0) return "";
  try {
    return JSON.stringify(args);
  } catch {
    return "";
  }
}

/** A single tool invocation: name, a short args summary, and a status badge (spinner while running). */
function ToolCallCard({ tool, args, status }: ToolCallState) {
  const summary = summarizeArgs(args);
  return (
    <div className="mt-2 rounded-md border border-border bg-muted/40 px-3 py-2 text-xs">
      <div className="flex items-center justify-between gap-2">
        <span className="font-mono font-medium">{tool}</span>
        {status ? (
          <Badge variant={status === "ok" ? "success" : status === "denied" ? "warning" : "danger"}>
            {STATUS_LABEL[status]}
          </Badge>
        ) : (
          <Spinner className="size-3" label="Running" />
        )}
      </div>
      {summary && <div className="mt-1 truncate font-mono text-muted-foreground">{summary}</div>}
    </div>
  );
}

/**
 * Right-hand chat drawer: a streamed conversation with the configured coding
 * agent, plus collapsible tool-call cards for anything it invokes. `context`
 * (the resource/namespace the user had open) is rendered as a removable chip
 * and, while attached, prefaced onto the prompt sent to the agent. The header
 * carries an agent picker sourced from `listAgents()`; an unavailable agent
 * shows its install link and disables Send.
 */
export function AssistantDrawer({
  open,
  onClose,
  context,
}: {
  open: boolean;
  onClose: () => void;
  context?: AssistantContext;
}) {
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [toolCalls, setToolCalls] = useState<Record<string, ToolCallState>>({});
  const [input, setInput] = useState("");
  const [sending, setSending] = useState(false);
  const [agents, setAgents] = useState<AgentInfo[]>([]);
  const [selectedKind, setSelectedKind] = useState("");
  const [attachedContext, setAttachedContext] = useState<AssistantContext | undefined>(context);

  const sessionRef = useRef<string | null>(null);
  const nextId = useRef(0);

  // Re-sync the locally-editable context (and the agent list) each time the
  // drawer is (re)opened, so a fresh open picks up the caller's latest target.
  useEffect(() => {
    if (!open) return;
    setAttachedContext(context);
  }, [open, context]);

  useEffect(() => {
    if (!open) return;
    listAgents()
      .then((list) => {
        setAgents(list);
        const firstAvailable = list.find((a) => a.available) ?? list[0];
        setSelectedKind(firstAvailable?.kind ?? "");
      })
      .catch(() => {
        setAgents([]);
        setSelectedKind("");
      });
  }, [open]);

  const selectedAgent = agents.find((a) => a.kind === selectedKind);
  const agentPath = selectedAgent?.path ?? "";
  const canSend = !!selectedAgent?.available;

  function applyEvent(e: AgentEvent) {
    switch (e.type) {
      case "toolCallStart":
        setToolCalls((tc) => ({ ...tc, [e.id]: { tool: e.tool, args: e.args, status: null } }));
        setMessages((msgs) => {
          const last = msgs[msgs.length - 1];
          if (!last || last.role !== "assistant") return msgs;
          return [...msgs.slice(0, -1), { ...last, toolCallIds: [...(last.toolCallIds ?? []), e.id] }];
        });
        break;
      case "toolResult":
        setToolCalls((tc) => (tc[e.id] ? { ...tc, [e.id]: { ...tc[e.id], status: e.status } } : tc));
        break;
      case "textDelta":
        setMessages((msgs) => {
          const last = msgs[msgs.length - 1];
          if (!last || last.role !== "assistant") return msgs;
          return [...msgs.slice(0, -1), { ...last, text: last.text + e.text }];
        });
        break;
      case "error":
        setMessages((msgs) => {
          const last = msgs[msgs.length - 1];
          if (!last || last.role !== "assistant") return msgs;
          const prefix = last.text ? `${last.text}\n` : "";
          return [...msgs.slice(0, -1), { ...last, text: `${prefix}Error: ${e.message}` }];
        });
        break;
      case "turnDone":
        break;
    }
  }

  async function handleSend() {
    const prompt = input.trim();
    if (!prompt || sending || !canSend) return;
    const outgoing = attachedContext ? `${contextPreface(attachedContext)}\n\n${prompt}` : prompt;
    setInput("");
    setMessages((msgs) => [
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
        <option key={a.kind} value={a.kind} disabled={!a.available}>
          {a.label}
          {a.available ? "" : " (not installed)"}
        </option>
      ))}
    </select>
  );

  return (
    <Drawer open={open} onClose={onClose} title="Assistant" headerActions={agentPicker || undefined} defaultWidth={420}>
      <div className="flex h-full flex-col gap-3">
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
                    : "inline-block whitespace-pre-wrap rounded-md bg-muted px-3 py-2 text-left text-sm"
                }
              >
                {m.text || (m.role === "assistant" && sending ? "…" : "")}
              </div>
              {m.toolCallIds?.map((id) => {
                const tc = toolCalls[id];
                return tc ? <ToolCallCard key={id} tool={tc.tool} args={tc.args} status={tc.status} /> : null;
              })}
            </div>
          ))}
        </div>
        {!canSend && (
          <p className="shrink-0 text-xs text-muted-foreground">
            {selectedAgent
              ? selectedAgent.installUrl
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
    </Drawer>
  );
}
