import { useState } from "react";
import { isTauri, plural, type ClusterContext } from "@srelens/core";
import { Alert, Button, EmptyState, LoadingState, RawError, StatusPill } from "@srelens/ui-kit";
import { attentionPath, attentionQuestion, rankedAttention, type ClusterAttention } from "../../lib/attention";
import { detailRoute } from "../../lib/detailRoute";
import { summarise } from "../../lib/errorCopy";
import { getMark } from "../../lib/marks";
import { openOnCluster } from "../../lib/openCluster";
import { useAskAssistant } from "./ask";

/** How many rows show before the reader asks for the rest — a strip, not a second resource list. */
const SHOWN = 10;

export interface NeedsAttentionProps {
  /** The clusters that were read: this workspace's connected, unpaused ones. */
  targets: readonly ClusterContext[];
  /** Each target's latest answer; a target with no entry has not answered yet. */
  scans: Readonly<Record<string, ClusterAttention>>;
}

/**
 * What is wrong right now, across the clusters Home could read.
 *
 * Three things never read as good news: a cluster that has not answered yet
 * ("Still checking"), a cluster whose read refused (said by name, with the
 * reason), and a pod list the backend capped. "Nothing needs attention" is
 * drawn only when every target answered every read in full.
 *
 * Every row opens its object on ITS cluster — the detail route carries no
 * cluster, so the cluster has to be the one in focus — and on the desktop
 * offers Ask, which fills the assistant's prompt with a question about it.
 * The web host has no assistant to ask, so it offers none.
 */
export function NeedsAttention({ targets, scans }: NeedsAttentionProps) {
  const desktop = isTauri();
  const ask = useAskAssistant();
  const [all, setAll] = useState(false);
  const label = (t: ClusterContext) => getMark(t.stableId, t.name).name;
  const items = rankedAttention(targets, scans);
  const pending = targets.filter((t) => !scans[t.stableId]).length;
  const failed = targets.filter((t) => (scans[t.stableId]?.failures.length ?? 0) > 0);
  const short = targets.filter((t) => scans[t.stableId]?.truncated);
  const eventsShort = targets.filter((t) => scans[t.stableId]?.eventsTruncated);
  const shown = all ? items : items.slice(0, SHOWN);

  return (
    <section className="home-section" aria-labelledby="home-attention-title">
      <div className="home-section-heading">
        <h2 id="home-attention-title">
          Needs attention {items.length > 0 && <span className="text-muted">{items.length}</span>}
        </h2>
      </div>
      {targets.length === 0 ? (
        <EmptyState
          compact
          title="No connected clusters to check"
          hint="This reads the clusters in this workspace that are connected and not paused. Open a cluster to connect it."
        />
      ) : pending === targets.length ? (
        <LoadingState label={`Checking ${plural(targets.length, "cluster")}…`} />
      ) : (
        <>
          {failed.map((t) => {
            const { detail, raw } = summarise(scans[t.stableId].failures);
            return (
              <Alert key={t.stableId} tone="warn" title={`Could not check everything on ${label(t)}`} className="home-section-alert">
                {detail}
                <RawError text={raw ?? ""} className="mt-1" />
              </Alert>
            );
          })}
          {short.map((t) => (
            <Alert key={t.stableId} tone="warn" title={`More pods on ${label(t)} may need a look than this shows`} className="home-section-alert">
              The list stops before the whole set. The cluster&rsquo;s pod list has all of them.
            </Alert>
          ))}
          {eventsShort.map((t) => (
            <Alert key={t.stableId} tone="warn" title={`Not every event on ${label(t)} could be read`} className="home-section-alert">
              The event list stopped at its limit, so recent warnings may be missing here. The cluster&rsquo;s Events screen has them all.
            </Alert>
          ))}
          {pending > 0 && <p className="home-note">Still checking {plural(pending, "cluster")}</p>}
          {items.length > 0 ? (
            <ul className="home-attention-list" aria-label="Things that need attention">
              {shown.map(({ item, context }) => {
                const where = label(context);
                const path = attentionPath(item);
                return (
                  <li key={`${item.clusterId}/${item.kind}/${item.namespace}/${item.name}`} className="home-attention-row">
                    <button
                      type="button"
                      className="home-attention-open"
                      aria-label={`Open ${item.kind} ${path} on ${where}`}
                      onClick={() => openOnCluster(context, detailRoute(item.kind, item.namespace || null, item.name))}
                    >
                      <StatusPill status={item.problem} kind={item.cause === "crash" || item.cause === "image" ? "danger" : "warning"} tinted />
                      <span className="min-w-0 flex-1 truncate font-medium" title={path}>{path}</span>
                      <span className="shrink-0 truncate text-xs text-muted">{item.kind} · {where}</span>
                    </button>
                    {desktop && (
                      <Button variant="secondary" size="sm" aria-label={`Ask about ${path} on ${where}`} onClick={() => ask(attentionQuestion(item), context)}>
                        Ask
                      </Button>
                    )}
                  </li>
                );
              })}
            </ul>
          ) : (
            pending === 0 && failed.length === 0 && short.length === 0 && eventsShort.length === 0 && (
              <EmptyState
                compact
                title="Nothing needs attention"
                hint={`Checked ${plural(targets.length, "connected cluster")}: no crash-looping pods or image pulls failing, no unavailable replicas, no warnings in the last hour.`}
              />
            )
          )}
          {!all && items.length > SHOWN && (
            <div className="home-section-foot">
              <Button variant="secondary" size="sm" onClick={() => setAll(true)}>Show all {items.length}</Button>
            </div>
          )}
        </>
      )}
    </section>
  );
}
