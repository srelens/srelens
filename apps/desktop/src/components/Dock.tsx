import React, { useEffect, useRef, useState } from "react";
import { Logs, Plus, SquareTerminal, X } from "lucide-react";
import { PodTerminal } from "./PodTerminal";
import { LocalTerminal } from "./LocalTerminal";
import { LogsView, type LogsSource } from "./LogsView";
import { HelmOpPane } from "./HelmOpPane";

export type DockKind = "terminal" | "logs" | "shell" | "helm";

export interface DockSession {
  id: number;
  kind: DockKind;
  context: string;
  namespace: string;
  /** Present for terminals and single-pod logs. */
  pod?: string;
  /** Preselect this container in single-pod logs (from a per-container action). */
  container?: string;
  /** Override the exec command for a terminal (e.g. node shell `nsenter …`). */
  execCommand?: string[];
  /** Present for workload (e.g. Deployment) logs that span many pods. */
  workload?: { kind: string; name: string };
  /** Extra kubeconfig files, for a local `shell` terminal scoped to the context. */
  kubeconfigFiles?: string[];
  /** Present for streamed helm operations. */
  helm?: { args: string[]; title: string; values?: string; onComplete?: () => void };
  /** A pod to delete when this session closes (node debug shell cleanup). */
  deleteOnClose?: { context: string; namespace: string; pod: string };
}

/** Tab/session label: the pod name, the workload kind/name, or the context for a shell. */
function sessionLabel(s: DockSession): string {
  if (s.kind === "shell") return `kubectl · ${s.context}`;
  if (s.kind === "helm") return s.helm?.title ?? "Helm";
  // Two shells into the same multi-container pod are different sessions, so
  // the container has to be in the label or the tabs are indistinguishable.
  if (s.pod && s.kind === "terminal" && s.container) return `${s.pod} · ${s.container}`;
  if (s.pod) return s.pod;
  if (s.workload) return `${s.workload.kind}/${s.workload.name}`;
  return "session";
}

/**
 * srelens bottom dock: a resizable panel with a tab per session —
 * in-pod shells (kube-rs exec) and pod/workload logs both live here.
 */
export function Dock({
  sessions,
  activeId,
  height,
  onActivate,
  onCloseTab,
  onClose,
  onResize,
  onNewTerminal,
}: {
  sessions: DockSession[];
  activeId: number | null;
  height: number;
  onActivate: (id: number) => void;
  onCloseTab: (id: number) => void;
  onClose: () => void;
  onResize: (height: number) => void;
  /** Open another terminal for the active context (shown as a "+" in the tab bar). */
  onNewTerminal?: () => void;
}) {
  const startY = useRef(0);
  const startH = useRef(0);
  const heightRef = useRef(height);
  heightRef.current = height;
  const handleRef = useRef<HTMLDivElement>(null);

  // In-memory tab renames (double-click a tab to rename; not persisted).
  const [titles, setTitles] = useState<Record<number, string>>({});
  const [renamingId, setRenamingId] = useState<number | null>(null);
  const [draft, setDraft] = useState("");
  const labelOf = (s: DockSession) => titles[s.id] ?? sessionLabel(s);
  const startRename = (s: DockSession) => {
    setDraft(labelOf(s));
    setRenamingId(s.id);
  };
  const commitRename = (id: number) => {
    const name = draft.trim();
    setTitles((current) => {
      const next = { ...current };
      if (name) next[id] = name;
      else delete next[id];
      return next;
    });
    setRenamingId(null);
  };

  useEffect(() => {
    const handle = handleRef.current;
    if (!handle) return;
    function onMove(e: MouseEvent) {
      const delta = startY.current - e.clientY;
      onResize(Math.max(120, Math.min(720, startH.current + delta)));
    }
    function onUp() {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
      document.body.style.userSelect = "";
    }
    function startDrag(e: MouseEvent) {
      e.preventDefault();
      startY.current = e.clientY;
      startH.current = heightRef.current;
      document.body.style.userSelect = "none";
      window.addEventListener("mousemove", onMove);
      window.addEventListener("mouseup", onUp);
    }
    handle.addEventListener("mousedown", startDrag);
    return () => {
      handle.removeEventListener("mousedown", startDrag);
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
      document.body.style.userSelect = "";
    };
  }, [onResize]);

  return (
    <div className="fl-dock" style={{ height }}>
      <div className="fl-dock__resize" ref={handleRef} role="separator" aria-orientation="horizontal" aria-label="Resize logs panel">
        <span className="fl-dock__grip" />
      </div>
      <div className="fl-dock__tabs">
        <div className="fl-dock__tablist" role="tablist">
          {sessions.map((s) => (
            <div
              key={s.id}
              role="tab"
              aria-selected={s.id === activeId}
              title={`${labelOf(s)} — double-click to rename`}
              className={`fl-dock__tab${s.id === activeId ? " fl-dock__tab--active" : ""}`}
              onClick={() => onActivate(s.id)}
            >
              <span className="fl-dock__tab-main">
                {s.kind === "terminal" || s.kind === "shell" ? (
                  <SquareTerminal aria-hidden="true" />
                ) : (
                  <Logs aria-hidden="true" />
                )}
                {renamingId === s.id ? (
                  <input
                    className="fl-dock__tab-rename"
                    value={draft}
                    autoFocus
                    onChange={(e) => setDraft(e.target.value)}
                    onClick={(e) => e.stopPropagation()}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") commitRename(s.id);
                      else if (e.key === "Escape") setRenamingId(null);
                    }}
                    onBlur={() => commitRename(s.id)}
                    aria-label={`Rename ${sessionLabel(s)}`}
                  />
                ) : (
                  <span
                    className="fl-dock__tab-label"
                    onDoubleClick={(e) => {
                      e.stopPropagation();
                      startRename(s);
                    }}
                  >
                    {labelOf(s)}
                  </span>
                )}
              </span>
              <button
                className="fl-dock__tab-close"
                aria-label={`Close ${labelOf(s)} ${s.kind}`}
                onClick={(e) => {
                  e.stopPropagation();
                  onCloseTab(s.id);
                }}
              >
                <X aria-hidden="true" />
              </button>
            </div>
          ))}
        </div>
        {onNewTerminal && (
          <button
            className="fl-dock__new"
            aria-label="New terminal"
            title="New terminal (same context)"
            onClick={onNewTerminal}
          >
            <Plus aria-hidden="true" />
          </button>
        )}
        <button className="fl-dock__close" aria-label="Close dock" onClick={onClose}>
          <X aria-hidden="true" />
        </button>
      </div>
      <div className="fl-dock__body">
        {/* Every session stays mounted; only the active one is shown. Unmounting
            an inactive tab would tear down its shell/log stream and lose the
            terminal scrollback, so we hide rather than remove. */}
        {sessions.map((s) => (
          <div
            key={s.id}
            className="fl-dock__pane"
            style={{ display: s.id === activeId ? "block" : "none" }}
          >
            {s.kind === "helm" ? (
              <HelmOpPane session={s} />
            ) : s.kind === "shell" ? (
              <LocalTerminal context={s.context} kubeconfigFiles={s.kubeconfigFiles ?? []} />
            ) : s.kind === "terminal" && s.pod ? (
              <PodTerminal
                context={s.context}
                namespace={s.namespace}
                pod={s.pod}
                container={s.container}
                command={s.execCommand}
              />
            ) : (
              <LogsView
                context={s.context}
                namespace={s.namespace}
                initialContainer={s.container}
                source={
                  (s.pod
                    ? { type: "pod", pod: s.pod }
                    : { type: "workload", kind: s.workload!.kind, name: s.workload!.name }) as LogsSource
                }
              />
            )}
          </div>
        ))}
      </div>
    </div>
  );
}
