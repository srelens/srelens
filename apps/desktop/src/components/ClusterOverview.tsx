import React, { useEffect, useRef, useState } from "react";
import { RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { listNamespaces, listPods } from "../lib/workloads";
import { listNodes, listResource, listEvents } from "../lib/manifest";
import {
  DashboardSegmentBar,
  EmptyState,
  ErrorState,
  MetricTile,
  PageHeader,
  PageShell,
  SectionPanel,
  Spinner,
  StatusMeter,
} from "../ui";
import { describeError, isExecAuthError } from "../lib/errors";
import { isTauri } from "../transport/platform";
import type { ResourceKind } from "./ResourceBrowser";

interface Stats {
  nodes: { total: number; ready: number };
  pods: { total: number; running: number; pending: number; other: number };
  deployments: number;
  services: number;
  namespaces: number;
  events: { total: number; normal: number; warnings: number; recentWarnings: string[] };
}

interface OverviewSnapshot {
  stats: Stats;
  updatedAt: number;
}

const CACHE_TTL_MS = 30_000;
const overviewCache = new Map<string, OverviewSnapshot>();
const overviewRequests = new Map<string, Promise<OverviewSnapshot>>();

/** Clear cached overview snapshots. Exported for deterministic tests and future logout/reset flows. */
export function clearClusterOverviewCache(context?: string) {
  if (context) {
    overviewCache.delete(context);
    overviewRequests.delete(context);
    return;
  }
  overviewCache.clear();
  overviewRequests.clear();
}

function formatUpdatedAt(updatedAt: number) {
  return new Intl.DateTimeFormat(undefined, { timeStyle: "medium" }).format(new Date(updatedAt));
}

async function fetchOverview(context: string): Promise<OverviewSnapshot> {
  const [nodes, pods, deps, svcs, ns, events] = await Promise.all([
    listNodes(context),
    listPods(context, ""),
    listResource(context, "Deployment", ""),
    listResource(context, "Service", ""),
    listNamespaces(context),
    listEvents(context, null),
  ]);
  const firstError = nodes.error || pods.error || deps.error || svcs.error || ns.error || events.error;
  if (firstError) throw new Error(firstError);

  const podList = pods.pods ?? [];
  const eventList = events.events ?? [];
  const warningEvents = eventList.filter((event) => event.type === "Warning");
  return {
    stats: {
      nodes: {
        total: (nodes.nodes ?? []).length,
        ready: (nodes.nodes ?? []).filter((node) => node.status === "Ready").length,
      },
      pods: {
        total: podList.length,
        running: podList.filter((pod) => pod.phase === "Running").length,
        pending: podList.filter((pod) => pod.phase === "Pending").length,
        other: podList.filter((pod) => pod.phase !== "Running" && pod.phase !== "Pending").length,
      },
      deployments: (deps.items ?? []).length,
      services: (svcs.items ?? []).length,
      namespaces: (ns.namespaces ?? []).length,
      events: {
        total: eventList.length,
        normal: eventList.filter((event) => event.type !== "Warning").length,
        warnings: warningEvents.length,
        recentWarnings: warningEvents.slice(0, 2).map((event) => `${event.reason}: ${event.message}`),
      },
    },
    updatedAt: Date.now(),
  };
}

function requestOverview(context: string, force: boolean): Promise<OverviewSnapshot> {
  const cached = overviewCache.get(context);
  if (!force && cached && Date.now() - cached.updatedAt < CACHE_TTL_MS) {
    return Promise.resolve(cached);
  }

  const pending = overviewRequests.get(context);
  if (pending) return pending;

  const request = fetchOverview(context)
    .then((snapshot) => {
      overviewCache.set(context, snapshot);
      return snapshot;
    })
    .finally(() => {
      overviewRequests.delete(context);
    });
  overviewRequests.set(context, request);
  return request;
}

function percent(part: number, total: number) {
  if (total <= 0) return 0;
  return (part / total) * 100;
}

/** Cluster overview dashboard: at-a-glance counts and health. */
export function ClusterOverview({
  context,
  onOpenView,
  onDiagnose,
}: {
  context: string;
  onOpenView?: (kind: ResourceKind) => void;
  /** Open the Toolbox to diagnose this context (shown on exec-auth errors). */
  onDiagnose?: () => void;
}) {
  const initialSnapshot = overviewCache.get(context);
  const [stats, setStats] = useState<Stats | null>(() => initialSnapshot?.stats ?? null);
  const [error, setError] = useState("");
  const [lastUpdated, setLastUpdated] = useState(() =>
    initialSnapshot ? formatUpdatedAt(initialSnapshot.updatedAt) : "",
  );
  const [refreshing, setRefreshing] = useState(false);
  const [reloadKey, setReloadKey] = useState(0);
  const forceRefresh = useRef(false);

  useEffect(() => {
    let active = true;
    const cached = overviewCache.get(context);
    const force = forceRefresh.current;
    forceRefresh.current = false;

    setStats(cached?.stats ?? null);
    setError("");
    setLastUpdated(cached ? formatUpdatedAt(cached.updatedAt) : "");

    const fresh = cached && Date.now() - cached.updatedAt < CACHE_TTL_MS;
    if (fresh && !force) {
      return () => {
        active = false;
      };
    }

    setRefreshing(true);
    void requestOverview(context, force)
      .then((snapshot) => {
        if (!active) return;
        setStats(snapshot.stats);
        setLastUpdated(formatUpdatedAt(snapshot.updatedAt));
      })
      .catch((cause: unknown) => {
        if (active) setError(cause instanceof Error ? cause.message : String(cause));
      })
      .finally(() => {
        if (active) setRefreshing(false);
      });
    return () => {
      active = false;
    };
  }, [context, reloadKey]);

  function refresh() {
    forceRefresh.current = true;
    setReloadKey((key) => key + 1);
  }

  if (error && !stats) {
    const friendly = describeError(error);
    return (
      <ErrorState
        title={friendly.title}
        detail={friendly.detail}
        onRetry={refresh}
        action={
          // The Toolbox can install a missing plugin on desktop, but tool
          // installs are disabled in the web container (and an exec plugin
          // can't run there regardless) — so only offer it on desktop.
          onDiagnose && isExecAuthError(error) && isTauri()
            ? { label: "Diagnose in Toolbox", onClick: onDiagnose }
            : undefined
        }
      />
    );
  }
  if (!stats) return <Spinner label="Loading overview" />;

  const nodeReadiness = percent(stats.nodes.ready, stats.nodes.total);
  const podHealth = percent(stats.pods.running, stats.pods.total);
  const eventHealth = percent(stats.events.normal, stats.events.total);

  return (
    <PageShell>
      <PageHeader
        eyebrow="Cluster overview"
        title={context}
        description={`${stats.nodes.ready}/${stats.nodes.total} ready nodes · ${stats.pods.running}/${stats.pods.total} running pods · updated ${lastUpdated || "now"}${error ? " · refresh failed" : ""}`}
        actions={
          <Button
            variant="outline"
            size="sm"
            onClick={refresh}
            disabled={refreshing}
            aria-label="Refresh cluster overview"
          >
            <RefreshCw data-icon="inline-start" className={refreshing ? "animate-spin" : undefined} />
            {refreshing ? "Refreshing" : "Refresh"}
          </Button>
        }
      />

      <div className="fl-metric-grid">
        <MetricTile label="Nodes" value={`${stats.nodes.ready} / ${stats.nodes.total}`} description="Ready / total" tone={nodeReadiness === 100 ? "success" : "warning"} />
        <MetricTile label="Pods" value={`${stats.pods.running} / ${stats.pods.total}`} description="Running / total" tone={stats.pods.other > 0 ? "warning" : "success"} />
        <MetricTile label="Workloads" value={stats.deployments} description="Deployments" tone="primary" />
        <MetricTile label="Services" value={stats.services} description="Cluster services" tone="info" />
        <MetricTile label="Namespaces" value={stats.namespaces} description="Active namespaces" />
        <MetricTile label="Warnings" value={stats.events.warnings} description="Recent events" tone={stats.events.warnings > 0 ? "warning" : "success"} />
      </div>

      <SectionPanel title="Cluster health" description="Readiness and recent event signal from Kubernetes API data.">
        <div className="fl-status-grid">
          <StatusMeter
            label="Node readiness"
            value={nodeReadiness}
            detail={`${stats.nodes.ready} ready / ${stats.nodes.total} total`}
            tone={nodeReadiness === 100 ? "success" : "warning"}
          />
          <StatusMeter
            label="Pod health"
            value={podHealth}
            detail={`${stats.pods.running} running / ${stats.pods.total} total`}
            tone={stats.pods.other > 0 ? "warning" : "primary"}
          />
          <StatusMeter
            label="Event health"
            value={eventHealth}
            detail={`${stats.events.normal} normal / ${stats.events.total} total`}
            tone={stats.events.warnings > 0 ? "warning" : "success"}
          />
        </div>
      </SectionPanel>

      <div className="fl-overview-grid">
        <SectionPanel title="Pod distribution">
          <DashboardSegmentBar
            segments={[
              { value: stats.pods.running, tone: "success", label: "Running" },
              { value: stats.pods.pending, tone: "warning", label: "Pending" },
              { value: stats.pods.other, tone: "danger", label: "Other" },
            ]}
          />
          <div className="fl-inline-actions">
            <button type="button" className="fl-text-action" onClick={() => onOpenView?.("pods")}>View pods</button>
            <button type="button" className="fl-text-action" onClick={() => onOpenView?.("nodes")}>View nodes</button>
          </div>
        </SectionPanel>

        <SectionPanel title="Recent warnings">
          {stats.events.recentWarnings.length > 0 ? (
            <ul className="fl-dashboard-events">
              {stats.events.recentWarnings.map((message) => (
                <li key={message}>{message}</li>
              ))}
            </ul>
          ) : (
            <EmptyState title="No warning events" description="The API did not return recent warning events." />
          )}
          <div className="fl-inline-actions">
            <button type="button" className="fl-text-action" onClick={() => onOpenView?.("events")}>View events</button>
          </div>
        </SectionPanel>
      </div>
    </PageShell>
  );
}
