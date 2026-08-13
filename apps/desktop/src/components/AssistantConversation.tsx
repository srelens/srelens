import React, { forwardRef, useEffect, useImperativeHandle, useRef, useState } from "react";
import { Bot, Check, ChevronDown, ChevronRight, Copy, Paperclip, Sparkles, Wrench } from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Badge, Button, Spinner, TextInput } from "../ui";
import { cancelChat, listAgents, startChat, sendChat, type AgentEvent, type AgentInfo, type ToolStatus } from "../lib/chat";
import { respondToConfirm, type ConfirmRequest } from "../lib/mcpSecurity";
import { getPrompt, listPrompts, type PromptSummary } from "../lib/prompts";
import { listSkills, loadSkill, type SkillMeta } from "../lib/skills";
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
import { relativeTime } from "../lib/relativeTime";
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

/**
 * Preface prepended to the outgoing prompt on the global (multi-context) tab
 * so the agent knows which cluster(s) it may act on — a tool call always
 * needs a context, and with more than one cluster selected the agent has to
 * pick one per call rather than assume the default. Empty selection (Send is
 * disabled in that state, see `canSend`) yields no preface at all.
 */
function multiContextPreface(contexts: string[]): string {
  if (contexts.length === 0) return "";
  if (contexts.length === 1) {
    return `Work in the cluster \`${contexts[0]}\` (the default context). Pass its context to each tool call.\n\n`;
  }
  const list = contexts.map((c) => `\`${c}\``).join(", ");
  return `You may work across these clusters: ${list}. Pass the appropriate context to each tool call.\n\n`;
}

/**
 * Fetches each active skill's body and concatenates them into a guidance
 * block prepended to the outgoing prompt — after any context/multi-context
 * preface, before the user's own text — kept out of the visible transcript
 * exactly like that preface. Empty when no skill is active.
 *
 * Uses `allSettled` rather than `all`: an active skill can go missing (e.g.
 * deleted from disk after being activated in an old, still-open session), and
 * a `loadSkill` rejection must not abort the whole turn — it just drops that
 * one skill's guidance and sends with whatever else loaded.
 */
async function loadSkillsGuidance(names: string[]): Promise<string> {
  if (names.length === 0) return "";
  const results = await Promise.allSettled(names.map((name) => loadSkill(name)));
  const bodies = results.filter((r) => r.status === "fulfilled").map((r) => r.value.body);
  if (bodies.length === 0) return "";
  return `Apply these skills:\n\n${bodies.join("\n\n")}\n\n`;
}

/** Picks the image files out of a paste event's clipboard data — everything
 * else (plain text, HTML) is left for the composer's normal paste handling. */
function extractImageFiles(clipboardData: DataTransfer | null | undefined): File[] {
  if (!clipboardData) return [];
  return Array.from(clipboardData.files ?? []).filter((f) => f.type.startsWith("image/"));
}

/** Reads a `File`'s bytes into a base64 data URI. Split out on its own so a
 * test can drive it with an in-memory `File` — no real filesystem I/O either
 * way, since a `File` picked from an `<input>` or clipboard is already just
 * bytes in memory. */
function readImageFile(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(reader.error ?? new Error("failed to read image file"));
    reader.readAsDataURL(file);
  });
}

/** Strips the `data:image/...;base64,` prefix off a data URI, leaving the
 * raw base64 payload `chat_send`'s `images: Vec<String>` (Task 15) expects.
 * Exported for a direct unit test of the no-prefix branch. */
export function stripDataUri(uri: string): string {
  const i = uri.indexOf(",");
  return i === -1 ? uri : uri.slice(i + 1);
}

/** Persist the agent the user last actually used so a fresh chat defaults back
 * to it instead of always falling to the first-available (Claude). Stored
 * globally (not per-session — that's `Session.agentKind`); guarded so a missing
 * `localStorage` (or a private-mode throw) degrades to the first-available
 * default rather than erroring. */
const LAST_AGENT_KEY = "srelens.assistant.lastAgent";
function loadLastAgent(): string | null {
  try {
    return localStorage.getItem(LAST_AGENT_KEY);
  } catch {
    return null;
  }
}
function saveLastAgent(kind: string) {
  try {
    if (kind) localStorage.setItem(LAST_AGENT_KEY, kind);
  } catch {
    /* ignore — persistence is a convenience, not required */
  }
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
  /** Reasoning/thinking text streamed before the answer — only some agents
   * emit it (Cursor does; Claude/Codex headless don't). Shown in a collapsible
   * "Thoughts" section above the tools and answer. */
  thoughts?: string;
  /** Seconds spent thinking, for the "Thoughts · Ns" label. */
  thoughtSecs?: number;
  /** Data URIs (`data:image/...;base64,...`) attached when this (user)
   * message was sent — the displayable form, rendered inline in the bubble
   * and round-tripped via `StoredMessage.images` so a reloaded session keeps
   * them. Only ever set for `role: "user"`. */
  images?: string[];
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
    const stored: StoredMessage = { id: m.id, role: m.role, text: m.text };
    if (m.images && m.images.length > 0) stored.images = m.images;
    if (toolCalls.length > 0) stored.toolCalls = toolCalls;
    if (m.thoughts) stored.thoughts = m.thoughts;
    if (m.thoughtSecs) stored.thoughtSecs = m.thoughtSecs;
    return stored;
  });
}

/** The inverse of `toStoredMessages` — rebuilds the live `ChatMessage[]` plus
 * a `toolCalls` record from a loaded session's opaque `messages`. */
function fromStoredMessages(stored: StoredMessage[]): { msgs: ChatMessage[]; calls: Record<string, ToolCallState> } {
  const calls: Record<string, ToolCallState> = {};
  const msgs: ChatMessage[] = stored.map((m) => {
    for (const tc of m.toolCalls ?? []) calls[tc.id] = { tool: tc.tool, args: tc.args, status: tc.status };
    const toolCallIds = m.toolCalls?.map((tc) => tc.id);
    const msg: ChatMessage = { id: m.id, role: m.role, text: m.text };
    if (m.images && m.images.length > 0) msg.images = m.images;
    if (toolCallIds?.length) msg.toolCallIds = toolCallIds;
    if (m.thoughts) msg.thoughts = m.thoughts;
    if (m.thoughtSecs) msg.thoughtSecs = m.thoughtSecs;
    return msg;
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
 * A turn's tool calls, folded into one collapsible "Tools · N" row so a busy
 * turn's calls don't crowd out the answer. Collapsed by default; the row shows
 * the count and a running spinner (any call still in flight) or an error badge
 * (any call failed/denied). Expanding reveals each `ToolCallCard`.
 */
function ToolCallGroup({ toolCalls }: { toolCalls: ToolCallState[] }) {
  const [expanded, setExpanded] = useState(false);
  const running = toolCalls.some((tc) => tc.status === null);
  const failed = toolCalls.some((tc) => tc.status === "error" || tc.status === "denied");
  return (
    <div className="rounded-lg border border-border bg-muted/30 text-xs">
      <button
        type="button"
        aria-expanded={expanded}
        aria-label={`Tools (${toolCalls.length})`}
        onClick={() => setExpanded((e) => !e)}
        className="flex w-full items-center gap-1.5 px-2.5 py-1.5 text-left text-muted-foreground hover:text-foreground"
      >
        {expanded ? (
          <ChevronDown aria-hidden="true" className="size-3.5 shrink-0" />
        ) : (
          <ChevronRight aria-hidden="true" className="size-3.5 shrink-0" />
        )}
        <Wrench aria-hidden="true" className="size-3.5 shrink-0" />
        <span className="font-medium text-foreground">Tools</span>
        <span>· {toolCalls.length}</span>
        {running ? (
          <Spinner className="size-3" label="Running" />
        ) : failed ? (
          <Badge variant="danger">error</Badge>
        ) : null}
      </button>
      {expanded && (
        <div className="border-t border-border px-2.5 pb-2">
          {toolCalls.map((tc, i) => (
            <ToolCallCard key={i} tool={tc.tool} args={tc.args} status={tc.status} />
          ))}
        </div>
      )}
    </div>
  );
}

/** Small "Copy" affordance for an assistant answer — copies the raw markdown
 * text and briefly flips to a check. */
function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      type="button"
      aria-label="Copy answer"
      onClick={() => {
        void navigator.clipboard?.writeText(text);
        setCopied(true);
        window.setTimeout(() => setCopied(false), 1500);
      }}
      className="inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 text-xs text-muted-foreground hover:bg-accent hover:text-foreground"
    >
      {copied ? <Check aria-hidden="true" className="size-3.5" /> : <Copy aria-hidden="true" className="size-3.5" />}
      {copied ? "Copied" : "Copy"}
    </button>
  );
}

/**
 * The agent's reasoning for a turn, folded into a collapsible "Thoughts · Ns"
 * row above the tools and answer. Only rendered when the agent actually
 * streamed reasoning: the native agent and Cursor stream it, Codex reports
 * reasoning summaries when the model reasons enough to produce them (the
 * adapter requests them via `model_reasoning_summary`), and Claude headless
 * redacts thinking content entirely (see `claude.rs`), so Claude turns have
 * no Thoughts row. Collapsed by default.
 */
function ThoughtsGroup({ text, secs }: { text: string; secs?: number }) {
  const [expanded, setExpanded] = useState(false);
  return (
    <div className="rounded-lg border border-border bg-muted/20 text-xs">
      <button
        type="button"
        aria-expanded={expanded}
        aria-label="Thoughts"
        onClick={() => setExpanded((e) => !e)}
        className="flex w-full items-center gap-1.5 px-2.5 py-1.5 text-left text-muted-foreground hover:text-foreground"
      >
        {expanded ? (
          <ChevronDown aria-hidden="true" className="size-3.5 shrink-0" />
        ) : (
          <ChevronRight aria-hidden="true" className="size-3.5 shrink-0" />
        )}
        <Sparkles aria-hidden="true" className="size-3.5 shrink-0" />
        <span className="font-medium text-foreground">Thoughts</span>
        {secs ? <span>· {secs}s</span> : null}
      </button>
      {expanded && (
        <div className="whitespace-pre-wrap border-t border-border px-2.5 py-2 leading-relaxed text-muted-foreground">
          {text}
        </div>
      )}
    </div>
  );
}

/**
 * The same `mcp://confirm-request` the modal (`McpConfirmDialog`) answers,
 * rendered inline in the transcript so the approval is visible next to the
 * turn that triggered it. Both views call `respondToConfirm` with the same
 * `id` — the backend resolves whichever answers first and then broadcasts
 * `mcp://confirm-resolved`, which removes the request from BOTH views, so
 * neither a stale card nor a stale modal outlives the decision.
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
 * The `/` slash menu: a dropdown listing srelens's diagnostic prompts (Task
 * 21) and saved skills (Task 23) under their own clearly-headed groups,
 * anchored above the composer's input row so it opens upward without being
 * clipped by the transcript below. Styled to match the `PopoverContent`
 * surface (`ContextMultiSelect`/`HistoryPopover`) even though this isn't a
 * Radix popover itself — it needs to open purely from the composer's input
 * value (a `/`-prefixed token), not a trigger click, so a plain positioned
 * `div` is simpler than fighting Radix's own open/anchor model for that.
 * Picking a prompt renders it into the composer input (`onPickPrompt`);
 * picking a skill just activates it (`onPickSkill`) — see `selectSkill`.
 */
function SlashMenu({
  prompts,
  skills,
  onPickPrompt,
  onPickSkill,
}: {
  prompts: PromptSummary[];
  skills: SkillMeta[];
  onPickPrompt: (p: PromptSummary) => void;
  onPickSkill: (s: SkillMeta) => void;
}) {
  return (
    <div className="absolute bottom-full left-0 z-50 mb-1 max-h-64 w-80 overflow-y-auto rounded-lg border border-border bg-popover p-1 text-popover-foreground shadow-md">
      {prompts.length === 0 && skills.length === 0 ? (
        <p className="px-2 py-1.5 text-xs text-muted-foreground">No matches.</p>
      ) : (
        <>
          {prompts.length > 0 && (
            <div>
              <p className="px-2 pt-1 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
                Prompts
              </p>
              {prompts.map((p) => (
                <button
                  key={p.name}
                  type="button"
                  className="flex w-full flex-col items-start gap-0.5 rounded px-2 py-1.5 text-left hover:bg-accent"
                  onClick={() => onPickPrompt(p)}
                >
                  <span className="font-mono text-xs font-medium">{p.name}</span>
                  <span className="w-full truncate text-xs text-muted-foreground">{p.description}</span>
                </button>
              ))}
            </div>
          )}
          {skills.length > 0 && (
            <div>
              <p className="px-2 pt-1 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
                Skills
              </p>
              {skills.map((s) => (
                <button
                  key={s.name}
                  type="button"
                  className="flex w-full flex-col items-start gap-0.5 rounded px-2 py-1.5 text-left hover:bg-accent"
                  onClick={() => onPickSkill(s)}
                >
                  <span className="font-mono text-xs font-medium">{s.name}</span>
                  <span className="w-full truncate text-xs text-muted-foreground">{s.description}</span>
                </button>
              ))}
            </div>
          )}
        </>
      )}
    </div>
  );
}

/**
 * "Contexts (N)" button opening a checklist of `available` kube contexts,
 * for the global (multi-context) tab only — the drawer keeps its single
 * resource `context` chip instead. Stays open across multiple toggles so
 * several clusters can be picked in one pass.
 */
function ContextMultiSelect({
  available,
  selected,
  onChange,
}: {
  available: string[];
  selected: string[];
  onChange: (next: string[]) => void;
}) {
  function toggle(name: string) {
    const set = new Set(selected);
    if (set.has(name)) set.delete(name);
    else set.add(name);
    onChange([...set]);
  }
  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button variant="secondary" size="xs">
          Contexts ({selected.length})
        </Button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-56 p-1">
        <div role="group" aria-label="Choose contexts">
          {available.map((name) => (
            <label key={name} className="flex items-center gap-2 rounded px-2 py-1.5 text-sm hover:bg-accent">
              <input type="checkbox" checked={selected.includes(name)} onChange={() => toggle(name)} />
              <span className="truncate">{name}</span>
            </label>
          ))}
        </div>
      </PopoverContent>
    </Popover>
  );
}

/**
 * Agent selector — a proper popover dropdown (not a bare native <select>): the
 * trigger shows the current agent with an icon, and the list marks the
 * selected one and disables agents that are not installed or still gated
 * (no kind is gated today — Claude, Codex, and Cursor are all selectable —
 * but the mechanism stays wired for a future agent that isn't ready yet), so
 * an unusable agent can't be picked. `role="combobox"` on the trigger +
 * `role="listbox"/"option"` on the list keep it accessible.
 */
function AgentPicker({
  agents,
  selectedKind,
  onSelect,
}: {
  agents: AgentInfo[];
  selectedKind: string;
  onSelect: (kind: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const current = agents.find((a) => a.kind === selectedKind);
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button
          variant="secondary"
          size="sm"
          role="combobox"
          aria-label="Agent"
          aria-expanded={open}
          aria-haspopup="listbox"
          className="gap-1.5"
        >
          <Bot aria-hidden="true" className="size-3.5 shrink-0" />
          <span className="max-w-[8rem] truncate">{current?.label ?? "Agent"}</span>
          <ChevronDown aria-hidden="true" className="size-3.5 shrink-0 opacity-60" />
        </Button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-56 p-1">
        <div role="listbox" aria-label="Choose agent">
          {agents.map((a) => {
            const disabled = !a.available || a.gated;
            const selected = a.kind === selectedKind;
            return (
              <button
                key={a.kind}
                type="button"
                role="option"
                aria-selected={selected}
                disabled={disabled}
                onClick={() => {
                  onSelect(a.kind);
                  setOpen(false);
                }}
                className="flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-sm hover:bg-accent disabled:pointer-events-none disabled:opacity-50"
              >
                <Check
                  aria-hidden="true"
                  className={`size-3.5 shrink-0 ${selected ? "opacity-100" : "opacity-0"}`}
                />
                <span className="min-w-0 flex-1 truncate">{a.label}</span>
                {!a.available ? (
                  <span className="shrink-0 text-xs text-muted-foreground">
                    {a.kind === "srelens" ? "needs API key" : "not installed"}
                  </span>
                ) : a.gated ? (
                  <span className="shrink-0 text-xs text-muted-foreground">soon</span>
                ) : null}
              </button>
            );
          })}
        </div>
      </PopoverContent>
    </Popover>
  );
}

/**
 * Compact session-history control used by the drawer (and standalone uses of
 * this component) — a single trigger button opens a popover with New Chat
 * plus the recent-sessions list (title + relative time, newest first,
 * click-to-load, always-visible delete). The full tab doesn't render this at
 * all: it renders the equivalent `HistoryRail` instead, driven by the same
 * `sessions`/`onNewChat`/`onSelectSession`/`onDeleteSession` via
 * `AssistantConversationHandle` (see `hideSessionControls`) — one source of
 * truth for the session list either way.
 */
function HistoryPopover({
  sessions,
  onNewChat,
  onSelectSession,
  onDeleteSession,
}: {
  sessions: SessionMeta[];
  onNewChat: () => void;
  onSelectSession: (id: string) => void;
  onDeleteSession: (id: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const now = Date.now();
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button variant="secondary" size="xs">
          History
        </Button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-64 p-1">
        <Button
          variant="ghost"
          size="xs"
          className="w-full justify-start"
          onClick={() => {
            onNewChat();
            setOpen(false);
          }}
        >
          New chat
        </Button>
        {sessions.length === 0 ? (
          <p className="px-2 py-1.5 text-xs text-muted-foreground">No saved chats yet.</p>
        ) : (
          <ul className="mt-1 max-h-72 overflow-y-auto">
            {sessions.map((s) => (
              <li key={s.id} className="flex items-center gap-1 rounded px-2 py-1.5 text-xs hover:bg-accent">
                <button
                  type="button"
                  className="min-w-0 flex-1 truncate text-left"
                  title={s.title}
                  onClick={() => {
                    onSelectSession(s.id);
                    setOpen(false);
                  }}
                >
                  <span className="block truncate">{s.title}</span>
                  <span className="block text-[10px] text-muted-foreground">{relativeTime(s.updatedAt, now)}</span>
                </button>
                <button
                  type="button"
                  aria-label={`Delete ${s.title}`}
                  onClick={() => onDeleteSession(s.id)}
                  className="shrink-0 text-muted-foreground hover:text-foreground"
                >
                  ✕
                </button>
              </li>
            ))}
          </ul>
        )}
      </PopoverContent>
    </Popover>
  );
}

/** Imperative surface a host (the full-tab rail) drives to operate the same
 * session state this component owns internally — see `hideSessionControls`
 * below. */
export interface AssistantConversationHandle {
  newChat: () => void;
  selectSession: (id: string) => void;
  deleteSession: (id: string) => void;
}

/**
 * The assistant conversation: a streamed exchange with the configured coding
 * agent, plus collapsible tool-call cards for anything it invokes. `context`
 * (the resource/namespace the caller had active) is rendered as a removable
 * chip and, while attached, prefaced onto the prompt sent to the agent — it
 * is entirely optional, so this same component serves both a resource-scoped
 * host (the `AssistantDrawer`) and a global one with no resource in scope
 * (the `AssistantTab`). The agent picker is sourced from `listAgents()`; an
 * unavailable agent shows its install link, a gated one (none currently —
 * Claude, Codex, and Cursor are all boxed to srelens's MCP tools and
 * selectable) shows a "coming soon" note instead — either way Send stays
 * disabled.
 *
 * `availableContexts`, when provided, switches this into multi-context mode
 * (the global tab, which has no single resource `context` to attach): a
 * "Contexts (N)" picker lets the user pick one or more kube contexts,
 * rendered as removable chips, prefaced onto the outgoing prompt (see
 * `multiContextPreface`), and required before Send is enabled — a tool call
 * always needs a context. The drawer never passes this prop, so its
 * single-resource `context` chip/preface/Send-gating (Task 10) is untouched.
 *
 * `hideSessionControls` (Task 19's full-tab rail) suppresses this
 * component's own `HistoryPopover` entirely — the host renders a
 * `HistoryRail` (or any other UI) instead, driving the same session state via
 * the imperative handle (`newChat`/`selectSession`/`deleteSession`) and
 * `onSessionsChanged`, so there is exactly one owner of `sessions` (this
 * component) and no risk of two independently-fetched copies drifting apart.
 */
export const AssistantConversation = forwardRef<
  AssistantConversationHandle,
  {
    context?: AssistantContext;
    availableContexts?: string[];
    className?: string;
    hideSessionControls?: boolean;
    onSessionsChanged?: (sessions: SessionMeta[]) => void;
    /** Bumped by the host whenever the Skills panel mutates the skill set (e.g.
     * on panel close), so the slash menu reloads instead of showing a stale
     * list until the whole assistant tab is reopened. */
    skillsRefreshKey?: number;
  }
>(function AssistantConversation(
  { context, availableContexts, className, hideSessionControls, onSessionsChanged, skillsRefreshKey },
  ref,
) {
  const multiContextMode = availableContexts !== undefined;
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [toolCalls, setToolCalls] = useState<Record<string, ToolCallState>>({});
  const [input, setInput] = useState("");
  const [sending, setSending] = useState(false);
  const [agents, setAgents] = useState<AgentInfo[]>([]);
  // Mirrors `agents` so the imperative-handle callbacks (which are captured
  // once, with an empty dep array) read the current list rather than the empty
  // one from first render — otherwise a session reopened from the full-tab
  // rail never restores its saved `agentKind`.
  const agentsRef = useRef<AgentInfo[]>([]);
  const [selectedKind, setSelectedKind] = useState("");
  const [attachedContext, setAttachedContext] = useState<AssistantContext | undefined>(context);
  // Multi-context mode only (`availableContexts` provided) — the kube
  // contexts selected for this global chat, in pick order.
  const [selectedContexts, setSelectedContexts] = useState<string[]>([]);
  const [pendingConfirms, setPendingConfirms] = useState<ConfirmRequest[]>([]);
  // Task 21's `/` slash menu — srelens's diagnostic prompts, loaded once on
  // mount (an empty list on failure just means the menu never has anything
  // to show, not a broken composer). `promptMenuDismissed` lets Escape close
  // the menu even while the input still reads as a `/`-prefixed token; it's
  // reset on every keystroke so the next slash reopens it. `promptError`
  // surfaces a rejected `getPrompt` (e.g. a required arg this menu can't
  // fill) inline, without touching the input.
  const [prompts, setPrompts] = useState<PromptSummary[]>([]);
  const [promptMenuDismissed, setPromptMenuDismissed] = useState(false);
  const [promptError, setPromptError] = useState<string | null>(null);
  // Task 23's skills half of the same slash menu — loaded once on mount,
  // same best-effort treatment as `prompts` above. Picking one (`selectSkill`)
  // adds its name to `activeSkills` (deduped) instead of touching the input;
  // each active skill shows as a removable chip in the composer and, on
  // send, has its body fetched and folded into a guidance block prefaced
  // onto the outgoing prompt (see `loadSkillsGuidance`).
  const [skills, setSkills] = useState<SkillMeta[]>([]);
  const [activeSkills, setActiveSkills] = useState<string[]>([]);
  // Images attached to the in-progress message, as displayable data URIs —
  // cleared on send (after being copied onto the outgoing user `ChatMessage`
  // and, base64-stripped, into `sendChat`'s `images` arg).
  const [pendingImages, setPendingImages] = useState<string[]>([]);
  // How many attached files are still being read into data URIs. Send is
  // disabled while > 0: pressing it mid-read would snapshot the previous
  // `pendingImages`, send without the new attachment, and leave the image
  // queued for the NEXT turn once the read resolves.
  const [pendingImageReads, setPendingImageReads] = useState(0);
  // The single source of truth for the saved-session list — rendered here via
  // `HistoryPopover` unless `hideSessionControls` is set, in which case the
  // host (the full-tab `HistoryRail`) mirrors it through `onSessionsChanged`
  // and drives it back through the imperative handle below.
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
  // Mirrors `sending` so the imperative-handle session actions (captured with
  // empty deps) can see whether a turn is in flight and refuse to switch — a
  // session change mid-stream would let the running turn's `turnDone` persist
  // its transcript under the newly selected session id, corrupting it.
  const sendingRef = useRef(false);
  // Monotonic token for async session loads. Every session action (select /
  // New chat / delete-then-new) bumps it; a `loadSession` that resolves after a
  // newer action fired sees its captured token is stale and drops its result,
  // so a slow read (e.g. an image-heavy session) can't clobber a later
  // selection.
  const loadSeqRef = useRef(0);
  // Set by Stop, read by `handleSend` right before it launches the agent, so a
  // Stop pressed during the async prep window (before any cancellable child
  // exists) still prevents the launch.
  const cancelRequestedRef = useRef(false);
  // Bumped each turn so tool-call state keys are unique per turn. Agents that
  // restart their tool-call ids every turn (Codex reuses `item_1`; the native
  // Gemini path restarts at `gemini-call-0-0`) would otherwise collide across
  // turns — a later turn overwriting an earlier turn's tool card and corrupting
  // the persisted transcript. Prefixing the id with this nonce keeps them
  // distinct. Also passed to `sendChat`/`cancelChat` as the turn generation, so
  // the backend can tell a Stop aimed at this turn from a stale one.
  const turnNonceRef = useRef(0);
  // Set once per disk session, on its first save; cleared by New chat and
  // restored from the loaded value when reopening a session, so re-saving
  // never stomps the original `createdAt`.
  const createdAtRef = useRef<number | null>(null);
  // The agent CLI's OWN session id for this conversation (what `sendChat`
  // last returned), tagged with the agent kind it belongs to: a Claude id
  // passed to Cursor's `--resume` (or vice versa) would error, so switching
  // agents mid-conversation must start that CLI fresh rather than resume a
  // foreign id. Persisted as `cliSessionId` and restored on reopen, so a
  // reopened conversation continues with real context (the CLIs keep their
  // session transcripts on disk across app restarts).
  const cliSessionRef = useRef<{ kind: string; id: string } | null>(null);
  // Chain of pending disk saves. `persistSession` enqueues onto it, and
  // session actions that touch the same files (delete, load) await it first —
  // otherwise a slow save still flushing after a turn settles could race
  // `chat_history_delete` and recreate the just-deleted file or index entry.
  const persistChainRef = useRef<Promise<void>>(Promise.resolve());

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
  // When the current turn began streaming reasoning, so the Thoughts section
  // can show how long it thought once the thinking ends.
  const thinkingStartRef = useRef<number | null>(null);
  // The agent kind the in-flight turn runs on. Codex delivers each reasoning
  // burst as one COMPLETED summary item — it arrives after the thinking
  // already happened, so wall-clock timing across its events would label a
  // tens-of-seconds burst "Thoughts · 1s". For codex the timer never starts
  // and the label shows no duration; agents that stream true deltas
  // (native, Cursor) keep it.
  const turnKindRef = useRef<string | null>(null);

  // This component only exists while its host (the drawer or the tab) is
  // showing it, so a subscription made on mount already covers "each time
  // this view becomes visible" — no visibility flag to gate on here.
  useEffect(() => {
    const unlisten = listen<ConfirmRequest>("mcp://confirm-request", (event) => {
      setPendingConfirms((q) => [...q, event.payload]);
    });
    // Answered anywhere (this card, the app-wide modal) or timed out — the
    // backend announces it and the inline card must go, or a stale approval
    // prompt would sit in the transcript forever.
    const unlistenResolved = listen<{ id: string }>("mcp://confirm-resolved", (event) => {
      setPendingConfirms((q) => q.filter((r) => r.id !== event.payload.id));
    });
    return () => {
      void unlisten.then((f) => f());
      void unlistenResolved.then((f) => f());
      // If the conversation unmounts mid-turn (drawer/tab closed, or the user
      // switched to another tab), the backend turn and its event subscription
      // would otherwise keep running invisibly — burning quota and invoking MCP
      // tools with no one watching. Cancel the in-flight turn on the way out.
      if (sendingRef.current) {
        // Unmount can land while `startChat()` is still pending — no session
        // id exists yet, so there's nothing to cancel on the backend. Set the
        // cancel flag unconditionally: `handleSend` re-checks it right before
        // launching, so the prep resolving after unmount doesn't start an
        // invisible turn. When a session does exist, also stop the backend.
        cancelRequestedRef.current = true;
        if (sessionRef.current) void cancelChat(sessionRef.current, turnNonceRef.current);
      }
    };
  }, []);

  // Load the slash menu's prompt list once on mount — same "best-effort,
  // empty on failure" treatment as `listAgents`/`listSessions` above.
  useEffect(() => {
    listPrompts()
      .then(setPrompts)
      .catch(() => setPrompts([]));
  }, []);

  // Load the slash menu's skills list — same "best-effort, empty on failure"
  // treatment as `listPrompts` above. Re-runs when `skillsRefreshKey` changes
  // so creating/renaming/deleting a skill in the Skills panel is reflected in
  // the slash menu without reopening the whole assistant tab.
  useEffect(() => {
    listSkills()
      .then(setSkills)
      .catch(() => setSkills([]));
  }, [skillsRefreshKey]);

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
    function refreshAgents() {
      listAgents()
        .then((list) => {
          setAgents(list);
          agentsRef.current = list;
          // Prefer the last-used agent (if it's still installed and selectable),
          // otherwise the first available non-gated one.
          const stored = loadLastAgent();
          const lastUsed = list.find((a) => a.kind === stored && a.available && !a.gated);
          const firstAvailable = list.find((a) => a.available && !a.gated) ?? list.find((a) => a.available) ?? list[0];
          setSelectedKind((lastUsed ?? firstAvailable)?.kind ?? "");
        })
        .catch(() => {
          setAgents([]);
          agentsRef.current = [];
          setSelectedKind("");
        });
    }
    refreshAgents();
    // This view can mount UNDER the VaultGate (a restored Assistant tab):
    // the first fetch then sees a locked vault — no API keys, native agent
    // unavailable. Re-list when the gate reports the vault unlocked.
    window.addEventListener("srelens:vault-unlocked", refreshAgents);
    return () => window.removeEventListener("srelens:vault-unlocked", refreshAgents);
  }, []);

  // Load the saved-session picker once on mount — `listSessions` already
  // returns newest-first, so this renders in that order verbatim.
  useEffect(() => {
    listSessions()
      .then(setSessions)
      .catch(() => setSessions([]));
  }, []);

  // Mirror every change to `sessions` (initial load, a save, a delete) out to
  // the host — the full-tab rail's only view of the list, so it never fetches
  // or stores its own copy.
  useEffect(() => {
    onSessionsChanged?.(sessions);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sessions]);

  const selectedAgent = agents.find((a) => a.kind === selectedKind);
  const agentPath = selectedAgent?.path ?? "";
  const agentReady = !!selectedAgent?.available && !selectedAgent?.gated;
  // In multi-context mode a tool call always needs a context, so Send stays
  // disabled until at least one is picked; the drawer (single resource
  // `context`, no `availableContexts`) is never gated on this.
  const canSend = agentReady && (!multiContextMode || selectedContexts.length > 0) && pendingImageReads === 0;

  // The slash menu is open exactly when the composer holds "/" or a
  // `/`-prefixed token with no whitespace (still mid-typing a prompt name),
  // there's at least one prompt to offer, and the user hasn't dismissed it
  // (Escape) since the last keystroke. The token after the slash narrows the
  // list by a case-insensitive substring match on the prompt name; an empty
  // token (just "/") shows everything.
  const slashMatch = /^\/(\S*)$/.exec(input);
  const promptMenuOpen =
    slashMatch !== null && !promptMenuDismissed && (prompts.length > 0 || skills.length > 0);
  const promptQuery = (slashMatch?.[1] ?? "").toLowerCase();
  const filteredPrompts = promptQuery
    ? prompts.filter((p) => p.name.toLowerCase().includes(promptQuery))
    : prompts;
  const filteredSkills = promptQuery
    ? skills.filter((s) => s.name.toLowerCase().includes(promptQuery))
    : skills;

  function handleInputChange(value: string) {
    setInput(value);
    setPromptMenuDismissed(false);
    setPromptError(null);
  }

  function handleComposerEscape() {
    if (promptMenuOpen) setPromptMenuDismissed(true);
  }

  /** Render `p` (`context` from the multi-context selection or the drawer's
   * resource context, empty if neither applies) and drop the result straight
   * into the composer for review — never auto-sent. A rejection (e.g. a
   * required arg this menu doesn't know how to fill) surfaces inline and
   * leaves the input untouched, rather than crashing or clobbering it. */
  async function selectPrompt(p: PromptSummary) {
    const ctx = multiContextMode ? (selectedContexts[0] ?? "") : (attachedContext?.context ?? "");
    try {
      const text = await getPrompt(p.name, { context: ctx });
      setInput(text);
      setPromptMenuDismissed(true);
      setPromptError(null);
    } catch (e) {
      setPromptError(e instanceof Error ? e.message : String(e));
    }
  }

  /** Activates a skill from the slash menu — unlike `selectPrompt`, this
   * never touches the composer input: it just adds the skill's name to
   * `activeSkills` (deduped) so it renders as a removable chip and gets
   * folded into the next turn's guidance block (see `loadSkillsGuidance`).
   * The menu is dismissed the same way Escape dismisses it, since picking a
   * skill doesn't change `input` and so wouldn't otherwise close the menu. */
  function selectSkill(s: SkillMeta) {
    setActiveSkills((prev) => (prev.includes(s.name) ? prev : [...prev, s.name]));
    setPromptMenuDismissed(true);
  }

  function removeSkill(name: string) {
    setActiveSkills((prev) => prev.filter((n) => n !== name));
  }

  /** Serialize the current transcript and save it to disk. Best-effort: a
   * failed save is swallowed rather than surfaced, since it must never break
   * the live chat the user is actually looking at. Saves are queued on
   * `persistChainRef` so at most one write is in flight and delete/load can
   * wait for all queued writes to land (they never reject — see the catch).
   *
   * The COMPLETE session — metadata included — is built synchronously here,
   * at enqueue time. A queued callback that read `createdAtRef`, contexts,
   * skills, or the agent pick when it eventually ran could pick up the NEXT
   * conversation's state (New chat is allowed the moment sending ends and
   * doesn't await this chain), stamping the old session with the new one's
   * metadata — or seeding the new conversation with the old `createdAt`. */
  function persistSession(msgs: ChatMessage[], calls: Record<string, ToolCallState>): Promise<void> {
    const id = sessionRef.current;
    if (!id) return Promise.resolve(); // nothing sent yet this conversation — no id to save under
    const now = Date.now();
    if (createdAtRef.current === null) createdAtRef.current = now;
    const session: Session = {
      id,
      title: deriveTitle(msgs),
      createdAt: createdAtRef.current,
      updatedAt: now,
      // Multi-context mode (the global tab) persists the real multi-select;
      // the drawer keeps remembering the one resource chip (if any) that was
      // attached when the turn ran.
      contexts: multiContextMode ? selectedContexts : attachedContext?.context ? [attachedContext.context] : [],
      skills: activeSkills,
      // The agent this conversation ran on, so reopening it restores the same
      // pick in the composer.
      agentKind: selectedKind || null,
      // The agent CLI's own session id (`sendChat`'s return), saved only
      // when it belongs to the agent this conversation is stamped with —
      // an id from a different CLI would make the restored conversation
      // pass a foreign id to `--resume` and error its first turn.
      cliSessionId: cliSessionRef.current?.kind === (selectedKind || null) ? cliSessionRef.current.id : null,
      messages: toStoredMessages(msgs, calls),
    };
    const next = persistChainRef.current.then(() => persistSessionNow(session));
    persistChainRef.current = next;
    return next;
  }

  /** The queued IO half: writes an already-snapshotted session and refreshes
   * the picker's metadata list. Reads no component state — see above. */
  async function persistSessionNow(session: Session) {
    try {
      await saveSession(session);
      setSessions((prev) => {
        const meta: SessionMeta = { id: session.id, title: session.title, createdAt: session.createdAt, updatedAt: session.updatedAt };
        return [meta, ...prev.filter((s) => s.id !== session.id)].sort((a, b) => b.updatedAt - a.updatedAt);
      });
    } catch {
      // Disk persistence is a convenience, not the primary function — the
      // live conversation just keeps going.
    }
  }

  /** Clear the transcript and drop the channel/disk session id so the next
   * `handleSend` mints a fresh one via `startChat()`. */
  function onNewChat() {
    // Refuse to switch away mid-turn: the running turn's listener would keep
    // applying events and its `turnDone` would persist a mixed transcript under
    // the wrong session. Stop the turn first, or wait for it to finish.
    if (sendingRef.current) return;
    // Invalidate any in-flight session load so its result can't land after this
    // reset and repopulate the transcript we're clearing.
    loadSeqRef.current++;
    setMessagesTracked(() => []);
    setToolCallsTracked(() => ({}));
    setInput("");
    sessionRef.current = null;
    createdAtRef.current = null;
    // A fresh conversation must start a fresh CLI session — carrying the old
    // id would silently resume the previous conversation's context.
    cliSessionRef.current = null;
    // Composer state is per-conversation, not global — without resetting
    // these, a "New chat" (or deleting the currently-open session, which
    // routes through here) would silently carry the old session's active
    // skills/pending images/selected contexts into the brand-new one, and
    // they'd get folded into the very next outgoing prompt and persisted
    // under the new session id.
    setActiveSkills([]);
    setPendingImages([]);
    setSelectedContexts([]);
    setPromptError(null);
  }

  /** Load a saved session and replay it into state read-only — no event
   * stream involved, just the transcript as it was last saved. Continuing
   * to type and send afterward appends further turns onto the same disk
   * session, resuming the CLI's own session via the restored
   * `cliSessionId` (see `cliSessionRef`). */
  async function onSelectSession(id: string) {
    // See onNewChat: don't swap sessions while a turn is streaming, or its
    // events would land in (and be saved under) the newly loaded session.
    if (sendingRef.current) return;
    const seq = ++loadSeqRef.current;
    try {
      // Read behind any queued save so a session reopened right after its
      // turn finished gets the just-written transcript, not the prior one.
      await persistChainRef.current;
      const session = await loadSession(id);
      // A newer session action fired while this read was in flight — discard
      // this now-stale result rather than overwriting the newer selection.
      if (loadSeqRef.current !== seq) return;
      const { msgs, calls } = fromStoredMessages(session.messages as unknown as StoredMessage[]);
      setMessagesTracked(() => msgs);
      setToolCallsTracked(() => calls);
      nextId.current = msgs.reduce((max, m) => Math.max(max, m.id + 1), 0);
      sessionRef.current = session.id;
      createdAtRef.current = session.createdAt;
      // Restore the CLI session id under the agent kind it was saved with so
      // the next send `--resume`s it — the CLIs keep session transcripts on
      // disk, so this works across app restarts too. Older saves (or native/
      // Codex conversations) have `null` here and simply start fresh.
      cliSessionRef.current =
        session.cliSessionId && session.agentKind
          ? { kind: session.agentKind, id: session.cliSessionId }
          : null;
      // Restore the global tab's multi-select from what was persisted;
      // meaningless in drawer mode, where the resource `context` prop drives
      // `attachedContext` instead and this state is never read.
      setSelectedContexts(session.contexts ?? []);
      // Restore the activated-skills chips (Task 23) the same way.
      setActiveSkills(session.skills ?? []);
      // Restore the agent the conversation used, as long as it's still a known
      // agent (otherwise keep the current pick rather than blanking the picker).
      // Read via the ref, not `agents` state: `onSelectSession` is captured in
      // the imperative handle with an empty dep array, so the `agents` closure
      // would be the empty first-render value.
      if (session.agentKind && agentsRef.current.some((a) => a.kind === session.agentKind)) {
        setSelectedKind(session.agentKind);
      }
    } catch {
      // Leave the current conversation untouched on a bad load.
    }
  }

  /** Delete a saved session from disk and drop it from the picker. If it was
   * the one currently open, this behaves like New chat. */
  async function onDeleteSession(id: string) {
    // Don't delete mid-turn: if it's the open session, the running turn would
    // re-save it right after; if it isn't, blocking is the safe simple rule.
    if (sendingRef.current) return;
    // A save from the just-finished turn may still be flushing (the guard
    // above only covers the turn itself) — let every queued write land before
    // deleting, or the slow save would recreate the file we're removing.
    await persistChainRef.current;
    // A turn that started while we waited will re-save its session — bail out
    // rather than deleting under it, same rule as the guard above.
    if (sendingRef.current) return;
    try {
      await deleteSessionCmd(id);
    } catch {
      // Best-effort — still drop it from the visible list below.
    }
    setSessions((prev) => prev.filter((s) => s.id !== id));
    if (sessionRef.current === id) onNewChat();
  }

  // Exposes the same three handlers the internal `HistoryPopover` calls so a
  // host that hides it (the full-tab `HistoryRail`, via `hideSessionControls`)
  // can drive the identical session state instead of keeping its own copy.
  useImperativeHandle(
    ref,
    () => ({
      newChat: onNewChat,
      selectSession: (id: string) => void onSelectSession(id),
      deleteSession: (id: string) => void onDeleteSession(id),
    }),
    // onNewChat/onSelectSession/onDeleteSession close over state via refs and
    // setState updaters, not over any value that changes across renders, so
    // there's nothing meaningful to list as a dependency here — the handle
    // only needs to be (re)built once.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [],
  );

  /** Reads each file and appends its data URI to `pendingImages` as it
   * resolves — a non-image or unreadable file is silently skipped, since
   * there's nothing useful to attach for it. */
  async function addImageFiles(files: File[]) {
    const images = files.filter((f) => f.type.startsWith("image/"));
    if (images.length === 0) return;
    // Counted up-front and down per file, so Send stays disabled until every
    // selected/pasted file has either attached or been skipped.
    setPendingImageReads((n) => n + images.length);
    for (const file of images) {
      try {
        const dataUri = await readImageFile(file);
        setPendingImages((imgs) => [...imgs, dataUri]);
      } catch {
        // Unreadable file — nothing to attach, silently skipped.
      } finally {
        setPendingImageReads((n) => n - 1);
      }
    }
  }

  /** The attach control's hidden `<input type="file">`. */
  function onAttachFiles(e: React.ChangeEvent<HTMLInputElement>) {
    const files = Array.from(e.target.files ?? []);
    e.target.value = ""; // allow re-selecting the same file later
    void addImageFiles(files);
  }

  /** Cmd/Ctrl-V of an image anywhere in the composer. Only intercepts the
   * paste when it actually carries an image — plain-text paste into the
   * input still behaves normally. */
  function onComposerPaste(e: React.ClipboardEvent<HTMLDivElement>) {
    const files = extractImageFiles(e.clipboardData);
    if (files.length === 0) return;
    e.preventDefault();
    void addImageFiles(files);
  }

  function removePendingImage(index: number) {
    setPendingImages((imgs) => imgs.filter((_, i) => i !== index));
  }

  function applyEvent(e: AgentEvent) {
    // Thinking ends the moment any non-thinking event arrives — stamp how long
    // it thought onto the current assistant message (only if it thought at all).
    function endThinking() {
      if (thinkingStartRef.current === null) return;
      const secs = Math.max(1, Math.round((Date.now() - thinkingStartRef.current) / 1000));
      thinkingStartRef.current = null;
      setMessagesTracked((msgs) => {
        const last = msgs[msgs.length - 1];
        if (!last || last.role !== "assistant" || !last.thoughts) return msgs;
        return [...msgs.slice(0, -1), { ...last, thoughtSecs: secs }];
      });
    }
    switch (e.type) {
      case "thinking":
        // No timer for codex — see `turnKindRef`.
        if (turnKindRef.current !== "codex" && thinkingStartRef.current === null)
          thinkingStartRef.current = Date.now();
        setMessagesTracked((msgs) => {
          const last = msgs[msgs.length - 1];
          if (!last || last.role !== "assistant") return msgs;
          return [...msgs.slice(0, -1), { ...last, thoughts: (last.thoughts ?? "") + e.text }];
        });
        break;
      case "toolCallStart": {
        endThinking();
        toolEventSincePendingDelta.current = true;
        // Namespace the id by turn so a later turn's reused id can't overwrite
        // this turn's card (see `turnNonceRef`).
        const key = `${turnNonceRef.current}#${e.id}`;
        setToolCallsTracked((tc) => ({ ...tc, [key]: { tool: e.tool, args: e.args, status: null } }));
        setMessagesTracked((msgs) => {
          const last = msgs[msgs.length - 1];
          if (!last || last.role !== "assistant") return msgs;
          return [...msgs.slice(0, -1), { ...last, toolCallIds: [...(last.toolCallIds ?? []), key] }];
        });
        break;
      }
      case "toolResult": {
        toolEventSincePendingDelta.current = true;
        const key = `${turnNonceRef.current}#${e.id}`;
        setToolCallsTracked((tc) => (tc[key] ? { ...tc, [key]: { ...tc[key], status: e.status } } : tc));
        break;
      }
      case "textDelta": {
        endThinking();
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
        setMessagesTracked((msgs) => {
          const error = { id: nextId.current++, role: "error" as const, text: e.message };
          const last = msgs[msgs.length - 1];
          // An ADVISORY error (e.g. "attachments aren't supported") arrives
          // before any streaming and the turn then proceeds. Appending it
          // after the untouched assistant placeholder would orphan the
          // placeholder — every later `textDelta` targets the LAST message —
          // and silently discard the whole reply. Slot the error in front of
          // a still-pristine placeholder so the stream keeps its target; a
          // terminal error (placeholder already streamed into) appends.
          const pristine =
            last && last.role === "assistant" && last.text === "" && !last.toolCallIds && !last.thoughts;
          return pristine ? [...msgs.slice(0, -1), error, last] : [...msgs, error];
        });
        break;
      case "turnDone":
        endThinking();
        // A Stop (or an agent crash) can end the turn while calls are still
        // awaiting their toolResult. Left `null`, they'd render — and be
        // persisted, and re-render after reload — as running forever. They
        // didn't complete: settle them as errored before the terminal save.
        // `setToolCallsTracked` updates `toolCallsRef` synchronously, so the
        // persist below sees the settled statuses.
        setToolCallsTracked((tc) => {
          if (!Object.values(tc).some((c) => c.status === null)) return tc;
          return Object.fromEntries(
            Object.entries(tc).map(([k, c]) => [k, c.status === null ? { ...c, status: "error" as const } : c]),
          );
        });
        void persistSession(messagesRef.current, toolCallsRef.current);
        break;
    }
  }

  async function handleSend() {
    const prompt = input.trim();
    if (!prompt || sending || !canSend) return;
    // A session load may still be in flight (the old conversation stays
    // visible while `loadSession` reads). Invalidate it: resolving mid-turn
    // would swap the transcript and `sessionRef` under this send, and the
    // turn's events would append to — and persist under — the wrong session.
    loadSeqRef.current++;
    // The context/multi-context preface, still ending in a blank line when
    // non-empty — the skills guidance block (fetched below, once the turn is
    // actually committed to sending) slots in after it and before `prompt`,
    // same as the preface: never part of the visible user bubble, which
    // always shows `prompt` alone.
    const preface = multiContextMode
      ? multiContextPreface(selectedContexts)
      : attachedContext
        ? `${contextPreface(attachedContext)}\n\n`
        : "";
    const attachedImages = pendingImages;
    const rawImages = attachedImages.map(stripDataUri);
    setInput("");
    setPendingImages([]);
    setMessagesTracked((msgs) => [
      ...msgs,
      {
        id: nextId.current++,
        role: "user",
        text: prompt,
        ...(attachedImages.length > 0 ? { images: attachedImages } : {}),
      },
      { id: nextId.current++, role: "assistant", text: "" },
    ]);
    setSending(true);
    sendingRef.current = true;
    // New turn: bump the tool-call key namespace (see `turnNonceRef`).
    turnNonceRef.current += 1;
    // Fresh turn: clear any Stop left set from a prior turn. A Stop pressed
    // during the async prep below (startChat / skills load) sets this, and we
    // check it right before launching so the agent isn't started after all.
    cancelRequestedRef.current = false;
    try {
      let session = sessionRef.current;
      if (!session) {
        session = await startChat();
        sessionRef.current = session;
      }
      const guidance = await loadSkillsGuidance(activeSkills);
      const outgoing = `${preface}${guidance}${prompt}`;
      const usedKind = selectedAgent?.kind ?? selectedKind;
      // Stop was pressed while we were preparing, before any child existed for
      // `cancelChat` to kill. Honor it: don't launch the agent, and drop the
      // empty assistant placeholder so no blank reply lingers.
      if (cancelRequestedRef.current) {
        setMessagesTracked((msgs) => {
          const last = msgs[msgs.length - 1];
          return last && last.role === "assistant" && last.text === "" ? msgs.slice(0, -1) : msgs;
        });
        return;
      }
      saveLastAgent(usedKind); // remember what was actually used for the next fresh chat
      turnKindRef.current = usedKind; // drives the Thoughts timing decision — see the ref
      // Resume the CLI's own session only when this turn runs on the same
      // agent the stored id came from (see `cliSessionRef`).
      const resume = cliSessionRef.current?.kind === usedKind ? cliSessionRef.current.id : null;
      const cliId = await sendChat(session, outgoing, agentPath, applyEvent, rawImages, usedKind, turnNonceRef.current, resume);
      // The return IS what to store now (see chat_send): a cancelled or
      // drifty-but-clean turn echoes the resumed id back, so `null` with an
      // id on hand always means CLEAR — either the turn ran on an agent
      // with no id (native, Codex: the stored session is now missing turns
      // the transcript visibly contains) or the resume itself crashed (the
      // CLI lost the session; keeping the id would retry the same failing
      // --resume forever). A transport rejection skips this entirely (the
      // catch below) and keeps the stored id.
      const prev = cliSessionRef.current;
      if (cliId ? prev?.id !== cliId || prev?.kind !== usedKind : prev !== null) {
        cliSessionRef.current = cliId ? { kind: usedKind, id: cliId } : null;
        // The turnDone-triggered save snapshots at the moment the event
        // streams — BEFORE `sendChat` resolves with this id — so that save
        // carried the previous turn's id. Queue one more so the transcript
        // on disk gets the id the next turn must resume.
        void persistSession(messagesRef.current, toolCallsRef.current);
      }
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
      sendingRef.current = false;
    }
  }

  /** Cancels the in-flight turn. `handleSend`'s own `finally` clears
   * `sending` once the (now-cancelled) `sendChat` call settles — nothing
   * further to do here besides asking the backend to stop. */
  function handleStop() {
    // Set unconditionally: if Stop lands while `handleSend` is still preparing
    // (before a child exists, so the `cancelChat` below would be a no-op), this
    // flag makes `handleSend` skip the launch entirely. The turn nonce tells
    // the backend which send this Stop is aimed at, so a cancel that beats
    // `chat_send` to the backend is honored by that turn and no other.
    cancelRequestedRef.current = true;
    const session = sessionRef.current;
    if (session) void cancelChat(session, turnNonceRef.current);
  }

  const agentPicker = agents.length > 0 && (
    <AgentPicker
      agents={agents}
      selectedKind={selectedKind}
      onSelect={(kind) => {
        setSelectedKind(kind);
        saveLastAgent(kind);
      }}
    />
  );

  return (
    <div className={`flex h-full flex-col gap-3${className ? ` ${className}` : ""}`}>
      {!hideSessionControls && (
        <div className="flex shrink-0 items-center justify-end">
          <HistoryPopover
            sessions={sessions}
            onNewChat={onNewChat}
            onSelectSession={(id) => void onSelectSession(id)}
            onDeleteSession={(id) => void onDeleteSession(id)}
          />
        </div>
      )}
      <div className="flex-1 overflow-y-auto">
        {messages.length === 0 && pendingConfirms.length === 0 ? (
          <div className="flex h-full flex-col items-center justify-center gap-4 px-6 text-center">
            <div className="flex size-14 items-center justify-center rounded-2xl border border-border bg-muted/40 text-muted-foreground">
              <Sparkles aria-hidden="true" className="size-7" />
            </div>
            <div className="space-y-1.5">
              <p className="text-lg font-semibold text-foreground">Your Kubernetes assistant</p>
              <p className="max-w-sm text-sm text-muted-foreground">
                Ask about this cluster to get started. Type <span className="font-mono text-foreground">/</span> for
                prompts &amp; skills.
              </p>
            </div>
          </div>
        ) : (
          <div className="mx-auto w-full max-w-3xl space-y-6 px-4 py-6">
            {messages.map((m) => (
              <div key={m.id}>
                {m.role === "user" ? (
                  <div className="rounded-xl border border-border bg-muted/40 px-4 py-3 text-sm">
                    <p className="whitespace-pre-wrap">{m.text}</p>
                    {m.images && m.images.length > 0 && (
                      <div className="mt-2 flex flex-wrap gap-2">
                        {m.images.map((src, i) => (
                          <img
                            key={i}
                            src={src}
                            alt={`Attached image ${i + 1}`}
                            className="h-20 w-20 rounded-md border border-border object-cover"
                          />
                        ))}
                      </div>
                    )}
                  </div>
                ) : m.role === "error" ? (
                  <div className="whitespace-pre-wrap rounded-xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
                    {m.text}
                  </div>
                ) : (
                  <div className="space-y-2">
                    {m.thoughts ? <ThoughtsGroup text={m.thoughts} secs={m.thoughtSecs} /> : null}
                    {(() => {
                      const calls = (m.toolCallIds ?? [])
                        .map((id) => toolCalls[id])
                        .filter((tc): tc is ToolCallState => Boolean(tc));
                      return calls.length > 0 ? <ToolCallGroup toolCalls={calls} /> : null;
                    })()}
                    {m.text ? (
                      <div className="group/answer relative">
                        <div className="text-sm leading-relaxed">
                          <AssistantMarkdown text={m.text} />
                        </div>
                        <div className="mt-1 opacity-0 transition-opacity group-hover/answer:opacity-100">
                          <CopyButton text={m.text} />
                        </div>
                      </div>
                    ) : sending ? (
                      <span className="text-sm text-muted-foreground">…</span>
                    ) : null}
                  </div>
                )}
              </div>
            ))}
            {pendingConfirms.map((req) => (
              <ConfirmCard key={req.id} request={req} onAnswer={answerConfirm} />
            ))}
          </div>
        )}
      </div>
      <div className="mx-auto w-full max-w-3xl shrink-0 px-4 pb-4">
        {!agentReady && (
          <p className="mb-2 px-1 text-xs text-muted-foreground">
            {selectedAgent
              ? selectedAgent.gated
                ? `${selectedAgent.label} support is coming — use Claude for now.`
                : selectedAgent.kind === "srelens"
                  ? "Add an API key for the srelens agent in Settings → Assistant to use it."
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
        {/* Composer card: a chip row (context / skills / pending images) and a
            borderless input sit above a compact toolbar (attach, agent picker,
            contexts, Send/Stop) — one grouped surface, Cursor-style. */}
        <div
          data-testid="assistant-composer"
          className="flex flex-col gap-2 rounded-2xl border border-border bg-card p-2 shadow-sm transition-colors focus-within:border-ring/70"
          onPaste={onComposerPaste}
        >
          {(attachedContext ||
            selectedContexts.length > 0 ||
            pendingImages.length > 0 ||
            activeSkills.length > 0) && (
            <div className="flex flex-wrap items-center gap-1.5 px-1 pt-1">
              {attachedContext && (
                <div className="flex items-center gap-2 rounded-md border border-border bg-muted/40 px-2.5 py-1 text-xs">
                  <span className="min-w-0 max-w-[16rem] truncate">{formatContext(attachedContext)}</span>
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
              {selectedContexts.map((name) => (
                <span
                  key={name}
                  className="inline-flex max-w-[10rem] items-center gap-1 rounded-md border border-border bg-muted/40 px-2 py-1 text-xs"
                >
                  <span className="truncate">{name}</span>
                  <button
                    type="button"
                    aria-label={`Remove ${name}`}
                    onClick={() => setSelectedContexts((cs) => cs.filter((c) => c !== name))}
                    className="text-muted-foreground hover:text-foreground"
                  >
                    ✕
                  </button>
                </span>
              ))}
              {pendingImages.map((src, i) => (
                <span key={i} className="relative inline-flex">
                  <img
                    src={src}
                    alt={`Pending image ${i + 1}`}
                    className="h-12 w-12 rounded-md border border-border object-cover"
                  />
                  <button
                    type="button"
                    aria-label={`Remove image ${i + 1}`}
                    onClick={() => removePendingImage(i)}
                    className="absolute -right-1 -top-1 flex h-4 w-4 items-center justify-center rounded-full border border-border bg-background text-[10px] leading-none text-muted-foreground hover:text-foreground"
                  >
                    ✕
                  </button>
                </span>
              ))}
              {activeSkills.map((name) => (
                <span
                  key={name}
                  className="inline-flex max-w-[10rem] items-center gap-1 rounded-md border border-primary/30 bg-primary/10 px-2 py-1 text-xs text-primary"
                >
                  <span className="truncate">{name}</span>
                  <button
                    type="button"
                    aria-label={`Remove skill ${name}`}
                    onClick={() => removeSkill(name)}
                    className="opacity-70 hover:opacity-100"
                  >
                    ✕
                  </button>
                </span>
              ))}
            </div>
          )}
          {promptError && <p className="px-1 text-xs text-destructive">{promptError}</p>}
          <div className="relative">
            {promptMenuOpen && (
              <SlashMenu
                prompts={filteredPrompts}
                skills={filteredSkills}
                onPickPrompt={(p) => void selectPrompt(p)}
                onPickSkill={selectSkill}
              />
            )}
            <TextInput
              value={input}
              onValueChange={handleInputChange}
              onEnter={() => {
                if (!promptMenuOpen) handleSend();
              }}
              onEscape={handleComposerEscape}
              placeholder="Ask about this cluster…   /  for prompts & skills"
              disabled={sending}
              className="w-full border-0 bg-transparent px-2 py-2 shadow-none focus-visible:ring-0 focus-visible:ring-offset-0"
            />
          </div>
          <div className="flex items-center gap-1">
            <label
              title="Attach image"
              className="flex size-8 shrink-0 cursor-pointer items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground"
            >
              <Paperclip aria-hidden="true" className="size-4" />
              <input
                type="file"
                accept="image/*"
                multiple
                aria-label="Attach image"
                onChange={onAttachFiles}
                disabled={sending}
                className="hidden"
              />
            </label>
            {agentPicker}
            {availableContexts !== undefined && (
              <ContextMultiSelect available={availableContexts} selected={selectedContexts} onChange={setSelectedContexts} />
            )}
            <div className="flex-1" />
            <Button
              variant={sending ? "secondary" : "primary"}
              size="sm"
              onClick={sending ? handleStop : handleSend}
              disabled={!sending && (!input.trim() || !canSend)}
            >
              {sending ? "Stop" : "Send"}
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
});
