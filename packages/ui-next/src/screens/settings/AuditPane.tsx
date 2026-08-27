import { useEffect, useState } from "react";
import { auditTail, describeError, type AuditEntry } from "@srelens/core";
import { LoadingState, Panel, Section, Table, toneColor, type Column, type Tone } from "@srelens/ui-kit";
import { FailureState } from "../../lib/errorCopy";

/**
 * §23's `Audit` pane: every capability call an MCP-connected agent has made,
 * whether it was allowed or not — the pane someone opens after an incident.
 *
 * **This is a window, not the whole trail.** `auditTail` (`packages/core/src/
 * lib/mcpSecurity.ts`) takes a `limit` and returns the newest that many
 * entries; the on-disk log the backend reads it from is capped at 5 MB and
 * rotates past that, so "the most recent {@link LIMIT}" is a real ceiling, not
 * a decoration. A table that showed those rows with no word about how many
 * there might have been before them would let a reader conclude the trail
 * they are looking at is the whole story. It says the number instead.
 *
 * **No `Export`** (#371). There is nothing here that serialises the trail to
 * a file, and on web there is no filesystem to write one to even if there
 * were — so rather than draw a button that cannot work, this says so in one
 * sentence.
 *
 * **An empty trail is not a failure.** A fresh install has made no capability
 * calls yet; `Table`'s own empty state (no `role="alert"`) says that plainly.
 * Only a load that actually failed — `auditTail` itself rejecting — reaches
 * {@link FailureState}, which is the one place on this pane an `alert` is
 * drawn.
 *
 * **Every verdict word and colour below comes from the entry**, via
 * {@link verdictOf} — see its comment for why that one small mapping exists
 * at all, given how many of its kind this redesign has already removed.
 */

/** How many of the newest entries this pane asks for and says it is showing. */
export const LIMIT = 50;

type Verdict = { word: string; tone: Tone };

/**
 * `AuditEntry` carries `decision` ("approved" | "denied" | "auto") and
 * `outcome` ("ok" | "error") — real fields, not invented ones — but no single
 * verdict word or colour of its own for a table cell to read off directly.
 * This project deliberately removed ten hand-paired label/tone tables during
 * the redesign and kept three as marked survivors; this is a fourth, kept
 * this small (four cases, straight from those two fields) on purpose. If a
 * second surface ever needs the same words, this belongs on `AuditEntry`
 * itself rather than staying duplicated here.
 */
function verdictOf(entry: AuditEntry): Verdict {
  if (entry.decision === "denied") return { word: "denied", tone: "sev" };
  if (entry.outcome === "error") return { word: "failed", tone: "warn" };
  return entry.decision === "approved" ? { word: "approved", tone: "ok" } : { word: "allowed", tone: "muted" };
}

/**
 * The reason beside a verdict, when the entry carries one. `err` is
 * sometimes a curated policy sentence (`crates/mcp/src/policy.rs`) and
 * sometimes a raw `CapabilityError`'s `Display` text (`handler error: …`,
 * `crates/mcp/src/stdio.rs`) — the exact shape `describeError` exists to
 * clean up (`packages/core/src/lib/errors.ts`). Routed through it either way,
 * so this pane never has to know which kind a given row's `err` is.
 */
function reasonOf(entry: AuditEntry): string | null {
  return entry.err ? describeError(entry.err).detail : null;
}

/**
 * A capability's `args` carry whatever names its target, and this pane has
 * no per-capability schema to read them against — only the conventional keys
 * seen across the calls core already wraps (`context`, `namespace`, `name`;
 * `packages/core/src/lib/helm.ts`'s `getHelmRelease({ context, namespace,
 * name })` is one), plus `node` for the node-scoped capabilities (`node.
 * cordon`, `node.drain`) that have no namespace at all. Falls back to the raw
 * args rather than going blank, for a call this doesn't recognise the shape
 * of.
 */
function targetOf(entry: AuditEntry): string {
  const args = entry.args ?? {};
  const segments = ["context", "namespace", "node", "name"]
    .map((key) => args[key])
    .filter((value): value is string => typeof value === "string" && value.length > 0);
  if (segments.length > 0) return segments.join("/");
  const rest = JSON.stringify(args);
  return rest && rest !== "{}" ? rest : "—";
}

const columns: Column<AuditEntry>[] = [
  {
    key: "ts",
    header: "Time",
    render: (entry) => {
      const d = new Date(entry.ts * 1000);
      const pad = (n: number) => String(n).padStart(2, "0");
      return (
        <span className="tabular-nums text-muted">
          {pad(d.getHours())}:{pad(d.getMinutes())}:{pad(d.getSeconds())}
        </span>
      );
    },
    getValue: (entry) => entry.ts,
  },
  {
    key: "transport",
    header: "Client",
    // `AuditEntry` names the transport a call arrived on, not who used it —
    // the MCP server pane draws the same line for the same reason (#369):
    // srelens does not track which client connected.
    render: (entry) => <span>{entry.transport}</span>,
  },
  {
    key: "tool",
    header: "Capability",
    render: (entry) => (
      <code className="code" style={{ color: toneColor("accent") }}>
        {entry.tool}
      </code>
    ),
  },
  {
    key: "target",
    header: "Target",
    render: (entry) => {
      const target = targetOf(entry);
      return (
        // `min-width: auto` has cost this migration eight defects, and a
        // namespace-qualified object name is exactly the unbounded string
        // that triggers it: capped and truncated, with the full value in a
        // `title` since the visible text is the only copy on screen.
        <span data-testid="audit-target" className="path block max-w-[220px] truncate font-mono" title={target}>
          {target}
        </span>
      );
    },
    getValue: targetOf,
  },
  {
    key: "verdict",
    header: "Verdict",
    render: (entry) => {
      const { word, tone } = verdictOf(entry);
      const reason = reasonOf(entry);
      return (
        <span style={{ color: toneColor(tone) }}>
          {word}
          {reason ? ` · ${reason}` : null}
        </span>
      );
    },
  },
];

export function AuditPane() {
  const [entries, setEntries] = useState<AuditEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<unknown>(null);

  useEffect(() => {
    let cancelled = false;
    auditTail(LIMIT)
      .then((rows) => {
        if (!cancelled) setEntries(rows);
      })
      .catch((e) => {
        if (!cancelled) setError(e);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <Panel title="Audit · every capability call, allowed or not">
      <p className="text-[0.75rem] leading-relaxed text-muted">
        Showing the most recent {LIMIT} capability calls. Older calls exist only in the log file itself, not here.
      </p>
      {loading ? (
        <LoadingState label="Reading the audit trail" />
      ) : error !== null ? (
        <FailureState title="The audit trail could not be read" error={error} />
      ) : (
        <Section padded={false} className="mt-2">
          <Table
            columns={columns}
            data={entries}
            getRowKey={(entry) => String(entries.indexOf(entry))}
            emptyText="No capability calls yet"
            emptyHint="A fresh install has made none — this is not an error."
          />
        </Section>
      )}
      {/* #371: no serialisation exists for this trail, and the web build has
          no filesystem to write one to even if it did. */}
      <p className="mt-3 text-[0.75rem] leading-relaxed text-muted">
        There is no way to export this trail — no serialisation exists for it, and srelens running on web has no
        filesystem to save one to.
      </p>
    </Panel>
  );
}
