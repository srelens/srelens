import { useEffect, useMemo, useRef, useState } from "react";
import {
  actOnExtensionResource,
  inspectExtensionResource,
  renderConfirmTemplate,
  runBulk,
  type BulkProgress,
} from "@srelens/core";
import { Button } from "@srelens/ui-kit";
import { HostConfirmation, boundedPlainText } from "../confirm/HostConfirmation";
import { useConfirmationApp } from "../confirm/confirmationApp";
import { confirmFields } from "../confirm/confirmRequest";
import { useResource } from "../lib/useResource";
import { ACTION_LABELS, isKnownAction } from "./actionLabels";
import { plainText } from "./displayText";
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

/**
 * Whether one action can run against one resource — **the seam for #550**.
 *
 * Declarative `availableWhen` preconditions are #550's, being implemented
 * alongside this. Nothing here evaluates a predicate: the default is "every
 * selected resource is available", because a host with no predicate to run has
 * no grounds to say an action does not apply. When #550 lands it supplies this
 * function and `applies to 9 of 12` starts telling the truth about a mixed
 * selection without anything else here changing.
 */
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
  error: "Rejected",
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
  const first = selection[0];
  const firstKey = first ? bulkResourceKey(first) : "";
  /**
   * The host's menu for this KIND, read once from the first selected resource.
   *
   * `actions` and `actionMeta` are facts about the capability and the action,
   * not about the row — the same list and the same level the detail pane shows
   * for any resource of this binding. Reading one is what lets the bar offer
   * only what the host would actually run, instead of a button per action this
   * build happens to have a label for.
   */
  const menu = useResource(
    async () => (first ? await inspectExtensionResource({ ...target, namespace: first.namespace, name: first.name }) : null),
    [target.id, target.revision, target.capability, target.context, firstKey],
    () => false,
  );
  const [pending, setPending] = useState<string | null>(null);
  const [progress, setProgress] = useState<readonly BulkProgress<BulkResource>[] | null>(null);
  const [result, setResult] = useState<BulkActionResult | null>(null);
  const [busy, setBusy] = useState(false);
  const abort = useRef<AbortController | null>(null);
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);

  const offered = (menu.data?.actions ?? []).filter(isKnownAction);
  const applicability = useMemo(
    () => bulkApplicability(selection, (resource) => (pending && available ? available(pending, resource) : true)),
    [selection, pending, available],
  );

  const label = pending ? ACTION_LABELS[pending] : "";
  const meta = pending ? menu.data?.actionMeta?.[pending] : undefined;
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
        confirmFields({ action: pending ?? undefined, cluster: target.context, kind: menu.data?.resource?.kind }),
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
      return { error: e instanceof Error ? e.message : String(e) };
    }
  };

  const start = async (action: string) => {
    const items = applicability.applicable;
    if (items.length === 0) return;
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
              onClick={() => {
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
            <strong>Could not read the actions for this kind</strong>
            <p>{menu.error}</p>
          </div>
          <Button variant="outline" size="xs" onClick={menu.reload}>
            Retry
          </Button>
        </div>
      )}
      {pending !== null && (
        <div className="extension-action-review" role="dialog" aria-label={`Review ${label} on ${resources(applicability.applicable.length)}`}>
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
                  disabled={applicability.applicable.length === 0}
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
        <div className="extension-bulk-run">
          <div className="extension-bulk-bar">
            <strong role="status">
              {progress.filter((item) => item.state === "ok" || item.state === "error").length.toLocaleString("en-US")} of{" "}
              {progress.length.toLocaleString("en-US")} requested
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
                  {item.error && <span className="extension-bulk-reason">{plainText(item.error)}</span>}
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
                ? `None of the ${resources(result.succeeded.length + result.failed.length + result.cancelled.length)} were accepted.`
                : `Partial: ${result.succeeded.length.toLocaleString("en-US")} accepted, ${result.failed.length.toLocaleString("en-US")} rejected, ${result.cancelled.length.toLocaleString("en-US")} not requested.`}
          </p>
          {result.failed.length > 0 && (
            <div className="extension-bulk-group" data-testid="bulk-failures">
              <h5>Rejected ({result.failed.length.toLocaleString("en-US")})</h5>
              <ul>
                {result.failed.map((failure) => (
                  <li key={failure.resource}>
                    <span className="extension-bulk-name">{drawResource(failure.resource)}</span>
                    <span className="extension-bulk-reason">{plainText(failure.reason)}</span>
                  </li>
                ))}
              </ul>
            </div>
          )}
          <NameList testId="bulk-cancelled" title="Not requested" items={result.cancelled} />
          <NameList testId="bulk-succeeded" title="Accepted" items={result.succeeded} />
          <div className="extension-bulk-bar">
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
