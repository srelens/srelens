import { parseAssistantMarkdown, type CapabilityImpact, type MdBlock, type NoteSpan } from "@srelens/core";
import { useId, useState } from "react";
import { AgentMark, Badge, CopyButton, Spinner, cx, toneWash, type Tone } from "@srelens/ui-kit";
import type { GateRecord, ToolCallRecord, Turn } from "../../lib/agentRun";
import { CAPABILITY_CATALOG } from "@srelens/core";
import { IMPACT_LABEL } from "../../confirm/HostConfirmation";
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
 *
 * **The app's type scale, not the framework's (#946).** Text is set in the
 * `--text-body` / `--text-label` / `--text-meta` tokens every other screen
 * uses, so an answer reads at the size of the list beside it and its tables
 * at the size of the app's own. Tailwind's `text-sm`/`text-xs` are 14px and
 * 12px, sizes nothing else in the app is set in.
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

/** The confirmation's own colours for a level (`hostConfirmation.css`): the
 *  default ink for low, warn for medium, sev for high. */
const IMPACT_TONE: Record<CapabilityImpact, Tone> = { low: "muted", medium: "warn", high: "sev" };

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
 * has actually failed or been refused), args faint, what the result said, and
 * the round trip srelens itself measured, right-aligned.
 *
 * **The result is the backend's reading of the call's own output** (#385):
 * a count, a kind and name, or a first line — never a summary srelens wrote
 * for a call it did not see. Drawn in the row's tone when the call failed or
 * was refused, since then it is the reason.
 *
 * **`ms` is rendered only when it exists.** `ToolCallRecord.ms` is absent
 * until `toolResult` lands (see its doc in `lib/agentRun.ts`) — a call still
 * in flight has measured nothing yet, and printing `0ms` for it would tell
 * the reader a round trip that hasn't happened already finished instantly.
 */
function ToolCallRow({ call }: { call: ToolCallRecord }) {
  const args = summarizeArgs(call.args);
  const capability = call.status === "error" ? "text-sev" : call.status === "denied" ? "text-warn" : "text-accent";
  const result = call.status === "error" ? "text-sev" : call.status === "denied" ? "text-warn" : "text-faint";
  // The tone says it, and so does a word (design.md §5, PR #806 review): a
  // failed `0 pods` and a successful one differ by more than colour. A call
  // that succeeded carries no status tone, and its duration says it finished.
  const outcome = call.status === "error" ? "Failed" : call.status === "denied" ? "Denied" : undefined;
  return (
    <div className="tool-call flex min-w-0 items-center gap-2">
      <span className={cx("shrink-0", capability)}>{call.tool}</span>
      {args !== "" && <span className="min-w-0 flex-1 truncate text-faint">{args}</span>}
      {outcome !== undefined && <span className={cx("shrink-0", result)}>{outcome}</span>}
      {call.summary !== undefined && <span className={cx("min-w-0 shrink truncate", result)}>{call.summary}</span>}
      {call.ms !== undefined && <span className="ml-auto shrink-0 text-faint">{Math.round(call.ms)}ms</span>}
    </div>
  );
}

/**
 * A tool's name as a reader says it: the MCP server prefix an agent's client
 * puts on every call (`mcp__srelens__k8s_listPods`) dropped, since in this
 * app every tool is srelens's. The expanded rows keep the name as called.
 */
export function shortToolName(tool: string): string {
  const match = /^mcp__[^_]+(?:_[^_]+)*?__(.+)$/.exec(tool);
  return match ? match[1] : tool;
}

/** `850ms`, or `1.9s` from a second up — for a sum of round trips. */
function duration(ms: number): string {
  return ms < 1000 ? `${Math.round(ms)}ms` : `${(ms / 1000).toFixed(1)}s`;
}

/**
 * An agent turn's tool calls, folded into one line until the reader asks for
 * them (#946) — the way Claude's own interfaces show them. The calls are how
 * an answer was reached, not the answer, and drawn in full they put boxes of
 * JSON between a question and what was asked for.
 *
 * **Folded, the line still says what is going on.** While the turn is
 * working, the call in flight: `Running k8s_listPods…`, beside a spinner.
 * Once it is done: how many tools it used, the last one and what it came
 * back with, and the time the calls took. A failed or refused call is counted
 * in words and in its tone on the folded line too, because a missing fact in
 * the answer is often explained by one.
 *
 * Open per turn and local to it: a reader opening one turn's calls is not
 * asking to see every turn's.
 */
function ToolCalls({
  calls,
  working,
  gates = [],
}: {
  calls: readonly ToolCallRecord[];
  working: boolean;
  /** This turn's gates (#959): under the folded line, or each under its call. */
  gates?: readonly GateRecord[];
}) {
  const [open, setOpen] = useState(false);
  const listId = useId();
  // A call with no status has not returned. Only while the turn is still
  // working is that "running": a restored conversation can hold calls whose
  // result was never recorded, and those are not running now.
  const running = working ? calls.find((c) => c.status === null) : undefined;
  const failed = calls.filter((c) => c.status === "error").length;
  const denied = calls.filter((c) => c.status === "denied").length;
  const took = calls.reduce((sum, c) => sum + (c.ms ?? 0), 0);
  const last = calls[calls.length - 1];
  const count = `${calls.length} tool${calls.length === 1 ? "" : "s"}`;
  return (
    <div className="flex min-w-0 flex-col gap-1" data-slot="tool-calls">
      <button
        type="button"
        className="flex min-w-0 items-center gap-1.5 self-start rounded-tile py-0.5 pr-1 text-left text-[length:var(--text-label)] text-muted hover:text-ink"
        aria-expanded={open}
        aria-controls={open ? listId : undefined}
        onClick={() => setOpen((o) => !o)}
      >
        <svg width="10" height="10" viewBox="0 0 24 24" fill="none" aria-hidden="true" className={cx("shrink-0 transition-transform", open && "rotate-90")}>
          <path d="m9 6 6 6-6 6" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
        {running ? (
          <>
            {/* The words say it; the spinner is only motion. */}
            <span aria-hidden="true" className="flex shrink-0"><Spinner className="size-2.5" /></span>
            <span className="min-w-0 truncate whitespace-nowrap">
              Running <span className="font-mono text-ink">{shortToolName(running.tool)}</span>…
            </span>
            {calls.length > 1 && <span className="shrink-0 whitespace-nowrap text-faint">{count}</span>}
          </>
        ) : (
          <>
            <span className="shrink-0 whitespace-nowrap">Used {count}</span>
            <span className="min-w-0 truncate whitespace-nowrap text-faint">
              · <span className="font-mono">{shortToolName(last.tool)}</span>
              {last.summary !== undefined && <> → {last.summary}</>}
            </span>
            {took > 0 && <span className="shrink-0 whitespace-nowrap text-faint">· {duration(took)}</span>}
          </>
        )}
        {failed > 0 && <span className="shrink-0 whitespace-nowrap text-sev">· {failed} failed</span>}
        {denied > 0 && <span className="shrink-0 whitespace-nowrap text-warn">· {denied} denied</span>}
      </button>
      {/* The turn's gates stay in sight while its calls are folded: a gate
          waiting for an answer, or one the reader gave, is the step in this
          turn the reader took part in (#959). */}
      {!open && <GateList gates={gates} />}
      {open && (
        <div id={listId} className="flex min-w-0 flex-col gap-1">
          {calls.map((call) => (
            <div key={call.id} className="flex min-w-0 flex-col gap-1">
              <ToolCallRow call={call} />
              <GateList gates={gates.filter((g) => g.callId === call.id)} />
            </div>
          ))}
          {/* Gates no call in the turn could be matched to, after its calls. */}
          <GateList gates={gates.filter((g) => !calls.some((c) => c.id === g.callId))} />
        </div>
      )}
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
 *   pods… Expected full recovery in ~90 s. Reversible…"). What the card shows
 *   instead is the host's own sentence for the call — "Roll every pod of
 *   Deployment shop/checkout-api in cluster prod-eu?" — and its impact level
 *   (#388): the question the reader actually answered, rendered by the host
 *   from the capability's template, never by this component. Recovery time
 *   and reversibility are not things srelens knows, so they stay out.
 *
 * The badges ARE honest: `destructive` is a real field on the capability
 * registry, so this reads it rather than guessing from the tool's name, and
 * the impact is the host's, named in the confirmation's own words.
 */
function GateRow({ gate }: { gate: GateRecord }) {
  // Folded by default (#959): one line saying what was asked, how much it
  // disturbs and what came of it. The question and the arguments are the
  // record's detail, opened when the reader wants them — drawn in full they
  // took as much room as the answer, and the arguments name the very things
  // a reader may not want on screen.
  const [open, setOpen] = useState(false);
  const detailsId = useId();
  const facts = CAPABILITY_CATALOG.find((c) => c.id === gate.tool);
  const args = prettyArgs(gate.args);
  return (
    <div className="min-w-0 rounded-card border border-rule" data-slot="gate" data-outcome={gate.outcome}>
      <button
        type="button"
        className={cx("flex w-full min-w-0 items-center gap-2 px-3 py-1.5 text-left", open ? "rounded-t-card" : "rounded-card")}
        style={{ background: toneWash(GATE_TONE[gate.outcome]) }}
        aria-expanded={open}
        aria-controls={open ? detailsId : undefined}
        onClick={() => setOpen((o) => !o)}
      >
        <svg width="10" height="10" viewBox="0 0 24 24" fill="none" aria-hidden="true" className={cx("shrink-0 text-muted transition-transform", open && "rotate-90")}>
          <path d="m9 6 6 6-6 6" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
        <span className="min-w-0 truncate whitespace-nowrap font-mono text-[length:var(--text-label)] font-medium text-ink">{gate.tool}</span>
        <span className="flex-1" />
        {gate.impact !== undefined && <Badge tone={IMPACT_TONE[gate.impact]}>{IMPACT_LABEL[gate.impact]}</Badge>}
        {facts?.destructive === true && <Badge tone="sev">destructive</Badge>}
        <Badge tone={GATE_TONE[gate.outcome]}>{GATE_WORD[gate.outcome]}</Badge>
        {/* The TIME only: the badge already carries the word. */}
        {gate.at !== undefined && <TurnClock at={gate.at} className="shrink-0" />}
      </button>
      {open && (
        <div id={detailsId} className="min-w-0">
          {gate.prompt !== undefined && (
            <p className="min-w-0 break-words px-3 py-2 text-[length:var(--text-body)] text-ink">{gate.prompt}</p>
          )}
          {args !== "" && (
            <pre className="scroll m-0 max-h-40 overflow-auto whitespace-pre-wrap break-words rounded-b-card bg-canvas-deep px-3 py-2 font-mono text-[length:var(--text-meta)] leading-relaxed text-faint">
              {args}
            </pre>
          )}
        </div>
      )}
    </div>
  );
}

/** Gates, one under the other. */
function GateList({ gates }: { gates: readonly GateRecord[] }) {
  if (gates.length === 0) return null;
  return (
    <div className="flex min-w-0 flex-col gap-1">
      {gates.map((gate) => (
        <GateRow key={gate.id} gate={gate} />
      ))}
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
            <code key={i} className="break-words rounded-tile bg-sunk px-1 py-0.5 font-mono text-[length:var(--text-label)]">
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
        ? "text-[0.9375rem] font-semibold"
        : block.level === 2
          ? "text-[length:var(--text-body)] font-semibold"
          : "text-[length:var(--text-body)] font-semibold text-muted";
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
      <pre className="min-w-0 overflow-x-auto rounded-tile border border-rule bg-sunk p-2 font-mono text-[length:var(--text-label)]">
        <code>{block.text}</code>
      </pre>
    );
  }
  if (block.kind === "table") {
    return (
      <div className="min-w-0 overflow-x-auto">
        <table className="w-full border-collapse text-[length:var(--text-body)]">
          <thead>
            <tr>
              {block.headers.map((h, j) => (
                <th
                  key={j}
                  className="break-words border-b border-rule px-2 py-1 text-left text-[length:var(--text-label)] font-medium text-faint"
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
    <div className="flex min-w-0 flex-col gap-2 text-[length:var(--text-body)] leading-relaxed">
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
    <span className={cx("text-[length:var(--text-meta)] tabular-nums text-faint", className)}>
      {`${pad2(d.getHours())}:${pad2(d.getMinutes())}:${pad2(d.getSeconds())}`}
    </span>
  );
}

function UserTurn({ turn }: { turn: Turn }) {
  return (
    // NO fill of its own. §5 draws the question in an accent wash, and inside
    // an exchange card that wash is a second canvas — so one exchange read as
    // two, which is what got reported after the container went in. The
    // exchange is the canvas; the question is told apart by weight and by
    // sitting right, not by having a surface of its own.
    <div className="flex justify-end">
      <div className="min-w-0 max-w-[85%] px-0 py-0 text-right text-[length:var(--text-body)] font-medium text-ink">
        {turn.images && turn.images.length > 0 && (
          <div className="mb-1.5 flex flex-wrap justify-end gap-1.5">
            {turn.images.map((src, i) => (
              <img key={i} src={src} alt="Attached" className="h-16 w-16 rounded-tile object-cover" />
            ))}
          </div>
        )}
        <p className="whitespace-pre-wrap break-words">{turn.text}</p>
        {/* Right-aligned with the question it belongs to — and absent when the
            time was never recorded (a conversation restored from classic,
            whose stored messages carry none). A borrowed stamp under every
            turn would be srelens claiming a time it was never told. */}
        {turn.atRecorded !== false && <TurnClock at={turn.at} className="mt-0.5 block text-right" />}
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
function AgentTurn({
  turn,
  compact,
  working,
  gates = [],
}: {
  turn: Turn;
  compact?: boolean;
  working?: boolean;
  gates?: readonly GateRecord[];
}) {
  return (
    <div className="flex min-w-0 items-start gap-2.5">
      {!compact && <AgentMark size={20} label="Agent" className="mt-0.5" />}
      <div className="flex min-w-0 flex-1 flex-col gap-2">
        {turn.thoughts && (
          <p className="min-w-0 whitespace-pre-wrap break-words text-[length:var(--text-label)] text-faint">
            <span className="font-medium text-muted">Thoughts</span> · {turn.thoughts}
          </p>
        )}
        {turn.calls.length > 0 ? (
          <ToolCalls calls={turn.calls} working={working === true} gates={gates} />
        ) : (
          <GateList gates={gates} />
        )}
        {/* A warning the stream raised and then carried on past — an
            unsupported attachment, an image it could not decode. Shown
            because the reader's attachment did not arrive, and NOT allowed to
            turn the answer below it red: an answer that came back is a real
            answer. */}
        {turn.notes?.map((note, i) => (
          <p key={i} className="min-w-0 break-words text-[length:var(--text-label)] text-sev">
            {note}
          </p>
        ))}
        <Answer text={turn.text} />
        {/* Under the answer, not beside the mark: the mock puts it at the end
            of what was said. Only once there IS something said — a turn still
            streaming has no finish to stamp. */}
        {turn.text !== "" && turn.atRecorded !== false && <TurnClock at={turn.at} />}
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
      <p className="min-w-0 flex-1 break-words text-[length:var(--text-body)] text-sev">{turn.text}</p>
    </div>
  );
}

/**
 * The turns, grouped into exchanges: each starts at the reader's question and
 * carries everything that answered it.
 *
 * `turns` is flat, and drawn flat it gave no seam between one question and the
 * next — a long answer ran straight into the following question. A leading run
 * of non-user turns (a restored transcript, an answer with no recorded
 * question) becomes its own group rather than being dropped.
 */
function exchanges(turns: readonly Turn[]): Turn[][] {
  const out: Turn[][] = [];
  for (const turn of turns) {
    if (turn.role === "user" || out.length === 0) out.push([turn]);
    else out[out.length - 1].push(turn);
  }
  return out;
}

/**
 * An exchange as plain text, for the clipboard — the question, then what was
 * said back.
 *
 * Tool calls are left out on purpose: they are how the answer was reached, not
 * the answer, and a paste full of `k8s_listPods {...}` buries the part the
 * reader wanted. Empty when nothing has been said yet, so the control does not
 * offer to copy a question on its own.
 */
function exchangeText(exchange: readonly Turn[]): string {
  const said = exchange.filter((t) => t.role !== "user" && t.text !== "");
  if (said.length === 0) return "";
  const asked = exchange.find((t) => t.role === "user");
  return [asked?.text, ...said.map((t) => t.text)].filter(Boolean).join("\n\n");
}

/**
 * The whole conversation as plain text, for the clipboard.
 *
 * Built from the same {@link exchangeText} each exchange's own control uses,
 * so what a reader gets from the header is exactly the exchanges concatenated
 * — not a second, subtly different rendering of the same chat. Exchanges with
 * nothing said yet drop out, the way the per-exchange control is withheld.
 */
export function transcriptText(turns: readonly Turn[]): string {
  return exchanges(turns)
    .map(exchangeText)
    .filter((t) => t !== "")
    .join("\n\n---\n\n");
}

export function Transcript({
  turns,
  gates,
  compact,
  busy = false,
  live = true,
}: {
  turns: readonly Turn[];
  gates: readonly GateRecord[];
  compact?: boolean;
  /** The run is answering: its last agent turn is the one still working. */
  busy?: boolean;
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
  // Each gate at the turn that asked for it (#959); one whose turn is not in
  // this transcript — recorded before gates kept their place, or raised with
  // no answer in progress — after everything, as before.
  const turnIds = new Set(turns.map((t) => t.id));
  const placed = new Map<number, GateRecord[]>();
  const trailing: GateRecord[] = [];
  for (const gate of gates) {
    if (gate.turnId !== undefined && turnIds.has(gate.turnId)) {
      placed.set(gate.turnId, [...(placed.get(gate.turnId) ?? []), gate]);
    } else {
      trailing.push(gate);
    }
  }
  return (
    <div
      {...(live ? { role: "log" as const, "aria-live": "polite" as const } : {})}
      className="flex min-w-0 flex-col gap-3"
    >
      {exchanges(turns).map((exchange) => (
        <div
          key={exchange[0].id}
          // One EXCHANGE per container: the question and everything answering
          // it, inside a thin border. A flat run of turns left no seam between
          // one question and the next, so a long answer and the question after
          // it read as continuous.
          className="flex min-w-0 flex-col gap-2 rounded-card border border-rule bg-[var(--ground-surface)] px-3 py-2.5"
        >
          {exchange.map((turn) => (
            <div key={turn.id} className="min-w-0">
              {turn.role === "user" ? (
                <UserTurn turn={turn} />
              ) : turn.role === "error" ? (
                <ErrorTurn turn={turn} compact={compact} />
              ) : (
                <AgentTurn
                  turn={turn}
                  compact={compact}
                  working={busy && turn === turns[turns.length - 1]}
                  gates={placed.get(turn.id)}
                />
              )}
            </div>
          ))}
          {/* One copy per exchange, not per turn: what a reader wants is the
              question and its answer together — pasting an answer with no
              question is half a record. Only once there IS an answer. */}
          {exchangeText(exchange) !== "" && (
            <div className="flex justify-end">
              <CopyButton text={exchangeText(exchange)} label="Copy this question and answer" iconOnly />
            </div>
          )}
        </div>
      ))}
      <GateList gates={trailing} />
    </div>
  );
}
