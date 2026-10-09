import type { ClusterContext } from "@srelens/core";
import { EmptyState, Eyebrow } from "@srelens/ui-kit";
import { useContexts } from "../../lib/clusters";
import { recentKey, useAllRecentLogSubjects, useOfferedRecents } from "../../lib/logRecents";
import { getMark } from "../../lib/marks";
import { focusCluster, openOnCluster } from "../../lib/openCluster";
import { reopenClosed, useTabs } from "../../lib/tabsStore";
import { logsRoute } from "../Logs";

/** How many recently closed tabs are offered — the most recent, not the whole stack. */
const CLOSED_SHOWN = 5;

const label = (context: ClusterContext) => getMark(context.stableId, context.name).name;

/**
 * "Pick up where you left off": recently closed tabs and recently followed
 * logs, each opened on the cluster it was about.
 *
 * A closed tab names its cluster by the context name on its label; one whose
 * cluster no longer exists cannot be opened on its own cluster, so it is left
 * out rather than reopened onto whichever cluster is in focus. An app-wide tab
 * (Settings) has no cluster and simply comes back.
 *
 * Followed logs are checked against their cluster before they are offered,
 * exactly as `/logs` checks them, and only on `targets` — the connected,
 * unpaused clusters — because checking is a read. For the same reason nothing
 * is checked while `paused`: Home sealed, behind another tab, or hidden.
 */
export function PickUp({ targets, paused = false }: { targets: readonly ClusterContext[]; paused?: boolean }) {
  const { closed } = useTabs();
  const contexts = useContexts();
  const recents = useAllRecentLogSubjects();
  const tabs = closed
    // One row per route: the stack is most recent first, so the first is the one to bring back.
    .filter((tab, at) => closed.findIndex((t) => t.route === tab.route) === at)
    .map((tab) => ({ tab, context: tab.sub ? contexts.find((c) => c.name === tab.sub) : undefined }))
    .filter(({ tab, context }) => !tab.sub || context)
    .slice(0, CLOSED_SHOWN);
  const followed = targets.filter((t) => recents.some((r) => r.cluster === t.stableId));

  return (
    <section className="home-section" aria-labelledby="home-pickup-title">
      <div className="home-section-heading">
        <h2 id="home-pickup-title">Pick up where you left off</h2>
      </div>
      {tabs.length === 0 && followed.length === 0 ? (
        <EmptyState compact title="Nothing to pick up yet" hint="Tabs you close and logs you follow on a connected cluster show here." />
      ) : (
        <div className="home-pickup">
          {tabs.length > 0 && (
            <div>
              <Eyebrow>Recently closed</Eyebrow>
              <ul className="home-pick-list">
                {tabs.map(({ tab, context }) => (
                  <li key={tab.id}>
                    <button
                      type="button"
                      className="home-pick-row"
                      aria-label={context ? `Reopen ${tab.title} on ${label(context)}` : `Reopen ${tab.title}`}
                      onClick={() => {
                        if (context) focusCluster(context);
                        reopenClosed(tab.id);
                      }}
                    >
                      <span className="min-w-0 truncate font-medium">{tab.title}</span>
                      {context && <span className="shrink-0 truncate text-xs text-muted">{label(context)}</span>}
                    </button>
                  </li>
                ))}
              </ul>
            </div>
          )}
          {followed.length > 0 && (
            <div>
              <Eyebrow>Recently followed logs</Eyebrow>
              <ul className="home-pick-list">
                {followed.map((context) => <FollowedLogs key={context.stableId} context={context} paused={paused} />)}
              </ul>
            </div>
          )}
        </div>
      )}
    </section>
  );
}

/**
 * One cluster's checked log subjects, as rows of the list above. This cluster
 * has subjects stored, or it would not be here; until the check offers any of
 * them — still out, or held while Home is out of view — it says so, rather
 * than leaving the heading over nothing.
 */
function FollowedLogs({ context, paused }: { context: ClusterContext; paused: boolean }) {
  const offered = useOfferedRecents(context.name, context.stableId, paused);
  const where = label(context);
  if (offered.length === 0) {
    return (
      <li className="home-pick-pending text-xs text-muted">
        {paused ? `Will check what you followed on ${where}` : `Checking what you followed on ${where}…`}
      </li>
    );
  }
  return (
    <>
      {offered.map(({ entry, presence }) => (
        <li key={recentKey(entry)}>
          <button
            type="button"
            className="home-pick-row"
            aria-label={`Follow logs of ${entry.kind} ${entry.namespace}/${entry.name} on ${where}`}
            // Shown, not hidden, and not a way in: what was followed is not there just now.
            disabled={presence === "gone"}
            onClick={() => openOnCluster(context, logsRoute(entry.kind, entry.namespace, entry.name))}
          >
            <span className="min-w-0 truncate font-medium">{`${entry.kind}/${entry.name}`}</span>
            <span className="shrink-0 truncate text-xs text-muted">
              {presence === "gone" ? "no longer on this cluster" : `${entry.namespace} · ${where}`}
            </span>
          </button>
        </li>
      ))}
    </>
  );
}
