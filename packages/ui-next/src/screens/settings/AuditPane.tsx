import { useEffect, useState } from "react";
import { auditTail, describeError, readPromptIssues, type PromptIssue, type AuditEntry } from "@srelens/core";
import { Button, LoadingState, Panel, Section, Table, toneColor, type Column, type Tone } from "@srelens/ui-kit";
import { FailureAlert, FailureState } from "../../lib/errorCopy";

/**
 * §23's `Audit` pane: every capability call an MCP-connected agent has made,
 * whether it was allowed or not, and every mutating or sensitive one made in
 * the app itself — the pane someone opens after an incident. The title says
 * both halves in those terms rather than claiming "every capability call",
 * which is true of the agent side and not of this one.
 *
 * **Both surfaces, since #555.** The trail held MCP calls alone, because the
 * audit sink lived in the MCP crate: a capability invoked from the app went
 * straight to the registry through `invoke_capability`
 * (`apps/desktop/src-tauri/src/bridge.rs`), so an Argo CD sync or a Flux
 * reconcile clicked on this very screen's sibling left nothing behind, while
 * the identical call from an agent was written down. The sink moved beside the
 * registry — the one place both paths meet
 * (`crates/capability/src/audit.rs`) — and `Source` below is how a reader
 * tells "I did this" from "something else did".
 *
 * **The asymmetry in what is recorded is deliberate and it is visible here.**
 * MCP records every call, reads included. The app records only what it
 * changed, plus reads that return secret material: a resource screen fires
 * dozens of list calls a minute and burying the writes under them would cost
 * this pane its purpose. The sentence under the table says so, because a
 * reader who assumes otherwise would read the absence of their own reads as
 * evidence of something.
 *
 * **It never leaves the machine.** One `0600` JSONL file under the app's
 * config directory, read from here and nowhere else. Nothing uploads it.
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
  // `rejected` and `failed` are two different answers to "did it happen?" —
  // srelens would not do it, or the cluster would not — and collapsing them
  // into one word is the mistake this project has a rule about. The backend
  // tells them apart (`crates/capability/src/lib.rs`), so this does too, and
  // in colour as well as in the word: amber for a call that never ran and
  // broke nothing, red for one that ran and did not finish. `denied` is red
  // beside it on purpose — both are rows where what you asked for did not
  // happen and something is worth looking at; `rejected` is the one that is
  // usually just a malformed call.
  if (entry.outcome === "rejected") return { word: "rejected", tone: "warn" };
  if (entry.outcome === "failed") return { word: "failed", tone: "sev" };
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
 * What the call named, cluster first.
 *
 * The backend extracts `cluster` and `resource` when it writes the record
 * (`describe_target`, `crates/capability/src/audit.rs`), because it is the
 * half of this system that knows the argument shapes — including
 * `extensions.action`, whose whole target sits nested under `resource` where
 * the flat key sweep below would never have found it. So those two fields are
 * read first.
 *
 * The sweep stays as the fallback for a record that carries neither: the
 * conventional keys across the calls core already wraps (`context`,
 * `namespace`, `name`; `packages/core/src/lib/helm.ts`'s `getHelmRelease({
 * context, namespace, name })` is one), plus `node` for the node-scoped
 * capabilities (`node.cordon`, `node.drain`) that have no namespace at all.
 * Falls back to the raw args rather than going blank, for a call this doesn't
 * recognise the shape of.
 */
function targetOf(entry: AuditEntry): string {
  const named = [entry.cluster, entry.resource].filter(
    (value): value is string => typeof value === "string" && value.length > 0,
  );
  if (named.length > 0) return named.join("/");
  const args = entry.args ?? {};
  const segments = ["context", "namespace", "node", "name"]
    .map((key) => args[key])
    .filter((value): value is string => typeof value === "string" && value.length > 0);
  if (segments.length > 0) return segments.join("/");
  const rest = JSON.stringify(args);
  return rest && rest !== "{}" ? rest : "—";
}

/**
 * The app a call went through, when it went through one — `id@revision`,
 * because an update rolls the revision and leaves the ID alone, so the ID
 * alone cannot say which manifest and which grants were in force
 * (`crates/registry/src/extensions.rs`).
 */
function appOf(entry: AuditEntry): string | null {
  return entry.app ? `${entry.app.id}@${entry.app.revision}` : null;
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
    key: "source",
    // The first question this pane is opened with is "was that me or an
    // agent?", so that is the column: `ui` or `mcp`, with the transport
    // underneath it for the MCP rows because "which client do I go turn off"
    // is the second question. It is not headed `Client`: §23 drew product
    // names there and #369 says plainly that srelens does not track which
    // client connected, so a `Client` header would claim exactly what that
    // issue says srelens cannot know. The MCP server pane declines to draw a
    // clients list for the same reason.
    header: "Source",
    render: (entry) => (
      <span className="whitespace-nowrap" data-testid="audit-source">
        {entry.source}
        {entry.source === "mcp" ? <span className="text-muted"> · {entry.transport}</span> : null}
      </span>
    ),
    getValue: (entry) => entry.source,
  },
  {
    key: "tool",
    header: "Capability",
    render: (entry) => {
      const app = appOf(entry);
      return (
        <div className="min-w-0">
          <code className="code" style={{ color: toneColor("accent") }}>
            {entry.tool}
          </code>
          {/* The app is under the capability rather than in a column of its
              own: most rows have none, and an empty column across a whole
              screen reads as a fact about the trail rather than about the
              row. */}
          {app ? (
            <span
              data-testid="audit-app"
              className="block max-w-[220px] truncate text-[0.7rem] text-muted"
              title={app}
            >
              via {app}
            </span>
          ) : null}
        </div>
      );
    },
    getValue: (entry) => entry.tool,
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
  const [issues, setIssues] = useState<PromptIssue[]>([]);
  const [issuesError, setIssuesError] = useState<unknown>(null);
  const [issuesLoading, setIssuesLoading] = useState(true);
  useEffect(() => {
    let cancelled = false;
    setIssuesLoading(true);
    readPromptIssues().then(rows => {
      if (!cancelled) { setIssues(rows); setIssuesError(null); }
    }).catch(error => {
      if (!cancelled) { setIssues([]); setIssuesError(error); }
    }).finally(() => { if (!cancelled) setIssuesLoading(false); });
    return () => { cancelled = true; };
  }, [nonce]);

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
    // The title says what the trail actually holds, not a tidier version of
    // it. "every capability call" is true of the agent half and false of this
    // app's, where plain reads are deliberately left out — and a title that
    // overstates the coverage is exactly how a reader concludes an absent row
    // means an absent action.
    <Panel title="Audit · every call an agent made, and every change made here">
      {/* flex-wrap rather than a fixed row: the sentence is the long half and
          grows in translation, and a flex child with nothing to stop it
          shrinking is where `min-width: auto` has cost this migration eight
          defects. */}
      <div className="flex flex-wrap items-start justify-between gap-2">
        <p className="min-w-0 flex-1 text-[0.75rem] leading-relaxed text-muted">
          Showing the most recent {LIMIT} capability calls. Older calls exist only in the log file itself, not here.
          Every call an agent made over MCP is recorded; from srelens itself, only the calls that changed something
          or read secret material are — an app's own reads are not events. The log is a file on this machine and
          is never sent anywhere.
        </p>
        <Button
          variant="secondary"
          size="sm"
          disabled={(loading || issuesLoading) && error === null && issuesError === null}
          onClick={() => setNonce((n) => n + 1)}
        >
          {(loading || issuesLoading) && error === null && issuesError === null ? "Reading…" : "Refresh"}
        </Button>
      </div>
      {issuesError !== null && <FailureAlert tone="sev" title="Prompt file diagnostics could not be read" error={issuesError} />}
      {!issuesLoading && issues.length > 0 && <div className="mt-3 border-y border-rule py-2 text-[0.75rem]">
        <p className="font-medium">{issues.length} prompt {issues.length === 1 ? "file" : "files"} could not be loaded</p>
        <ul className="mt-1 space-y-1">{issues.map(issue => <li key={`${issue.file}:${issue.problem}`}>
          <code className="block overflow-x-auto whitespace-nowrap">{issue.file}</code><span className="text-muted">{issue.problem}</span>
        </li>)}</ul>
      </div>}
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
      {/* #371: still no serialisation and still no filesystem on web, so
          still no button. What #555 changed is the seam under it — a record
          now carries `source`, `app`, `cluster` and `resource` as fields of
          its own rather than leaving a reader to infer them from `args`, so
          an export has rows to write without this pane having to reconstruct
          them. */}
      <p className="mt-3 text-[0.75rem] leading-relaxed text-muted">
        There is no way to export this trail — no serialisation exists for it, and srelens running on web has no
        filesystem to save one to.
      </p>
    </Panel>
  );
}
