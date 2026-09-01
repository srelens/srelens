import { parseAssistantMarkdown, type MdBlock, type NoteSpan } from "@srelens/core";
import { AgentMark, Badge, cx, toneWash, type Tone } from "@srelens/ui-kit";
import type { GateRecord, ToolCallRecord, Turn } from "../../lib/agentRun";
import { CAPABILITY_CATALOG } from "@srelens/core";
import { pad2 } from "../../lib/numbers";

/**
 * The one renderer for a run's turns — mounted by both the console dock
 * (`compact`) and the full `/agent` screen, per the spec's "one store, one
 * submit path, one gate, and two renderers over them." Both hosts read the
 * SAME `AgentRun`, so this component owns none of it: it takes `turns` and
 * `gates` as plain props and draws what it is handed.
 *
 * **Gates are not a `Turn` field (Ruling A/D).** A confirmation arrives on
 * `mcp://confirm-request`, an app-wide channel `AgentConsent` alone answers —
 * not from the chat stream this transcript otherwise reflects — so there is
 * no turn to hang it off without guessing which one asked for it. `gates`
 * is `AgentRun.gates` verbatim, rendered as its own trailing list rather than
 * threaded into any one turn.
 *
 * **Decision 1 — the gate is a record, never a second prompt.** Classic ran
 * `McpConfirmDialog` (a modal) and `AssistantConversation`'s inline
 * `ConfirmCard` off the SAME request, each with its own Approve/Deny
 * buttons — two doors on one gate. `AgentConsent` is the only subscriber and
 * the only caller of `respondToConfirm` in the new design; this component
 * only ever reads `outcome` and shows a `Badge` for it. No button here
 * answers anything, and none should ever be added.
 */

const GATE_TONE: Record<GateRecord["outcome"], Tone> = {
  pending: "muted",
  approved: "ok",
  denied: "sev",
  // Not `sev`: nothing went wrong, and not `ok` either — nobody here decided.
  settled: "muted",
};

const GATE_WORD: Record<GateRecord["outcome"], string> = {
  pending: "Pending",
  approved: "Applied",
  denied: "Denied",
  // Deliberately does not say HOW. `mcp://confirm-resolved` carries an id and
  // nothing else, so "Timed out" or "Denied elsewhere" would both be srelens
  // guessing at a fact it was never told.
  settled: "No longer waiting",
};

/** A one-line summary of a call/gate's args, or `""` for `null`/`{}` — the
 * same "nothing worth a line" rule `apps/desktop`'s `summarizeArgs` uses, so
 * an empty tool call doesn't draw a bare `{}` beside its name. */
function summarizeArgs(args: unknown): string {
  if (args == null) return "";
  if (typeof args === "object" && Object.keys(args as Record<string, unknown>).length === 0) return "";
  try {
    return JSON.stringify(args);
  } catch {
    return "";
  }
}

/** One tool call — capability in accent (recolored to `sev`/`warn` once it
 * has actually failed or been refused), args faint, and the round trip
 * srelens itself measured, right-aligned.
 *
 * **`ms` is rendered only when it exists.** `ToolCallRecord.ms` is absent
 * until `toolResult` lands (see its doc in `lib/agentRun.ts`) — a call still
 * in flight has measured nothing yet, and printing `0ms` for it would tell
 * the reader a round trip that hasn't happened already finished instantly.
 */
function ToolCallRow({ call }: { call: ToolCallRecord }) {
  const args = summarizeArgs(call.args);
  const capability = call.status === "error" ? "text-sev" : call.status === "denied" ? "text-warn" : "text-accent";
  return (
    <div className="tool-call flex min-w-0 items-center gap-2">
      <span className={cx("shrink-0", capability)}>{call.tool}</span>
      {args !== "" && <span className="min-w-0 flex-1 truncate text-faint">{args}</span>}
      {call.ms !== undefined && <span className="ml-auto shrink-0 text-faint">{Math.round(call.ms)}ms</span>}
    </div>
  );
}

/** `Applied 14:06`, or bare `Applied` while nothing has stamped a time yet
 * (Ruling I: `at` is only ever set by `AgentConsent`, once, at resolution —
 * never fabricated here and never taken from render time, which would report
 * when the badge drew rather than when the gate itself resolved). */
function gateLabel(gate: GateRecord): string {
  const word = GATE_WORD[gate.outcome];
  if (gate.at === undefined) return word;
  const d = new Date(gate.at);
  return `${word} ${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
}

/**
 * A gate's arguments as pretty JSON, the way §A.1 draws them.
 *
 * `summarizeArgs` gives the one-liner a tool-call ROW needs. A gate is the one
 * place a reader is being asked to approve something, so the arguments are the
 * thing they are actually judging — `{"replicas":40}` truncated into a row is
 * not enough to say yes to.
 */
function prettyArgs(args: unknown): string {
  if (args === null || args === undefined) return "";
  if (typeof args === "object" && Object.keys(args as object).length === 0) return "";
  try {
    return JSON.stringify(args, null, 2);
  } catch {
    // A cyclic or unserialisable payload: better the summary than nothing.
    return summarizeArgs(args);
  }
}

/**
 * One gate, as decision 1's record — never a second set of answer buttons.
 *
 * **What this card takes from §A.1's mock, and what it does not.** The mock
 * draws a bordered card, the capability in mono, a `DESTRUCTIVE` badge, the
 * arguments as pretty JSON, and three buttons. The card, the mono capability,
 * the badge and the JSON are all here. The buttons are not, and neither is the
 * effect paragraph:
 *
 * - **The buttons.** `AgentConsent` is the only subscriber to
 *   `mcp://confirm-request` and the only caller of `respondToConfirm`, on
 *   purpose. Classic listened twice and showed a modal AND an inline card,
 *   each with its own buttons, so answering one left the other stale — which
 *   is the whole reason `mcp://confirm-resolved` had to be invented. A second
 *   set here would rebuild exactly that.
 * - **The effect paragraph** ("Restores DB_POOL_MAX=40 and recreates 12
 *   pods…"). `ConfirmRequest` is `{ id, tool, args }` (#388). srelens does not
 *   know what a call will do, and a sentence saying it would be invented.
 *
 * The badge IS honest: `destructive` is a real field on the capability
 * registry, so this reads it rather than guessing from the tool's name.
 */
function GateRow({ gate }: { gate: GateRecord }) {
  const facts = CAPABILITY_CATALOG.find((c) => c.id === gate.tool);
  const args = prettyArgs(gate.args);
  return (
    <div className="min-w-0 rounded-card border border-rule">
      <div
        className="flex min-w-0 items-start justify-between gap-3 rounded-t-card px-3 py-2"
        style={{ background: toneWash(GATE_TONE[gate.outcome]) }}
      >
        <div className="min-w-0">
          <p className="min-w-0 break-words font-mono text-[0.8125rem] font-medium text-ink">{gate.tool}</p>
          {/* The TIME only. `gateLabel` is word-plus-time, and the badge
              already carries the word — printing both put "Applied" on the
              card twice. */}
          {gate.at !== undefined && <TurnClock at={gate.at} className="mt-0.5 block" />}
        </div>
        <div className="flex shrink-0 items-center gap-1.5">
          {facts?.destructive === true && <Badge tone="sev">destructive</Badge>}
          <Badge tone={GATE_TONE[gate.outcome]}>{GATE_WORD[gate.outcome]}</Badge>
        </div>
      </div>
      {args !== "" && (
        <pre className="scroll m-0 max-h-40 overflow-auto whitespace-pre-wrap break-words rounded-b-card bg-canvas-deep px-3 py-2 font-mono text-[0.6875rem] leading-relaxed text-faint">
          {args}
        </pre>
      )}
    </div>
  );
}

/** One line's inline runs — bare elements, never a `<span>` wrapper around
 * plain text, so a reply reads as prose rather than a lattice of nodes. */
function Spans({ spans }: { spans: NoteSpan[] }) {
  return (
    <>
      {spans.map((span, i) => {
        if (span.kind === "code") {
          return (
            <code key={i} className="break-words rounded-tile bg-sunk px-1 py-0.5 font-mono text-[0.8em]">
              {span.text}
            </code>
          );
        }
        if (span.kind === "strong") return <strong key={i}>{span.text}</strong>;
        return <span key={i}>{span.text}</span>;
      })}
    </>
  );
}

/**
 * One `parseAssistantMarkdown` block, through React elements only — never
 * `dangerouslySetInnerHTML`, matching the rule classic's own `AssistantMarkdown`
 * already insists on. Every kind the parser can produce is handled: a `table`
 * or `code` block silently dropped here is an answer the reader cannot read.
 */
function Block({ block }: { block: MdBlock }) {
  if (block.kind === "heading") {
    // Three tiers by level, matching classic's own `AssistantMarkdown.tsx`
    // (:62-70) — a heading inside a chat bubble stays a `<p>`, not document
    // structure, but a level-1 and a level-4 heading must not read alike.
    // Classic's `text-muted-foreground` is shadcn's vocabulary; `text-muted`
    // is this design's own token for the same idea.
    const headingClass =
      block.level <= 1
        ? "text-base font-semibold"
        : block.level === 2
          ? "text-[0.95rem] font-semibold"
          : "text-sm font-semibold text-muted";
    return (
      <p className={cx("break-words", headingClass)}>
        <Spans spans={block.spans} />
      </p>
    );
  }
  if (block.kind === "bullet") {
    return (
      <ul className="flex list-disc flex-col gap-1 pl-5">
        {block.items.map((item, j) => (
          <li key={j} className="break-words">
            <Spans spans={item} />
          </li>
        ))}
      </ul>
    );
  }
  if (block.kind === "ordered") {
    return (
      <ol className="flex list-decimal flex-col gap-1 pl-5">
        {block.items.map((item, j) => (
          <li key={j} className="break-words">
            <Spans spans={item} />
          </li>
        ))}
      </ol>
    );
  }
  if (block.kind === "code") {
    return (
      <pre className="min-w-0 overflow-x-auto rounded-tile border border-rule bg-sunk p-2 font-mono text-xs">
        <code>{block.text}</code>
      </pre>
    );
  }
  if (block.kind === "table") {
    return (
      <div className="min-w-0 overflow-x-auto">
        <table className="w-full border-collapse text-xs">
          <thead>
            <tr>
              {block.headers.map((h, j) => (
                <th
                  key={j}
                  className="break-words border-b border-rule px-2 py-1 text-left font-medium text-faint"
                >
                  <Spans spans={h} />
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {block.rows.map((row, r) => (
              <tr key={r}>
                {row.map((cell, c) => (
                  <td key={c} className="break-words border-b border-rule px-2 py-1 align-top">
                    <Spans spans={cell} />
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    );
  }
  // Only "paragraph" is left.
  return (
    <p className="break-words">
      <Spans spans={block.spans} />
    </p>
  );
}

/** An agent's answer text, parsed and drawn block by block. `null` for an
 * empty string (an in-flight turn with no text yet) rather than an empty
 * wrapper `div` nobody needs. */
function Answer({ text }: { text: string }) {
  const blocks = parseAssistantMarkdown(text);
  if (blocks.length === 0) return null;
  return (
    <div className="flex min-w-0 flex-col gap-2 text-sm leading-relaxed">
      {blocks.map((block, i) => (
        <Block key={i} block={block} />
      ))}
    </div>
  );
}

/** The reader's own turn — right-aligned in accent wash, images (if any)
 * shown above the text they were attached to. */
/**
 * A turn's own clock — `14:04:12`, as the mock draws under every turn.
 *
 * `Turn.at` was recorded from the start and nothing showed it, so a transcript
 * gave no way to line an answer up against anything else that happened. To the
 * SECOND, unlike the run head's `started 14:04`: turns in one conversation are
 * often a few seconds apart, and minutes alone would print the same stamp
 * three times in a row.
 */
function TurnClock({ at, className }: { at: number; className?: string }) {
  const d = new Date(at);
  return (
    <span className={cx("text-[0.6875rem] tabular-nums text-faint", className)}>
      {`${pad2(d.getHours())}:${pad2(d.getMinutes())}:${pad2(d.getSeconds())}`}
    </span>
  );
}

function UserTurn({ turn }: { turn: Turn }) {
  return (
    <div className="flex justify-end">
      <div className="min-w-0 max-w-[85%] rounded-card bg-accent-wash px-3 py-2 text-sm">
        {turn.images && turn.images.length > 0 && (
          <div className="mb-1.5 flex flex-wrap gap-1.5">
            {turn.images.map((src, i) => (
              <img key={i} src={src} alt="Attached" className="h-16 w-16 rounded-tile object-cover" />
            ))}
          </div>
        )}
        <p className="whitespace-pre-wrap break-words">{turn.text}</p>
        {/* Inside the wash and right-aligned with it, the way the mock draws
            it — the stamp belongs to this turn, not to the gap under it. */}
        <TurnClock at={turn.at} className="mt-1 block text-right" />
      </div>
    </div>
  );
}

/**
 * An agent's turn: its mark beside a column of `tool-call` rows, then the
 * answer. `compact` (the console dock) drops the mark — the dock already
 * carries one of its own in its header, and repeating it per turn is exactly
 * the rail-side chrome the dock has no room for.
 *
 * **The mark carries a label here (Ruling J).** `AgentMark`'s own doc makes it
 * decoration, `aria-hidden`, by default — correct beside the literal word
 * "Agent", but here it stands alone next to tool-call and answer content with
 * no such word beside it, so a screen-reader user needs the label to know
 * whose turn this is.
 *
 * **The thoughts row is absent, not merely empty, when the agent streamed
 * none.** Claude's headless mode redacts thinking entirely and Codex cannot
 * be timed (see `lib/agentRun.ts`'s `Turn.thoughts` doc) — `undefined` is the
 * ordinary case, not a gap to paper over with a header and nothing under it.
 */
function AgentTurn({ turn, compact }: { turn: Turn; compact?: boolean }) {
  return (
    <div className="flex min-w-0 items-start gap-2.5">
      {!compact && <AgentMark size={20} label="Agent" className="mt-0.5" />}
      <div className="flex min-w-0 flex-1 flex-col gap-2">
        {turn.thoughts && (
          <p className="min-w-0 whitespace-pre-wrap break-words text-xs text-faint">
            <span className="font-medium text-muted">Thoughts</span> · {turn.thoughts}
          </p>
        )}
        {turn.calls.length > 0 && (
          <div className="flex min-w-0 flex-col gap-1">
            {turn.calls.map((call) => (
              <ToolCallRow key={call.id} call={call} />
            ))}
          </div>
        )}
        {/* A warning the stream raised and then carried on past — an
            unsupported attachment, an image it could not decode. Shown
            because the reader's attachment did not arrive, and NOT allowed to
            turn the answer below it red: an answer that came back is a real
            answer. */}
        {turn.notes?.map((note, i) => (
          <p key={i} className="min-w-0 break-words text-xs text-sev">
            {note}
          </p>
        ))}
        <Answer text={turn.text} />
        {/* Under the answer, not beside the mark: the mock puts it at the end
            of what was said. Only once there IS something said — a turn still
            streaming has no finish to stamp. */}
        {turn.text !== "" && <TurnClock at={turn.at} />}
      </div>
    </div>
  );
}

/**
 * A turn the stream itself ended on. `turn.text` has already been through
 * `describeError` — `askAgent`'s `markTurnError` stamps it there, at the
 * store, the moment the failure happens — so this reads as the reader's own
 * sentence already; running it through the classifier a second time here
 * would be redundant, not safer.
 */
function ErrorTurn({ turn, compact }: { turn: Turn; compact?: boolean }) {
  return (
    <div className="flex min-w-0 items-start gap-2.5">
      {!compact && <AgentMark size={20} label="Agent" className="mt-0.5" />}
      <p className="min-w-0 flex-1 break-words text-sm text-sev">{turn.text}</p>
    </div>
  );
}

export function Transcript({
  turns,
  gates,
  compact,
  live = true,
}: {
  turns: readonly Turn[];
  gates: readonly GateRecord[];
  compact?: boolean;
  /**
   * Whether THIS component declares its own `role="log"` live region.
   * Defaults true — right for `/agent`, where this is the screen's only live
   * region (`screens/Agent.tsx`). `shell/Console.tsx` sets it false when
   * mounting this compact, because `ui-kit`'s `ConsoleDock` ALREADY wraps
   * `children` in `role="log" aria-live={live ? "polite" : "off"}`
   * (`ConsoleDock.tsx:187-193`) — a second, nested `role="log"` announces
   * inconsistently and often twice (I7).
   */
  live?: boolean;
}) {
  return (
    <div
      {...(live ? { role: "log" as const, "aria-live": "polite" as const } : {})}
      className="flex min-w-0 flex-col gap-3"
    >
      {turns.map((turn) => (
        <div key={turn.id} className="min-w-0">
          {turn.role === "user" ? (
            <UserTurn turn={turn} />
          ) : turn.role === "error" ? (
            <ErrorTurn turn={turn} compact={compact} />
          ) : (
            <AgentTurn turn={turn} compact={compact} />
          )}
        </div>
      ))}
      {gates.length > 0 && (
        <div className="flex min-w-0 flex-col gap-1">
          {gates.map((gate) => (
            <GateRow key={gate.id} gate={gate} />
          ))}
        </div>
      )}
    </div>
  );
}
