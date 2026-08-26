import { useEffect, useId, useMemo, useState } from "react";
import {
  type DiffRow,
  type Invoker,
  diffTextLines,
  getHelmRelease,
  helmStatus,
} from "@srelens/core";
import {
  Alert,
  Badge,
  Button,
  DiffLines,
  EmptyState,
  LiveSignal,
  LoadingState,
  statusTone,
} from "@srelens/ui-kit";
import { FailureState } from "../../lib/errorCopy";
import { type HelmOpRow, dismissHelmOp } from "../../lib/helmOps";

/** §16's second pane, in px. */
const WIDTH = 420;

/**
 * The default `ops`, hoisted.
 *
 * A `[]` written in the parameter list is a fresh array on every render, which
 * is the one case where the `useMemo` below can never hit: the default caller
 * — a screen with no operations in flight — would re-run `currentOp` for every
 * keystroke anywhere above it.
 */
const NO_OPS: readonly HelmOpRow[] = [];

/** The release the pane is looking at — one row of §16's release table. */
export interface PaneRelease {
  name: string;
  namespace: string;
  /** The revision running now: the diff's right-hand side. */
  revision: number;
  /** Helm's own status word. Toned by core's `helmStatus`, never by this file. */
  status: string;
}

export interface ReleasePaneProps {
  /** The cluster the release lives in — a kubeconfig context NAME. */
  context: string;
  /**
   * The selected release, or `null` when nothing is selected.
   *
   * §16's own pane is hard-wired to `checkout` and says clicking a release row
   * does nothing. This one follows the selection, which changes both the diff
   * and which operation is considered.
   */
  release: PaneRelease | null;
  /**
   * Every operation this window has started. The pane picks its own out of
   * them — see {@link currentOp}.
   *
   * A prop rather than a subscription of its own: the screen above already
   * reads the store for its table and its status strip, and three
   * `useSyncExternalStore` subscriptions to one module is three renders per
   * printed line.
   */
  ops?: readonly HelmOpRow[];
  /** Drop a finished operation's row. The store's own by default. */
  onDismiss?: (id: number) => void;
  /** §16's footer: roll back to the diff's left-hand revision. */
  onRollback?: (revision: number) => void;
  /** §16's footer: open the values editor — the dialog in `upgrade` mode. */
  onValuesEditor?: () => void;
  /** The capability invoker; core's default when omitted. */
  invoke?: Invoker;
}

/**
 * The operation this pane is about, out of everything the window is running.
 *
 * **Running outranks failed, and both outrank the diff.** A `helm upgrade`
 * still in flight is the thing the reader came to look at; a failure is the
 * only place its reason exists, because nothing else on the screen carries it.
 * A diff of two revisions is true whenever it is asked for and can wait.
 *
 * **A `done` operation never takes the pane, but it does retire a failure.**
 * Those are two different rules and collapsing them into one is a bug the
 * reader cannot get out of. Who-wins: a success moved the release on, and the
 * diff of the revision it produced says more than its own output does, so it
 * yields. Retirement: an attempt that started AFTER a failure — `done` or
 * `running` — is evidence that failure is spent, so it drops out of the pool
 * rather than holding the pane forever. Without the second rule a 10:00
 * upgrade that failed on an expired token keeps a red banner and its stale
 * output over a release the 10:05 retry left `deployed`, with the diff of the
 * revision that retry produced unreachable and Dismiss the only way out.
 *
 * **Retiring a failure here does not retire the ROW.** The store keeps it
 * until the reader dismisses it and the status strip goes on counting it,
 * deliberately: the strip answers "a failure happened and you have not
 * acknowledged it", this pane answers "here is what is worth looking at right
 * now". Dismissing clears both. Nothing in this function touches the store.
 *
 * Ties go to the newest start, then to the higher id — two operations started
 * inside the same millisecond are ordered by the sequence that registered
 * them rather than by the order the array happens to be in.
 */
export function currentOp(
  ops: readonly HelmOpRow[],
  context: string,
  release: PaneRelease | null,
): HelmOpRow | null {
  if (!release) return null;
  const mine = ops.filter(
    (o) => o.context === context && o.namespace === release.namespace && o.release === release.name,
  );
  const running = mine.filter((o) => o.state === "running");
  if (running.length > 0) return newest(running);
  // Every attempt that is not itself a failure. A failure with one of these
  // after it has been superseded and no longer holds the pane.
  const attempts = mine.filter((o) => o.state !== "failed");
  const unspent = mine.filter(
    (o) => o.state === "failed" && !attempts.some((later) => startedAfter(later, o)),
  );
  if (unspent.length === 0) return null;
  return newest(unspent);
}

/** Did `a` start after `b`? The tie-break is the store's id, which only rises. */
function startedAfter(a: HelmOpRow, b: HelmOpRow): boolean {
  return a.startedAt > b.startedAt || (a.startedAt === b.startedAt && a.id > b.id);
}

/** The latest-started row of a non-empty list. */
function newest(rows: readonly HelmOpRow[]): HelmOpRow {
  return rows.reduce((best, o) => (startedAfter(o, best) ? o : best));
}

/** Where the diff stands. */
type DiffLoad =
  | { status: "idle" }
  | { status: "loading" }
  | { status: "ready"; left: number; right: number; rows: DiffRow[] }
  | { status: "error"; error: string };

/**
 * §16's second pane: **a helm operation where the diff would be**.
 *
 * Two things can fill it and only one of them at a time. A running or failed
 * operation for the selected release wins — see {@link currentOp} — and
 * otherwise the pane diffs the release's current revision against the one
 * before it.
 *
 * **A release with one revision has nothing to diff, and says so.** This is
 * the commonest case on a freshly installed chart, and it is NOT the same
 * claim as an empty diff: "no changes" says srelens compared two manifests
 * and found them equal, which on revision 1 it did not do — there was no
 * left-hand side to compare with. Both sentences exist below and neither is
 * reachable from the other's state.
 *
 * **Nothing here cancels anything.** `dismissHelmOp` declines to remove a
 * `running` row by design, so this pane draws no dismiss control on one: a
 * button that looks like it stops a `helm upgrade` and does not would be worse
 * than no button, and the upgrade continues whether srelens watches or not.
 *
 * **A failed operation's reason is printed as the store wrote it.** `helmOps`
 * already ran it through `describeError`; running the resulting sentence
 * through it a second time classifies it on its own wording. Everything this
 * file fetches for itself does go through `describeError`, via
 * {@link FailureState}.
 *
 * **No status word or tone is invented here.** The head's badge is
 * `helmStatus`'s verdict on Helm's own word, tone included. The one thing this
 * file says about an operation — that it is live — is said by `LiveSignal`,
 * which is a `status` region, rather than by a word looked up in a table of
 * this file's own; ten of those were removed on this migration.
 */
export function ReleasePane({
  context,
  release,
  ops = NO_OPS,
  onDismiss = dismissHelmOp,
  onRollback,
  onValuesEditor,
  invoke,
}: ReleasePaneProps) {
  const headId = useId();
  const op = useMemo(() => currentOp(ops, context, release), [ops, context, release]);

  // Primitives, not the object: the screen above rebuilds its row objects on
  // every list refresh, and an effect keyed on identity would re-fetch both
  // manifests each time the table polled.
  const name = release?.name ?? "";
  const namespace = release?.namespace ?? "";
  const revision = release?.revision ?? 0;
  const held = op !== null;
  /** The revision the diff reads from, when there is an earlier one at all. */
  const previous = revision > 1 ? revision - 1 : null;

  const [diff, setDiff] = useState<DiffLoad>({ status: "idle" });

  useEffect(() => {
    // Nothing is fetched while an operation holds the pane: the round trip
    // would be spent on something the reader cannot see, and the effect runs
    // again the moment the operation settles or is dismissed.
    if (name === "" || held || previous === null) {
      setDiff({ status: "idle" });
      return;
    }
    let live = true;
    setDiff({ status: "loading" });
    void (async () => {
      const [before, after] = await Promise.all([
        getHelmRelease(context, namespace, name, invoke, previous),
        getHelmRelease(context, namespace, name, invoke, revision),
      ]);
      if (!live) return;
      const failure = before.error ?? after.error;
      if (failure !== undefined) {
        setDiff({ status: "error", error: failure });
        return;
      }
      if (!before.release || !after.release) {
        // Name the side that actually came back empty. Blaming the left
        // revision for the right one's absence sends the reader digging
        // through history for a release that was uninstalled between the list
        // refresh and this fetch. Both sides missing names the earlier one,
        // which is the first thing to go looking for either way.
        const missing = before.release ? revision : previous;
        setDiff({ status: "error", error: `helm returned no revision ${missing} of ${name}` });
        return;
      }
      setDiff({
        status: "ready",
        left: previous,
        right: revision,
        rows: diffTextLines(before.release.manifest, after.release.manifest),
      });
    })();
    return () => {
      live = false;
    };
  }, [context, namespace, name, revision, previous, held, invoke]);

  const verdict = release ? helmStatus(release.status) : null;
  const headText = !release
    ? "Release"
    : op
      ? `${release.name} · ${op.kind} · output`
      : previous === null
        ? // No third segment. §16's is `· rendered diff`, and there is no diff
          // here to name — writing it anyway would promise the thing the body
          // is about to say does not exist.
          `${release.name} · revision ${revision}`
        : `${release.name} · ${previous} → ${revision} · rendered diff`;

  /** §16's footer is about the diff; a rollback target it does not have is not offered. */
  const footer =
    !op && previous !== null && (onRollback || onValuesEditor) ? (
      <div
        data-slot="pane-footer"
        className="flex min-w-0 shrink-0 items-center gap-2 border-t border-rule bg-sunk px-2.5 py-1.5"
      >
        {onRollback && (
          <Button variant="danger" size="sm" onClick={() => onRollback(previous)}>
            Roll back to {previous}
          </Button>
        )}
        {onValuesEditor && (
          <Button variant="secondary" size="sm" onClick={onValuesEditor}>
            Values editor
          </Button>
        )}
      </div>
    ) : null;

  return (
    // `min-w-0` on every box below that holds a diff line or a printed one:
    // this column is 420px and the text in it is unbounded, and a flex child's
    // `min-width: auto` floor is what turns a long line into a sideways scroll
    // of the whole window rather than a wrap inside the pane.
    <aside aria-labelledby={headId} className="side-rail" style={{ width: WIDTH }}>
      <div id={headId} className="pane-head">
        <span className="min-w-0 flex-1 truncate">{headText}</span>
        {op?.state === "running" && (
          <LiveSignal label={`${op.kind} running`} tone="info" className="shrink-0" />
        )}
        {verdict && (
          <span className="shrink-0">
            <Badge tone={statusTone(verdict.health)}>{verdict.word}</Badge>
          </span>
        )}
      </div>

      <div data-slot="pane-body" className="side-rail-body min-w-0">
        {!release ? (
          <EmptyState
            title="No release selected"
            hint="Pick a release to see what its last revision changed."
            compact
          />
        ) : op ? (
          <Operation op={op} onDismiss={onDismiss} />
        ) : previous === null ? (
          <EmptyState
            title="Nothing to compare"
            hint={`${release.name} is at revision ${revision}, its first — there is no earlier revision to diff it against.`}
            compact
          />
        ) : diff.status === "loading" || diff.status === "idle" ? (
          <LoadingState label={`Rendering ${release.name}`} />
        ) : diff.status === "error" ? (
          <FailureState
            title={`Could not diff ${release.name}`}
            error={diff.error}
            className="px-3"
          />
        ) : !diff.rows.some((r) => r.tag !== "same") ? (
          /* Not `rows.length === 0`. Two identical manifests do not produce an
             empty list — they produce a full one of `same` rows, and printing
             it renders the whole manifest as context, which reads as "here is
             what is deployed" rather than "nothing changed". The claim is
             about whether anything DIFFERS, so that is what is asked. */
          <EmptyState
            title="No changes"
            hint={`Revisions ${diff.left} and ${diff.right} of ${release.name} render the same manifest.`}
            compact
          />
        ) : (
          <DiffLines rows={diff.rows} className="min-w-0 py-1" />
        )}
      </div>

      {footer}
    </aside>
  );
}

/**
 * What an operation is saying — its output, and, when it failed, why.
 *
 * The failure is a banner over the output rather than in place of it: the
 * lines helm printed before it gave up are usually the only description of
 * what it was doing, and #349's vanishing port-forward is what happens when a
 * dead thing is quietly cleared away instead.
 */
function Operation({ op, onDismiss }: { op: HelmOpRow; onDismiss: (id: number) => void }) {
  return (
    <div className="flex min-w-0 flex-col gap-2 py-2">
      {op.state === "failed" && (
        <div className="min-w-0 break-words px-2.5">
          {/* `op.error` verbatim. `helmOps` described it already; describing a
              described sentence classifies it on its own wording. */}
          <Alert tone="sev" title={`${op.kind} of ${op.release} failed`}>
            {op.error ?? "helm gave no reason."}
          </Alert>
          <Button
            variant="secondary"
            size="sm"
            className="mt-2"
            onClick={() => onDismiss(op.id)}
          >
            Dismiss
          </Button>
        </div>
      )}
      {op.output.length === 0 ? (
        <div className="px-2.5 text-[0.75rem] text-muted">
          {op.state === "running"
            ? "helm has not printed anything yet."
            : "helm printed nothing before it stopped."}
        </div>
      ) : (
        <div className="flex min-w-0 flex-col font-mono text-[0.6875rem] leading-relaxed">
          {op.output.map((line, i) => (
            <div
              // The store only ever appends, so a line's position is stable for
              // the life of the row.
              key={i}
              data-slot="op-line"
              className="min-w-0 whitespace-pre-wrap break-all px-2.5"
              style={{ color: "var(--ink-soft)" }}
            >
              {line}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
