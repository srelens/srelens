import { auditTail, describeError, type AuditEntry } from "@srelens/core";
import { Badge, Button, ErrorState, Spinner, Table, type BadgeVariant, type Column } from "../ui";
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

interface McpAuditListProps {
  /**
   * Called after the Refresh button bumps this list's own nonce, so a parent
   * can piggyback another stale-data panel (`McpPromptIssues`) on the same
   * affordance instead of growing a second Refresh button next to it.
   */
  onRefresh?: () => void;
}

/**
 * Recent MCP tool calls — what an agent actually did, and whether it was
 * allowed.
 *
 * **Loading, unreadable and empty are three states here.** `auditTail` used to
 * swallow every refusal and resolve to `[]`, so this panel could only ever say
 * "No agent activity yet." — the same words for a quiet cluster and for a
 * trail srelens could not read at all. It rejects now, and the refusal gets a
 * surface of its own: on the panel whose subject is what an agent did, "we do
 * not know" must not be printed as "nothing happened".
 */
export function McpAuditList({ onRefresh }: McpAuditListProps = {}) {
  const [entries, setEntries] = useState<AuditEntry[] | null>(null);
  const [error, setError] = useState<unknown>(null);
  // Bumped to re-run the fetch. Settings can sit open for a long while as
  // agents keep calling, and a list read once on mount quietly goes stale —
  // an operator looking for an agent's action would conclude it never happened.
  const [nonce, setNonce] = useState(0);

  useEffect(() => {
    let active = true;
    auditTail(50)
      .then((out) => {
        if (!active) return;
        setError(null);
        setEntries(out);
      })
      .catch((e: unknown) => {
        if (!active) return;
        // The rows are dropped as well as the error kept: a refusal that left
        // the previous read on screen would show a stale trail under no
        // warning that it is stale.
        setEntries(null);
        setError(e);
      });
    return () => {
      active = false;
    };
  }, [nonce]);

  const refresh = useCallback(() => {
    setNonce((n) => n + 1);
    onRefresh?.();
  }, [onRefresh]);

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
      {error !== null ? (
        <ErrorState
          title="The agent activity log could not be read"
          detail={describeError(error).detail}
          onRetry={refresh}
        />
      ) : entries === null ? (
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
