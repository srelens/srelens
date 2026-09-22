import { useMemo, useState, type ReactNode } from "react";
import type { CapabilityImpact, DiffRow } from "@srelens/core";
import { DiffLines } from "@srelens/ui-kit";
import { collapseDiff } from "../lib/diffCollapse";
import { escapeFormatCharacters, plainText } from "../extensions/displayText";

/**
 * The one confirmation srelens asks before a write, wherever the write came
 * from.
 *
 * **Why there is exactly one.** A write clicked in an app's resource view and
 * the same write asked for by an agent used to be two confirmations with two
 * implementations: an inline `role="dialog"` in `ExtensionResourceDetails`
 * carrying descriptions hardcoded in a UI constant, and a modal driven from
 * `mcp_confirm.rs` on three surfaces of its own. Two implementations are two
 * sets of words, and #549 will let an app's own `title` reach the sentence —
 * at which point "which of these two prompts am I looking at" becomes a
 * question an attacker gets to answer. So the question is built here, once,
 * and each surface supplies only its frame.
 *
 * **What it names**, always and in this order: the level, the host's sentence,
 * the pinned cluster, the object (or the count, for a bulk selection), and the
 * app that asked with its publisher — or the word `unsigned`. A frame may add
 * detail underneath through {@link HostConfirmationProps.details} and its own
 * buttons through {@link HostConfirmationProps.actions}; it cannot subtract,
 * reword or restyle any of the above. The transcript's inline card is smaller
 * than the modal, and the same question in a smaller frame must not become a
 * different question.
 *
 * **The words are the host's.** `question` is rendered by the host from a
 * template compiled into the binary, through the closed placeholder vocabulary
 * (`srelens_capability::CONFIRM_FIELDS` in Rust, `CONFIRM_TEMPLATE_FIELDS` in
 * `@srelens/core`). This component composes nothing from arguments and reads
 * none: what it is handed is what it draws. A `question` of `null` draws no
 * sentence at all rather than half of one — the surfaces then fall back to
 * what they showed before, which is a worse prompt than a good sentence and a
 * better one than "Drain ?" over an Approve button.
 *
 * **Nothing an app supplies gets a hook.** Every value that originated outside
 * the host — the app's name, its publisher, the object's namespace and name,
 * the patch — goes through {@link plainText} or
 * {@link escapeFormatCharacters} and is bounded to
 * {@link CONFIRM_DISPLAY_MAX_CHARS}, the same ceiling
 * `srelens_capability::annotations::CONFIRM_FIELD_MAX_CHARS` applies in Rust.
 * There is no `className`, no `style` and no slot above the buttons, so there
 * is no way to dress this as a permission prompt, an OS dialog or another
 * app's UI.
 */

/** What the level is called in front of a reader. Never a colour alone. */
const IMPACT_LABEL: Record<CapabilityImpact, string> = {
  low: "Low impact",
  medium: "Medium impact",
  high: "High impact",
};

/**
 * The longest any one value from outside the host may be drawn, in characters.
 *
 * Mirrors `CONFIRM_FIELD_MAX_CHARS` (`crates/capability/src/annotations.rs`)
 * so a value bounded by the backend and one bounded here are cut at the same
 * place. A Kubernetes name may legally run to 253 characters and a manifest's
 * `name` is only bounded by the manifest schema, so this cuts rather than
 * refuses: the question has the words that matter at both ends, and a value
 * long enough to push the tail out of the frame is the cheapest spoof there
 * is.
 */
export const CONFIRM_DISPLAY_MAX_CHARS = 80;

/**
 * One value from outside the host, as it may appear in the question: format,
 * control and separator characters written as `\uXXXX` and the result bounded.
 *
 * Escape first, then bound, for the reason the Rust side does: one override
 * escapes to six characters, so bounding the input would let eighty characters
 * draw as four hundred and eighty.
 */
export function boundedPlainText(value: string): string {
  const escaped = plainText(value);
  return [...escaped].length <= CONFIRM_DISPLAY_MAX_CHARS
    ? escaped
    : [...escaped].slice(0, CONFIRM_DISPLAY_MAX_CHARS - 1).join("") + "…";
}

/** The app a write was asked for through, as the HOST knows it. */
export interface ConfirmationApp {
  /**
   * What the host calls the app. Read from the host's own installed inventory
   * (`extensionLabel`), never from the request — so a caller cannot name
   * itself, and a quarantined app is named by its ID rather than by a stored
   * name the host no longer accepts.
   */
  name: string;
  /** Who signed it; `null` when it carries no signature, drawn as `unsigned`. */
  publisher: string | null;
}

/**
 * What the call is about to change.
 *
 * `bulk` exists now although #553 has not landed: the count is the only part
 * of this question a bulk execution changes, and leaving it out would mean
 * rewriting the one confirmation the moment the first one arrives.
 */
export type ConfirmationSubject =
  | { kind: "object"; namespace?: string | null; name: string }
  | { kind: "bulk"; count: number };

export interface HostConfirmationProps {
  /** The host's own sentence, already rendered. `null` draws none. */
  question: string | null;
  /**
   * The host's level for this call. `null` draws no badge: the level is a
   * fact about the capability and this component does not guess one, because
   * a badge that appears whether or not it means anything is worth nothing on
   * the prompt where it matters.
   */
  impact: CapabilityImpact | null;
  /** The kubeconfig context the call is pinned to. */
  cluster?: string | null;
  subject?: ConfirmationSubject | null;
  app?: ConfirmationApp | null;
  /** The exact patch, as the manifest diff renderer's rows. */
  patch?: DiffRow[] | null;
  /** `card` is the assistant transcript's inline frame, which is small. */
  frame?: "dialog" | "card";
  /** The frame's own extra detail — the tool id, the argument payload, a queue count. */
  details?: ReactNode;
  /** The frame's own buttons, for a frame whose chrome has none. */
  actions?: ReactNode;
}

/** `team/api`, `api`, or `12 resources`. */
function describeSubject(subject: ConfirmationSubject): string {
  if (subject.kind === "bulk") {
    return `${subject.count.toLocaleString("en-US")} ${subject.count === 1 ? "resource" : "resources"}`;
  }
  const name = boundedPlainText(subject.name);
  return subject.namespace ? `${boundedPlainText(subject.namespace)}/${name}` : name;
}

/** The patch with every format character written out, so no line can reorder another. */
function safeRows(rows: DiffRow[]): DiffRow[] {
  return rows.map((row) => ({
    tag: row.tag,
    left: row.left === null ? null : escapeFormatCharacters(row.left),
    right: row.right === null ? null : escapeFormatCharacters(row.right),
  }));
}

/** The changed lines, a little either side, and a counted gap for each long unchanged run. */
function Patch({ rows }: { rows: DiffRow[] }) {
  const safe = useMemo(() => safeRows(rows), [rows]);
  const segments = useMemo(() => collapseDiff(safe), [safe]);
  const [opened, setOpened] = useState<Set<number>>(new Set());
  if (safe.length === 0) return null;
  return (
    <div className="host-confirm-patch" data-testid="host-confirm-patch">
      {segments.map((segment) =>
        segment.kind === "rows" || opened.has(segment.from) ? (
          <DiffLines key={segment.from} rows={segment.rows} wrap={false} />
        ) : (
          <button
            key={segment.from}
            type="button"
            className="host-confirm-gap"
            onClick={() => setOpened((prev) => new Set(prev).add(segment.from))}
          >
            <span aria-hidden>⋯</span> Show {segment.rows.length} unchanged{" "}
            {segment.rows.length === 1 ? "line" : "lines"}
          </button>
        ),
      )}
    </div>
  );
}

export function HostConfirmation({
  question,
  impact,
  cluster,
  subject,
  app,
  patch,
  frame = "dialog",
  details,
  actions,
}: HostConfirmationProps) {
  return (
    <div className={`host-confirm host-confirm-${frame}`}>
      {impact !== null && (
        <p className="host-confirm-impact" data-impact={impact} data-testid="host-confirm-impact">
          {IMPACT_LABEL[impact]}
        </p>
      )}
      {question !== null && question !== "" && (
        <p className="host-confirm-question" data-testid="host-confirm-question">
          {question}
        </p>
      )}
      {(cluster || subject) && (
        <dl className="host-confirm-facts">
          {cluster ? (
            <>
              <dt>Cluster</dt>
              <dd data-testid="host-confirm-cluster">{boundedPlainText(cluster)}</dd>
            </>
          ) : null}
          {subject ? (
            <>
              <dt>{subject.kind === "bulk" ? "Selection" : "Resource"}</dt>
              <dd data-testid="host-confirm-target">{describeSubject(subject)}</dd>
            </>
          ) : null}
        </dl>
      )}
      {app && (
        <p className="host-confirm-requester" data-testid="host-confirm-requester">
          Requested by app{" "}
          <span data-testid="host-confirm-app-name">{boundedPlainText(app.name)}</span> (
          {app.publisher === null ? "unsigned" : boundedPlainText(app.publisher)})
        </p>
      )}
      {patch && patch.length > 0 && <Patch rows={patch} />}
      {details}
      {actions && <div className="host-confirm-actions">{actions}</div>}
    </div>
  );
}
