import React from "react";
import { CircleStop, Copy } from "lucide-react";
import { Table, Badge, Button, ColumnPicker, useColumnVisibility, type Column } from "../ui";
import { useForwards } from "./ForwardsIndicator";
import { stopPortForward, forwardUrl, forwardAddress, type ActiveForward } from "../lib/forward";

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
    </div>
  );
}
