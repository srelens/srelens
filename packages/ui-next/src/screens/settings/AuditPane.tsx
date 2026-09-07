import { useEffect, useState } from "react";
import { auditTail, describeError, type AuditEntry } from "@srelens/core";
import { Button, LoadingState, Panel, Section, Table, toneColor, type Column, type Tone } from "@srelens/ui-kit";
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
 * **And until this wave, `auditTail` could not reject.** It caught every
 * refusal and resolved to `[]`, so the branch above was unreachable and an
 * unreadable trail rendered "A fresh install has made none — this is not an
 * error." as fact, with no alert — guaranteed on the web build, where every
 * `invoke` rejects. The wrapper propagates now (`packages/core/src/lib/
 * mcpSecurity.ts`), which is what makes the three states here real rather than
 * decorative.
 *
 * **The fix went one layer deeper afterwards**, because the wrapper alone only
 * distinguished an IPC failure. `srelens_mcp::audit::tail` swallowed three I/O
 * failures of its own — open, seek, read — into the same empty vector, so a log
 * this pane could not read still arrived as a successful empty trail and still
 * rendered the fresh-install sentence. The backend returns `io::Result` now
 * (`crates/mcp/src/audit.rs`), empty only for a log that does not exist, and
 * `mcp_audit_tail` refuses with the file named — so the failure branch below
 * really does stand for an unreadable trail and not only for a broken bridge.
 *
 * **It re-reads on demand**, because classic's `McpAuditList` wrote the reason
 * down and this pane lost it: "a list read once on mount quietly goes stale —
 * an operator looking for an agent's action would conclude it never happened."
 * Settings sits open while agents keep calling. There is no poll — a trail
 * that refreshed itself under a reader scrolling it is worse than one they ask
 * for — so the rows on screen are always the answer to a read the reader
 * asked for.
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
    // Dated, not just clocked. The on-disk log is capped at 5 MB and rotates
    // past that, so the window this pane shows routinely spans days —
    // `14:02:11` alone cannot answer which day a call landed on, which is the
    // first question an operator reading this after an incident has. Local
    // time and the reader's own zone, like every other timestamp in this
    // package: the entry carries a Unix second and nothing about where it was
    // recorded.
    render: (entry) => {
      const d = new Date(entry.ts * 1000);
      const pad = (n: number) => String(n).padStart(2, "0");
      return (
        <span data-testid={`audit-time-${entry.ts}`} className="whitespace-nowrap tabular-nums text-muted">
          {d.getFullYear()}-{pad(d.getMonth() + 1)}-{pad(d.getDate())} {pad(d.getHours())}:
          {pad(d.getMinutes())}:{pad(d.getSeconds())}
        </span>
      );
    },
    getValue: (entry) => entry.ts,
  },
  {
    key: "transport",
    // Named for the value, which is `"stdio"` or `"http"`. §23's header is
    // `Client` over rows of product names, and #369 says plainly that srelens
    // does not track which client connected — so a `Client` header over a
    // transport claims exactly what that issue says srelens cannot know. The
    // MCP server pane declines to draw a clients list for the same reason.
    header: "Transport",
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
  /**
   * Bumped to re-run the read, the same shape classic's `McpAuditList` uses.
   * A nonce rather than a function the button calls directly, so the effect
   * stays the only place that touches these three pieces of state and its
   * `cancelled` flag keeps applying — a click during an in-flight read
   * supersedes it instead of racing it.
   */
  const [nonce, setNonce] = useState(0);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    auditTail(LIMIT)
      .then((rows) => {
        if (cancelled) return;
        // Cleared on success as well as set on failure: a re-read that worked
        // must not leave the previous refusal's alert standing over the rows
        // it just fetched.
        setError(null);
        setEntries(rows);
      })
      .catch((e) => {
        if (cancelled) return;
        // The rows go with it. A refusal that left the last good read on
        // screen would show a stale trail under an alert saying the trail
        // could not be read — two answers to one question.
        setEntries([]);
        setError(e);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [nonce]);

  return (
    <Panel title="Audit · every capability call, allowed or not">
      {/* flex-wrap rather than a fixed row: the sentence is the long half and
          grows in translation, and a flex child with nothing to stop it
          shrinking is where `min-width: auto` has cost this migration eight
          defects. */}
      <div className="flex flex-wrap items-start justify-between gap-2">
        <p className="min-w-0 flex-1 text-[0.75rem] leading-relaxed text-muted">
          Showing the most recent {LIMIT} capability calls. Older calls exist only in the log file itself, not here.
        </p>
        <Button
          variant="secondary"
          size="sm"
          disabled={loading}
          onClick={() => setNonce((n) => n + 1)}
        >
          {loading ? "Reading…" : "Refresh"}
        </Button>
      </div>
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
