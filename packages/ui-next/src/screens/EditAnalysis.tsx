import type { ReactNode } from "react";
import { Button, type EditorDiagnostic } from "@srelens/ui-kit";
import { FailureAlert } from "../lib/errorCopy";
import type { KeysHere, SchemaStatus } from "../lib/manifestSchema";

/**
 * The editor's sidebar: what is wrong with the draft, what a dry run said,
 * and what the schema allows where the cursor is.
 *
 * Every line here is derived from something the editor or the cluster
 * already told the screen — the lint pass's findings, the on-demand dry run,
 * the kind's OpenAPI schema — laid out where a reader can see all of it at
 * once instead of hovering gutter markers one at a time.
 *
 * The diff is NOT here. It was, and 360px of manifest diff was unreadable:
 * it has its own wider column beside this one, opened from the toolbar.
 */

/** What an on-demand dry run came back with. */
export interface DryRunState {
  status: "idle" | "running" | "passed" | "failed" | "error";
  /** The API server's objections, per document, when `failed`. */
  errors: { docIndex: number; message: string }[];
  /** Why the run itself could not be made, when `error`. */
  error?: string;
}

/**
 * The one-word verdict in the toolbar — "valid", "2 problems", "unchecked" —
 * with a dot in the tone the word already carries. Never the dot alone.
 */
export function ProblemsDot({ problems, checked }: { problems: EditorDiagnostic[]; checked: boolean }) {
  const n = problems.length;
  const label = !checked ? "unchecked" : n === 0 ? "valid" : `${n} problem${n === 1 ? "" : "s"}`;
  const tone = !checked ? "text-faint" : n === 0 ? "text-ok" : "text-sev";
  return (
    <span
      className="flex items-center gap-1.5 text-xs text-muted"
      data-testid="manifest-status"
      title={
        !checked
          ? "Nothing has checked the manifest yet"
          : n === 0
            ? "The last check found nothing wrong"
            : "See Problems in the sidebar"
      }
    >
      <span aria-hidden className={tone}>
        ●
      </span>
      {label}
    </span>
  );
}

function Section({
  title,
  count,
  detail,
  children,
}: {
  title: string;
  count?: number;
  detail?: string;
  children: ReactNode;
}) {
  return (
    <section className="flex flex-col gap-2 border-b border-rule px-3 py-3 last:border-b-0">
      <h3 className="flex items-baseline justify-between gap-2 text-[10px] uppercase tracking-wide text-faint">
        <span>
          {title}
          {detail && <span className="ml-2 normal-case tracking-normal text-muted">{detail}</span>}
        </span>
        {count !== undefined && <span className="tabular-nums">{count}</span>}
      </h3>
      {children}
    </section>
  );
}

/** The first sentence of a schema description, which is the part that fits on a line. */
function lead(text: string | undefined): string {
  if (!text) return "";
  const end = text.search(/\.\s|\.$/);
  return end === -1 ? text : text.slice(0, end);
}

export function EditAnalysis({
  problems,
  checked,
  keys,
  schema,
  kind,
  schemaError,
  onRetrySchema,
  dryRun,
}: {
  problems: EditorDiagnostic[];
  /** Whether any lint pass has run at all — before one has, "no problems" would be a guess. */
  checked: boolean;
  keys: KeysHere | null;
  schema: SchemaStatus;
  kind: string | null;
  /** Why the schema lookup failed, when `schema` is `failed`. */
  schemaError?: string;
  onRetrySchema?: () => void;
  dryRun: DryRunState;
}) {
  return (
    <aside className="rule-l scroll flex w-[22rem] shrink-0 flex-col text-sm" aria-label="Analysis">
      <Section title="Problems" count={problems.length}>
        {problems.length === 0 ? (
          <p className="text-xs text-muted">
            {checked
              ? "No problems. The check is the API server's own dry run, admission included; a webhook that does not support dry run answers only on Apply."
              : "Nothing has checked the manifest yet."}
          </p>
        ) : (
          <ul className="flex flex-col gap-1.5">
            {problems.map((p, i) => (
              <li key={`${p.line}:${i}`} className="grid grid-cols-[auto_1fr] gap-2 text-xs">
                <span className="tabular-nums text-faint">L{p.line}</span>
                <span className={p.severity === "error" ? "text-sev" : "text-warn"}>{p.message}</span>
              </li>
            ))}
          </ul>
        )}
        {dryRun.status === "running" && <p className="text-xs text-muted">Dry run against the API server…</p>}
        {dryRun.status === "passed" && (
          <p className="text-xs text-ok" data-testid="dry-run-verdict">
            Dry run passed. The API server accepted the manifest as it is now.
          </p>
        )}
        {dryRun.status === "failed" && (
          <div className="flex flex-col gap-1" data-testid="dry-run-verdict">
            <p className="text-xs text-sev">Dry run failed.</p>
            <ul className="flex flex-col gap-1">
              {dryRun.errors.map((e, i) => (
                <li key={i} className="grid grid-cols-[auto_1fr] gap-2 text-xs">
                  <span className="tabular-nums text-faint">doc {e.docIndex + 1}</span>
                  <span className="text-sev">{e.message}</span>
                </li>
              ))}
            </ul>
          </div>
        )}
        {dryRun.status === "error" && <FailureAlert title="Could not dry-run the manifest" error={dryRun.error} />}
      </Section>

      <Section
        title={keys?.onValue ? "Values valid here" : "Keys valid here"}
        detail={
          keys ? (keys.onValue ? keys.valueKey : keys.path.length > 0 ? keys.path.join(" › ") : "top level") : undefined
        }
      >
        {schema === "none" ? (
          <p className="text-xs text-muted">Name an apiVersion and a kind to see what goes here.</p>
        ) : schema === "loading" ? (
          <p className="text-xs text-muted">Loading the schema…</p>
        ) : schema === "failed" ? (
          // Not "the cluster has no schema for Secret": the cluster was not
          // heard from. Which it is changes what the reader does next.
          <div className="flex flex-col items-start gap-2">
            <p className="text-xs text-muted">
              Could not read {kind ?? "this kind"}&apos;s schema from the cluster, so nothing here can
              say what goes where the cursor is.
            </p>
            {schemaError && <p className="text-xs text-sev">{schemaError}</p>}
            {onRetrySchema && (
              <Button variant="ghost" onClick={onRetrySchema}>
                Try again
              </Button>
            )}
          </div>
        ) : schema === "absent" ? (
          <p className="text-xs text-muted">
            This cluster publishes no schema for {kind ?? "this kind"} — a CustomResourceDefinition
            with none of its own, or a kind spelled differently from any it serves.
          </p>
        ) : !keys || keys.entries.length === 0 ? (
          <p className="text-xs text-muted">Nothing more goes here.</p>
        ) : (
          <ul className="flex flex-col gap-1">
            {keys.entries.map((e) => (
              <li key={e.label} className="grid grid-cols-[auto_1fr_auto] items-baseline gap-2 text-xs">
                <code className="text-accent">{e.label}</code>
                <span className="truncate text-muted" title={e.info}>
                  {lead(e.info)}
                </span>
                <span className="text-faint">{e.detail}</span>
              </li>
            ))}
          </ul>
        )}
      </Section>
    </aside>
  );
}
