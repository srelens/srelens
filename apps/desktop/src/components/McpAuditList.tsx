import { auditTail, type AuditEntry } from "../lib/mcpSecurity";
import { Badge, Button, Spinner, Table, type BadgeVariant, type Column } from "../ui";
import { RefreshCw } from "lucide-react";
import { useCallback, useEffect, useState } from "react";

const DECISION_VARIANT: Record<AuditEntry["decision"], BadgeVariant> = {
  approved: "success",
  denied: "danger",
  auto: "neutral",
};

const OUTCOME_VARIANT: Record<AuditEntry["outcome"], BadgeVariant> = {
  ok: "success",
  error: "danger",
};

// Table rows need a stable key that survives sorting; the fetch-order index
// works since the underlying entries never reorder themselves.
type Row = AuditEntry & { id: number };

const columns: Column<Row>[] = [
  { key: "ts", header: "Time", render: (e) => new Date(e.ts * 1000).toLocaleString() },
  { key: "tool", header: "Tool", render: (e) => <code>{e.tool}</code> },
  { key: "transport", header: "Transport" },
  {
    key: "decision",
    header: "Decision",
    render: (e) => <Badge variant={DECISION_VARIANT[e.decision]}>{e.decision}</Badge>,
  },
  {
    key: "outcome",
    header: "Outcome",
    render: (e) => <Badge variant={OUTCOME_VARIANT[e.outcome]}>{e.outcome}</Badge>,
  },
];

/** Recent MCP tool calls — what an agent actually did, and whether it was allowed. */
export function McpAuditList() {
  const [entries, setEntries] = useState<AuditEntry[] | null>(null);
  // Bumped to re-run the fetch. Settings can sit open for a long while as
  // agents keep calling, and a list read once on mount quietly goes stale —
  // an operator looking for an agent's action would conclude it never happened.
  const [nonce, setNonce] = useState(0);

  useEffect(() => {
    let active = true;
    void auditTail(50).then((out) => {
      if (active) setEntries(out);
    });
    return () => {
      active = false;
    };
  }, [nonce]);

  const refresh = useCallback(() => setNonce((n) => n + 1), []);

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center">
        <Button
          variant="ghost"
          size="sm"
          className="ml-auto"
          onClick={refresh}
          aria-label="Refresh agent activity"
        >
          <RefreshCw data-icon="inline-start" />
          Refresh
        </Button>
      </div>
      {entries === null ? (
        <Spinner label="Loading agent activity" />
      ) : (
        <Table
          columns={columns}
          data={entries.map((entry, id) => ({ ...entry, id }))}
          getRowKey={(row) => String(row.id)}
          emptyText="No agent activity yet."
        />
      )}
    </div>
  );
}
