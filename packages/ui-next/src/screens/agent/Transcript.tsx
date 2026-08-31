import { parseAssistantMarkdown, type MdBlock, type NoteSpan } from "@srelens/core";
import { AgentMark, Badge, cx, type Tone } from "@srelens/ui-kit";
import type { GateRecord, ToolCallRecord, Turn } from "../../lib/agentRun";
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
};

const GATE_WORD: Record<GateRecord["outcome"], string> = {
  pending: "Pending",
  approved: "Applied",
  denied: "Denied",
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

/** One gate, as decision 1's record — never a second set of answer buttons. */
function GateRow({ gate }: { gate: GateRecord }) {
  const args = summarizeArgs(gate.args);
  return (
    <div className="tool-call flex min-w-0 items-center gap-2">
      <span className="shrink-0 text-accent">{gate.tool}</span>
      {args !== "" && <span className="min-w-0 flex-1 truncate text-faint">{args}</span>}
      <Badge tone={GATE_TONE[gate.outcome]}>{gateLabel(gate)}</Badge>
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
        <Answer text={turn.text} />
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
