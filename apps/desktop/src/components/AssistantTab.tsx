import React, { useRef, useState } from "react";
import { ChevronLeft, ChevronRight, MessageSquarePlus, Plus, Trash2, Wrench } from "lucide-react";
import { Button } from "../ui";
import { relativeTime } from "../lib/relativeTime";
import type { SessionMeta } from "../lib/chatHistory";
import { AssistantConversation, type AssistantConversationHandle } from "./AssistantConversation";
import { SkillsPanel } from "./SkillsPanel";

/**
 * Left history rail for the full-tab assistant: New Chat plus the saved
 * sessions (title + relative time, newest first — `sessions` already arrives
 * in that order from `AssistantConversation`'s `onSessionsChanged`), click to
 * load, hover to reveal delete. Collapses to a thin strip so the transcript
 * can reclaim the width. The three session actions are forwarded straight
 * through to `AssistantConversation` via its imperative handle — this
 * component holds no session state of its own, only the mirrored list it
 * renders. A "Skills" footer button (Task 22) opens `SkillsPanel`, owned by
 * the parent `AssistantTab` since the panel is a modal over the whole tab,
 * not scoped to the rail.
 */
function HistoryRail({
  collapsed,
  onToggleCollapsed,
  sessions,
  onNewChat,
  onSelectSession,
  onDeleteSession,
  onOpenSkills,
}: {
  collapsed: boolean;
  onToggleCollapsed: () => void;
  sessions: SessionMeta[];
  onNewChat: () => void;
  onSelectSession: (id: string) => void;
  onDeleteSession: (id: string) => void;
  onOpenSkills: () => void;
}) {
  if (collapsed) {
    return (
      <div className="flex w-11 shrink-0 flex-col items-center gap-1 border-r border-border bg-card py-2">
        <button
          type="button"
          aria-label="Expand history"
          onClick={onToggleCollapsed}
          className="flex size-8 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground"
        >
          <ChevronRight aria-hidden="true" className="size-4" />
        </button>
      </div>
    );
  }

  const now = Date.now();
  return (
    <div className="flex w-[264px] shrink-0 flex-col border-r border-border bg-card">
      <div className="flex shrink-0 items-center justify-between gap-2 px-3 pb-2 pt-3">
        <span className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">Chats</span>
        <button
          type="button"
          aria-label="Collapse history"
          onClick={onToggleCollapsed}
          className="flex size-7 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground"
        >
          <ChevronLeft aria-hidden="true" className="size-4" />
        </button>
      </div>
      <div className="shrink-0 px-2 pb-2">
        <Button variant="primary" size="sm" className="w-full justify-center gap-1.5" onClick={onNewChat}>
          <Plus aria-hidden="true" className="size-4" />
          New chat
        </Button>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto px-2">
        {sessions.length === 0 ? (
          <div className="flex flex-col items-center gap-2 px-3 py-10 text-center">
            <MessageSquarePlus aria-hidden="true" className="size-5 text-muted-foreground/50" />
            <p className="text-xs text-muted-foreground">No saved chats yet.</p>
          </div>
        ) : (
          <ul className="space-y-0.5 pb-2">
            {sessions.map((s) => (
              <li
                key={s.id}
                className="group flex items-center gap-1 rounded-lg px-2 py-1.5 text-sm hover:bg-accent"
              >
                <button
                  type="button"
                  className="min-w-0 flex-1 truncate text-left"
                  title={s.title}
                  onClick={() => onSelectSession(s.id)}
                >
                  <span className="block truncate font-medium">{s.title}</span>
                  <span className="block text-xs text-muted-foreground">{relativeTime(s.updatedAt, now)}</span>
                </button>
                <button
                  type="button"
                  aria-label={`Delete ${s.title}`}
                  onClick={() => onDeleteSession(s.id)}
                  className="flex size-6 shrink-0 items-center justify-center rounded-md text-muted-foreground opacity-0 hover:bg-background hover:text-destructive focus-visible:opacity-100 group-hover:opacity-100"
                >
                  <Trash2 aria-hidden="true" className="size-3.5" />
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
      <div className="shrink-0 border-t border-border p-2">
        <Button variant="ghost" size="sm" className="w-full justify-start gap-2" onClick={onOpenSkills}>
          <Wrench aria-hidden="true" className="size-4" />
          Skills
        </Button>
      </div>
    </div>
  );
}

/**
 * Full-tab, workspace-level host for the assistant — opened globally rather
 * than scoped to a specific resource. `cluster` is always `null` on this tab
 * (it's not scoped to whichever cluster happens to be active elsewhere in the
 * workspace), so it never attaches a resource `context`; `availableContexts`
 * (all configured kube contexts) drives the multi-cluster select instead —
 * see `AssistantConversation`.
 *
 * Two-pane, Cursor-inspired layout (Task 19): a collapsible left rail lists
 * saved sessions; the main pane hosts `AssistantConversation` with its own
 * session-history popover suppressed (`hideSessionControls`) so the list
 * isn't shown twice. `AssistantConversation` remains the sole owner of the
 * session list — this component only mirrors it (via `onSessionsChanged`) to
 * render the rail, and drives New Chat/select/delete back through the
 * imperative handle, so there is exactly one source of truth.
 */
export function AssistantTab({
  cluster,
  namespace,
  availableContexts,
}: {
  cluster: string | null;
  namespace?: string;
  availableContexts: string[];
}) {
  const convRef = useRef<AssistantConversationHandle>(null);
  const [sessions, setSessions] = useState<SessionMeta[]>([]);
  const [collapsed, setCollapsed] = useState(false);
  const [skillsOpen, setSkillsOpen] = useState(false);
  // Bumped when the Skills panel closes so the conversation reloads its slash
  // menu — the panel may have created, renamed, or deleted skills.
  const [skillsRefreshKey, setSkillsRefreshKey] = useState(0);

  return (
    <div className="flex h-full min-h-0">
      <HistoryRail
        collapsed={collapsed}
        onToggleCollapsed={() => setCollapsed((c) => !c)}
        sessions={sessions}
        onNewChat={() => convRef.current?.newChat()}
        onSelectSession={(id) => convRef.current?.selectSession(id)}
        onDeleteSession={(id) => convRef.current?.deleteSession(id)}
        onOpenSkills={() => setSkillsOpen(true)}
      />
      <div className="min-w-0 flex-1">
        <AssistantConversation
          ref={convRef}
          className="h-full"
          context={cluster ? { context: cluster, namespace } : undefined}
          availableContexts={availableContexts}
          hideSessionControls
          onSessionsChanged={setSessions}
          skillsRefreshKey={skillsRefreshKey}
        />
      </div>
      {skillsOpen && (
        <SkillsPanel
          onClose={() => {
            setSkillsOpen(false);
            setSkillsRefreshKey((k) => k + 1);
          }}
        />
      )}
    </div>
  );
}
