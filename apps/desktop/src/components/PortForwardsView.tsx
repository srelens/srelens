import React, { useCallback, useEffect, useState } from "react";
import { CircleStop, Copy, Play, Trash2 } from "lucide-react";
import { Table, Badge, Button, ColumnPicker, StatusPill, useColumnVisibility, type Column } from "../ui";
import { useForwards, forwardStatusKind, forwardStatusLabel } from "./ForwardsIndicator";
import {
  startPortForward,
  stopPortForward,
  forwardUrl,
  forwardAddress,
  type ActiveForward,
} from "../lib/forward";
import { listSavedForwards, deleteSavedForward, type SavedForward } from "../lib/savedForwards";

/**
 * Network overview of every active port-forward across all connected clusters.
 * Backed by the in-memory forwards store, so it updates live as forwards start,
 * stop, or drop on their own.
 */
export function PortForwardsView({ context }: { context?: string }) {
  const all = useForwards();
  // Show this cluster's forwards first, but list every cluster for a true
  // overview (the store is global).
  const forwards = context
    ? [...all].sort((a, b) => Number(b.context === context) - Number(a.context === context))
    : all;

  const columns: Column<ActiveForward>[] = [
    {
      key: "name",
      header: "Name",
      render: (f) => <span className="fl-mono">{f.name}</span>,
    },
    {
      key: "kind",
      header: "Kind",
      render: (f) => <Badge variant="info">{f.kind}</Badge>,
    },
    { key: "namespace", header: "Namespace", render: (f) => f.namespace || "—" },
    { key: "context", header: "Cluster", render: (f) => f.context },
    {
      key: "local",
      header: "Local",
      render: (f) => (
        <a
          className="fl-mono text-primary hover:underline"
          href={forwardUrl(f)}
          target="_blank"
          rel="noreferrer"
        >
          {forwardAddress(f)}
        </a>
      ),
    },
    { key: "remote", header: "Remote", render: (f) => <span className="fl-mono">{f.remotePort}</span> },
    {
      key: "status",
      header: "Status",
      render: (f) => <StatusPill status={forwardStatusLabel(f.status)} kind={forwardStatusKind(f.status)} />,
    },
    {
      key: "actions",
      header: "",
      render: (f) => (
        <div className="flex justify-end gap-2">
          <Button
            variant="ghost"
            onClick={() => void navigator.clipboard?.writeText(forwardAddress(f))}
          >
            <Copy data-icon="inline-start" />
            Copy
          </Button>
          <Button variant="danger" onClick={() => void stopPortForward(f.id)}>
            <CircleStop data-icon="inline-start" />
            Stop
          </Button>
        </div>
      ),
    },
  ];

  const { visibleColumns, columnOptions, hidden, toggle, pinnedKey } = useColumnVisibility(
    "portforwards",
    columns,
  );

  const [saved, setSaved] = useState<SavedForward[]>([]);
  const [startingId, setStartingId] = useState<string | null>(null);
  const [savedError, setSavedError] = useState("");

  const refreshSaved = useCallback(() => {
    if (!context) {
      setSaved([]);
      return;
    }
    void listSavedForwards(context).then(setSaved);
  }, [context]);

  useEffect(() => {
    refreshSaved();
  }, [refreshSaved]);

  async function handleStartSaved(sf: SavedForward) {
    if (!context) return;
    setStartingId(sf.id);
    setSavedError("");
    try {
      await startPortForward({
        context,
        namespace: sf.namespace,
        kind: sf.kind,
        name: sf.target,
        remotePort: sf.remotePort,
        localPort: sf.localPort,
      });
    } catch (e) {
      // Surface the failure (e.g. the pinned local port is now taken, or the
      // target is unreachable) instead of failing silently.
      setSavedError(`Couldn't start ${sf.name}: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setStartingId(null);
    }
  }

  async function handleDeleteSaved(sf: SavedForward) {
    if (!context) return;
    await deleteSavedForward(context, sf.id);
    refreshSaved();
  }

  const savedColumns: Column<SavedForward>[] = [
    { key: "name", header: "Name", render: (sf) => <span className="fl-mono">{sf.name}</span> },
    { key: "kind", header: "Kind", render: (sf) => <Badge variant="info">{sf.kind}</Badge> },
    { key: "namespace", header: "Namespace", render: (sf) => sf.namespace || "—" },
    { key: "target", header: "Target", render: (sf) => <span className="fl-mono">{sf.target}</span> },
    { key: "remote", header: "Remote", render: (sf) => <span className="fl-mono">{sf.remotePort}</span> },
    {
      key: "actions",
      header: "",
      render: (sf) => (
        <div className="flex justify-end gap-2">
          <Button
            variant="ghost"
            disabled={startingId === sf.id}
            onClick={() => void handleStartSaved(sf)}
          >
            <Play data-icon="inline-start" />
            Start
          </Button>
          <Button variant="danger" onClick={() => void handleDeleteSaved(sf)}>
            <Trash2 data-icon="inline-start" />
            Delete
          </Button>
        </div>
      ),
    },
  ];

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex shrink-0 items-center gap-2 border-b border-border px-3 py-2 text-sm">
        <span className="font-medium">Port Forwards</span>
        <Badge variant="info">{forwards.length}</Badge>
        <div className="ml-auto">
          <ColumnPicker columns={columnOptions} hidden={hidden} onToggle={toggle} pinnedKey={pinnedKey} />
        </div>
      </div>
      <div className="min-h-0 flex-1 overflow-auto">
        {forwards.length === 0 ? (
          <div className="p-8 text-center text-sm text-muted-foreground">
            No active port forwards. Open a Pod or Service and use the <strong>Forward</strong> action
            to start one.
          </div>
        ) : (
          <Table columns={visibleColumns} data={forwards} getRowKey={(f) => String(f.id)} />
        )}
      </div>
      {context && (
        <div className="flex max-h-64 shrink-0 flex-col border-t border-border">
          <div className="flex shrink-0 items-center gap-2 px-3 py-2 text-sm">
            <span className="font-medium">Saved</span>
            <Badge variant="neutral">{saved.length}</Badge>
          </div>
          {savedError && (
            <div role="alert" className="shrink-0 px-3 pb-2 text-xs text-destructive">
              {savedError}
            </div>
          )}
          <div className="min-h-0 flex-1 overflow-auto">
            {saved.length === 0 ? (
              <div className="p-4 text-center text-sm text-muted-foreground">
                No saved forwards for this cluster. Save one from the Forward dialog to reuse it later.
              </div>
            ) : (
              <Table columns={savedColumns} data={saved} getRowKey={(sf) => sf.id} />
            )}
          </div>
        </div>
      )}
    </div>
  );
}
