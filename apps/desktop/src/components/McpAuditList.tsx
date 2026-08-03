import { auditTail, type AuditEntry } from "../lib/mcpSecurity";
import { Badge, Spinner, Table, type BadgeVariant, type Column } from "../ui";
import { useEffect, useState } from "react";

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

  useEffect(() => {
    let active = true;
    void auditTail(50).then((out) => {
      if (active) setEntries(out);
    });
    return () => {
      active = false;
    };
  }, []);

  if (entries === null) return <Spinner label="Loading agent activity" />;

  const rows: Row[] = entries.map((entry, id) => ({ ...entry, id }));

  return (
    <Table
      columns={columns}
      data={rows}
      getRowKey={(row) => String(row.id)}
      emptyText="No agent activity yet."
    />
  );
}
