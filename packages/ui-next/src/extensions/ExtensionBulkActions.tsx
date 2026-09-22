import { useEffect, useMemo, useRef, useState } from "react";
import {
  actOnExtensionResource,
  inspectExtensionResource,
  renderConfirmTemplate,
  runBulk,
  unmetPredicate,
  type ExtensionResourceDetail,
  type BulkProgress,
} from "@srelens/core";
import { Button } from "@srelens/ui-kit";
import { HostConfirmation, boundedPlainText } from "../confirm/HostConfirmation";
import { useConfirmationApp } from "../confirm/confirmationApp";
import { confirmFields } from "../confirm/confirmRequest";
import { useResource } from "../lib/useResource";
import { ACTION_AVAILABILITY } from "./actionAvailability";
import { ACTION_LABELS, isKnownAction } from "./actionLabels";
import {
  bulkActionResult,
  bulkApplicability,
  bulkResourceKey,
  type BulkActionResult,
  type BulkResource,
} from "./bulkActions";

/**
 * Running one of an app's actions over a selection of its resources (#553).
 *
 * **One confirmation, not N.** The reader is asked once, through the one
 * host-owned confirmation (#552) — the same component, the same level badge
 * and the same cluster line an agent is shown for the same write — with the
 * selection named as a count and every affected resource listed under it.
 *
 * **One write per resource, not one for the batch.** Each item goes through
 * `actOnExtensionResource`, so each is a separate `extensions.action` call and
 * therefore a separate audit record (#555, recorded at the bridge in
 * `apps/desktop/src-tauri/src/bridge.rs`). A batch that audited as one line
 * would leave the trail unable to answer "was THIS resource changed".
 *
 * **Cancelling stops the queue, never a write in flight.** The signal reaches
 * the scheduler and nothing else; see `BulkRunOptions.signal` in
 * `@srelens/core`. A resource the cluster has already accepted is reported
 * accepted, and the ones never sent are reported as not requested — not as
 * failures, because nobody asked the cluster about them.
 *
 * **A partial run is never drawn as a success.** That is the whole point of
 * the issue: nine of twelve reconciles is nine of twelve, with the other three
 * named and the cluster's reason beside each.
 */

/** At most this many writes are in flight at once. */
export const BULK_CONCURRENCY = 4;

/** The app, binding and cluster a selection was made in. */
export interface ExtensionBulkTarget {
  id: string;
  revision: number;
  capability: string;
  context: string;
}

/** Optional additional availability supplied by the table. */
export type BulkActionAvailability = (action: string, resource: BulkResource) => boolean;

export interface ExtensionBulkActionsProps {
  target: ExtensionBulkTarget;
  selection: readonly BulkResource[];
  /** Empties the table's selection once the reader is done with the result. */
  onClear(): void;
  available?: BulkActionAvailability;
}

const count = (n: number, one: string, many: string) => `${n.toLocaleString("en-US")} ${n === 1 ? one : many}`;
const resources = (n: number) => count(n, "resource", "resources");

/** What the state of one item is called in front of a reader. Never a colour alone. */
const STATE_LABEL: Record<BulkProgress<BulkResource>["state"], string> = {
  pending: "Pending",
  running: "Requesting…",
  ok: "Accepted",
  error: "Failed",
  cancelled: "Not requested",
};

/** A resource name, escaped and bounded exactly as the confirmation bounds one. */
const drawResource = (resource: string) => boundedPlainText(resource);

function NameList({ testId, title, items }: { testId: string; title: string; items: readonly string[] }) {
  if (items.length === 0) return null;
  return (
    <div className="extension-bulk-group" data-testid={testId}>
      <h5>
        {title} ({items.length.toLocaleString("en-US")})
      </h5>
      <ul>
        {items.map((resource) => (
          <li key={resource}>
            <span className="extension-bulk-name">{drawResource(resource)}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}

export function ExtensionBulkActions({ target, selection, onClear, available }: ExtensionBulkActionsProps) {
  // Who the host says asked: read from its own installed inventory, never from
  // the app. See `confirmationApp.ts`.
  const app = useConfirmationApp({ id: target.id, revision: target.revision });
  const selectionKey = JSON.stringify(selection);
  const inspectionAbort = useRef<AbortController | null>(null);
  // Printer columns do not contain the fields predicates address. Inspect the
  // selection before offering actions, with the same bounded scheduler as writes.
  // Any failed read blocks review and exposes retry; unread is not unavailable.
  const menu = useResource(
    async () => {
      inspectionAbort.current?.abort();
      const controller = new AbortController();
      inspectionAbort.current = controller;
      const details = new Map<string, ExtensionResourceDetail>();
      const outcomes = await runBulk(selection, async (resource) => {
        const detail = await inspectExtensionResource({ ...target, ...resource });
        details.set(bulkResourceKey(resource), detail);
        return { ok: true };
      }, BULK_CONCURRENCY, { signal: controller.signal });
      const failed = outcomes.find((item) => item.status === "error");
      if (failed) throw new Error(`${bulkResourceKey(failed.item)}: ${failed.error || "The resource could not be read."}`);
      return { first: selection[0] ? details.get(bulkResourceKey(selection[0])) : undefined, details };
    },
    [target.id, target.revision, target.capability, target.context, selectionKey],
    () => false,
  );
  const [pending, setPending] = useState<string | null>(null);
  const [progress, setProgress] = useState<readonly BulkProgress<BulkResource>[] | null>(null);
  const [result, setResult] = useState<BulkActionResult | null>(null);
  const [busy, setBusy] = useState(false);
  const abort = useRef<AbortController | null>(null);
  const alive = useRef(true);
  const review = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement | null>(null);
  const cancelRun = useRef<HTMLDivElement>(null);
  const done = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (pending) review.current?.focus();
    else if (busy) cancelRun.current?.querySelector("button")?.focus();
    else if (result) done.current?.querySelector("button")?.focus();
    else if (trigger.current?.isConnected) trigger.current.focus();
  }, [pending, busy, result]);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      abort.current?.abort();
      inspectionAbort.current?.abort();
    };
  }, []);

  const offered = (menu.data?.first?.actions ?? []).filter(isKnownAction);
  const applicability = useMemo(
    () => bulkApplicability(selection, (resource) => {
      if (!pending) return true;
      if (available) return available(pending, resource);
      const detail = menu.data?.details.get(bulkResourceKey(resource));
      return !!detail && !!detail.actions?.includes(pending) && !unmetPredicate(ACTION_AVAILABILITY[pending], detail.resource);
    }),
    [selection, pending, available, menu.data],
  );

  const label = pending ? ACTION_LABELS[pending] : "";
  const meta = pending ? menu.data?.first?.actionMeta?.[pending] : undefined;
  /**
   * The host's sentence, where the host's template can be rendered without an
   * object.
   *
   * A confirmation template is written for one resource, so one naming
   * `{name}` outside an optional segment renders to `null` here — and `null`
   * draws no sentence rather than one with a hole in it (#552). The level, the
   * cluster, the count and the list of resources carry the question until a
   * bulk template exists; nothing is composed locally to fill the gap.
   */
  const question = meta
    ? renderConfirmTemplate(
        meta.confirm,
        confirmFields({ action: pending ?? undefined, cluster: target.context, kind: menu.data?.first?.resource?.kind }),
      )
    : null;

  const perform = (action: string) => async (resource: BulkResource) => {
    const ref = { ...target, namespace: resource.namespace, name: resource.name };
    try {
      // Pinned per resource, immediately before its own write, the same UID and
      // resourceVersion the host's stale-review guard checks.
      const detail = await inspectExtensionResource(ref);
      const { uid, resourceVersion } = detail.resource.metadata;
      if (!uid || !resourceVersion) return { error: "The host returned no version to pin the write to." };
      if (!detail.actions?.includes(action))
        return { error: `The host does not offer ${ACTION_LABELS[action] ?? action} on this resource.` };
      const acted = await actOnExtensionResource(ref, action, uid, resourceVersion);
      return acted.requested
        ? { ok: true }
        : { error: "The action was not acknowledged; refresh to check the resource." };
    } catch (e) {
      return { error: (e instanceof Error ? e.message : String(e)).trim() || "The operation failed without a reason." };
    }
  };

  const start = async (action: string) => {
    const items = applicability.applicable;
    if (items.length === 0 || menu.status !== "ready") return;
    const controller = new AbortController();
    abort.current = controller;
    setPending(null);
    setBusy(true);
    setResult(null);
    setProgress(items.map((item) => ({ item, state: "pending" as const })));
    const outcomes = await runBulk(items, perform(action), BULK_CONCURRENCY, {
      signal: controller.signal,
      onProgress: (items) => {
        if (alive.current) setProgress(items);
      },
    });
    if (!alive.current) return;
    abort.current = null;
    setResult(bulkActionResult(outcomes));
    setBusy(false);
  };

  if (selection.length === 0) return null;

  return (
    <div className="extension-bulk">
      <div className="extension-bulk-bar">
        <strong data-testid="bulk-count">{count(selection.length, "selected", "selected")}</strong>
        {menu.status === "loading" && <span className="extension-message">Loading actions…</span>}
        {menu.status !== "error" &&
          offered.map((action) => (
            <Button
              key={action}
              variant="outline"
              size="xs"
              disabled={busy || pending !== null}
              onClick={(event) => {
                trigger.current = event.currentTarget;
                setResult(null);
                setProgress(null);
                setPending(action);
              }}
            >
              {ACTION_LABELS[action]}
            </Button>
          ))}
        <Button variant="outline" size="xs" disabled={busy} onClick={onClear}>
          Clear selection
        </Button>
      </div>
      {menu.status === "error" && (
        // The host could not say which actions it offers. That is a failed
        // read, not an absence of actions, so it says so and offers a retry
        // rather than drawing an empty bar.
        <div className="extension-error" role="alert">
          <div>
            <strong>Could not read actions and availability for the selection</strong>
            <p>{boundedPlainText(menu.error ?? "The resource could not be read.")}</p>
          </div>
          <Button variant="outline" size="xs" onClick={menu.reload}>
            Retry
          </Button>
        </div>
      )}
      {pending !== null && (
        <div ref={review} tabIndex={-1} onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.preventDefault();
            event.stopPropagation();
            setPending(null);
          }
        }} className="extension-action-review" role="dialog" aria-label={`Review ${label} on ${resources(applicability.applicable.length)}`}>
          {/* The one host confirmation (#552), in this screen's own frame. The
              selection is named by its count, which is the only part of the
              question a bulk execution changes. */}
          <HostConfirmation
            question={question}
            impact={meta?.impact ?? null}
            cluster={target.context}
            subject={{ kind: "bulk", count: applicability.applicable.length }}
            app={app}
            details={
              <>
                {applicability.note && (
                  <p className="extension-bulk-applies" data-testid="bulk-applies">
                    {applicability.note}
                  </p>
                )}
                <ul className="extension-bulk-list" data-testid="bulk-resources">
                  {applicability.applicable.map((resource) => (
                    <li key={bulkResourceKey(resource)}>
                      <span className="extension-bulk-name">{drawResource(bulkResourceKey(resource))}</span>
                    </li>
                  ))}
                </ul>
              </>
            }
            actions={
              <>
                <Button
                  disabled={applicability.applicable.length === 0 || menu.status !== "ready"}
                  onClick={() => void start(pending)}
                >
                  {`${label} ${resources(applicability.applicable.length)}`}
                </Button>
                <Button variant="outline" onClick={() => setPending(null)}>
                  Cancel
                </Button>
              </>
            }
          />
        </div>
      )}
      {progress && !result && (
        <div className="extension-bulk-run" ref={cancelRun}>
          <div className="extension-bulk-bar">
            <strong role="status">
              {progress.filter((item) => item.state !== "pending" && item.state !== "running").length.toLocaleString("en-US")} of{" "}
              {progress.length.toLocaleString("en-US")} completed
            </strong>
            <Button
              variant="outline"
              size="xs"
              disabled={abort.current === null}
              onClick={() => abort.current?.abort()}
            >
              Cancel remaining
            </Button>
          </div>
          <ul className="extension-bulk-list" data-testid="bulk-progress">
            {progress.map((item) => {
              const key = bulkResourceKey(item.item);
              return (
                <li key={key} data-testid={`bulk-item-${key}`} data-state={item.state}>
                  <span className="extension-bulk-name">{drawResource(key)}</span>
                  <span className="extension-bulk-state">{STATE_LABEL[item.state]}</span>
                  {item.error && <span className="extension-bulk-reason">{boundedPlainText(item.error)}</span>}
                </li>
              );
            })}
          </ul>
        </div>
      )}
      {result && (
        <div
          className="extension-bulk-result"
          data-testid="bulk-result"
          data-status={result.status}
          role={result.status === "success" ? "status" : "alert"}
        >
          <p className="extension-bulk-headline">
            {result.status === "success"
              ? `All ${resources(result.succeeded.length)} were accepted.`
              : result.status === "failed"
                ? `No acceptance was confirmed for the ${resources(result.succeeded.length + result.failed.length + result.cancelled.length)}.`
                : `Partial: ${result.succeeded.length.toLocaleString("en-US")} accepted, ${result.failed.length.toLocaleString("en-US")} failed, ${result.cancelled.length.toLocaleString("en-US")} not requested.`}
          </p>
          {result.failed.length > 0 && (
            <div className="extension-bulk-group" data-testid="bulk-failures">
              <h5>Failed ({result.failed.length.toLocaleString("en-US")})</h5>
              <ul>
                {result.failed.map((failure) => (
                  <li key={failure.resource}>
                    <span className="extension-bulk-name">{drawResource(failure.resource)}</span>
                    <span className="extension-bulk-reason">{boundedPlainText(failure.reason)}</span>
                  </li>
                ))}
              </ul>
            </div>
          )}
          <NameList testId="bulk-cancelled" title="Not requested" items={result.cancelled} />
          <NameList testId="bulk-succeeded" title="Accepted" items={result.succeeded} />
          <div className="extension-bulk-bar" ref={done}>
            <Button
              variant="outline"
              size="xs"
              onClick={() => {
                setResult(null);
                setProgress(null);
                onClear();
              }}
            >
              Done
            </Button>
          </div>
        </div>
      )}
    </div>
  );
}
