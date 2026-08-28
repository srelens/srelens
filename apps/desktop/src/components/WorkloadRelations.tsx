import React, { useEffect, useState } from "react";
import {
  listReplicaSets,
  podsForSelector,
  podMetrics,
  type ReplicaSetSummary,
  type PodSummary,
} from "@srelens/core";
import { listJobs, type JobSummary } from "@srelens/core";
import { Spinner, StatusPill, Table, type StatusKind, type Column } from "../ui";
import type { OpenResource } from "@srelens/core";
import { ageSortValue } from "@srelens/core";

function phaseKind(phase: string): StatusKind {
  if (phase === "Running" || phase === "Succeeded") return "success";
  if (phase === "Pending") return "warning";
  if (phase === "Failed" || phase === "Unknown") return "danger";
  return "neutral";
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="fl-detail-section">
      <h4 className="fl-detail-section__title">{title}</h4>
      {children}
    </section>
  );
}

/** "Deploy Revisions": the ReplicaSets a Deployment has rolled out. */
export function DeployRevisions({
  context,
  namespace,
  ownerName,
  onOpenResource,
  listReplicaSetsFn = listReplicaSets,
}: {
  context: string;
  namespace: string;
  ownerName: string;
  onOpenResource?: OpenResource;
  listReplicaSetsFn?: typeof listReplicaSets;
}) {
  const [rows, setRows] = useState<ReplicaSetSummary[] | null>(null);
  const [error, setError] = useState("");

  useEffect(() => {
    let active = true;
    setRows(null);
    setError("");
    void listReplicaSetsFn(context, namespace, ownerName).then((out) => {
      if (!active) return;
      if (out.error) setError(out.error);
      else setRows(out.replicasets ?? []);
    });
    return () => {
      active = false;
    };
  }, [context, namespace, ownerName, listReplicaSetsFn]);

  if (error) return null; // a missing revisions list shouldn't break the panel
  if (rows === null)
    return (
      <Section title="Deploy Revisions">
        <Spinner label="Loading revisions" />
      </Section>
    );

  const columns: Column<ReplicaSetSummary>[] = [
    {
      key: "revision",
      header: "#",
      render: (r) => <span className="fl-mono">{r.revision || "—"}</span>,
    },
    { key: "name", header: "Name", render: (r) => <span className="fl-mono">{r.name}</span> },
    { key: "pods", header: "Pods", render: (r) => `${r.ready}/${r.desired}` },
    { key: "age", header: "Age", getSortValue: ageSortValue, render: (r) => r.age },
  ];

  return (
    <Section title="Deploy Revisions">
      <Table
        columns={columns}
        data={rows}
        getRowKey={(r) => r.name}
        onRowClick={
          onOpenResource
            ? (row) => onOpenResource({ kind: "ReplicaSet", namespace, name: row.name })
            : undefined
        }
        emptyText="No revisions"
      />
    </Section>
  );
}

/** "Recent Jobs": the Jobs a CronJob owns (its run history), newest-first. */
export function CronJobJobs({
  context,
  namespace,
  ownerName,
  onOpenResource,
  listJobsFn = listJobs,
}: {
  context: string;
  namespace: string;
  ownerName: string;
  onOpenResource?: OpenResource;
  listJobsFn?: typeof listJobs;
}) {
  const [rows, setRows] = useState<JobSummary[] | null>(null);
  const [error, setError] = useState("");

  useEffect(() => {
    let active = true;
    setRows(null);
    setError("");
    void listJobsFn(context, namespace).then((out) => {
      if (!active) return;
      if (out.error) setError(out.error);
      else setRows((out.jobs ?? []).filter((j) => j.owner === ownerName));
    });
    return () => {
      active = false;
    };
  }, [context, namespace, ownerName, listJobsFn]);

  if (error) return null; // a missing jobs list shouldn't break the panel
  if (rows === null)
    return (
      <Section title="Recent Jobs">
        <Spinner label="Loading jobs" />
      </Section>
    );

  const columns: Column<JobSummary>[] = [
    { key: "name", header: "Name", render: (j) => <span className="fl-mono">{j.name}</span> },
    { key: "completions", header: "Completions", render: (j) => j.completions },
    {
      key: "status",
      header: "Status",
      render: (j) => {
        const [status, kind]: [string, StatusKind] =
          j.failed > 0 ? ["Failed", "danger"] : j.active > 0 ? ["Active", "warning"] : ["Complete", "success"];
        return <StatusPill status={status} kind={kind} />;
      },
    },
    { key: "duration", header: "Duration", render: (j) => j.duration || "—" },
    { key: "age", header: "Age", getSortValue: ageSortValue, render: (j) => j.age },
  ];

  return (
    <Section title="Recent Jobs">
      <Table
        columns={columns}
        data={rows}
        getRowKey={(j) => j.name}
        onRowClick={
          onOpenResource ? (row) => onOpenResource({ kind: "Job", namespace, name: row.name }) : undefined
        }
        emptyText="No jobs yet"
      />
    </Section>
  );
}

type PodRow = PodSummary & { cpu?: number; memory?: number };

function RelationLink({
  label,
  ariaLabel,
  onClick,
}: {
  label: string;
  ariaLabel: string;
  onClick?: () => void;
}) {
  if (!onClick) return <span className="fl-mono">{label}</span>;
  return (
    <button
      type="button"
      className="fl-link fl-mono"
      aria-label={ariaLabel}
      onClick={(event) => {
        event.stopPropagation();
        onClick();
      }}
    >
      {label}
    </button>
  );
}

/** "Pods": the pods a workload manages, matched by its label selector. */
export function ManagedPods({
  context,
  namespace,
  selector,
  onOpenResource,
  podsForSelectorFn = podsForSelector,
  podMetricsFn = podMetrics,
}: {
  context: string;
  namespace: string;
  selector: Record<string, string>;
  onOpenResource?: OpenResource;
  podsForSelectorFn?: typeof podsForSelector;
  podMetricsFn?: typeof podMetrics;
}) {
  const [rows, setRows] = useState<PodRow[] | null>(null);
  const [error, setError] = useState("");
  const selectorKey = JSON.stringify(selector);

  useEffect(() => {
    let active = true;
    setRows(null);
    setError("");
    void Promise.all([
      podsForSelectorFn(context, namespace, selector),
      // Metrics are best-effort: a missing metrics-server must not hide pods.
      podMetricsFn(context, namespace).catch(() => ({ metrics: [] })),
    ]).then(([podsOut, metricsOut]) => {
      if (!active) return;
      if (podsOut.error) {
        setError(podsOut.error);
        return;
      }
      const usage = new Map((metricsOut.metrics ?? []).map((m) => [m.name, m]));
      setRows(
        (podsOut.pods ?? []).map((p) => {
          const m = usage.get(p.name);
          return { ...p, cpu: m?.cpuMillicores, memory: m?.memoryMiB };
        }),
      );
    });
    return () => {
      active = false;
    };
    // selectorKey captures selector identity without a new object each render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [context, namespace, selectorKey, podsForSelectorFn, podMetricsFn]);

  if (error) return null;
  if (rows === null)
    return (
      <Section title="Pods">
        <Spinner label="Loading pods" />
      </Section>
    );

  const columns: Column<PodRow>[] = [
    {
      key: "name",
      header: "Name",
      render: (pod) => (
        <RelationLink
          label={pod.name}
          ariaLabel={`Open Pod ${pod.name}`}
          onClick={
            onOpenResource
              ? () => onOpenResource({ kind: "Pod", namespace: pod.namespace || namespace, name: pod.name })
              : undefined
          }
        />
      ),
    },
    {
      key: "node",
      header: "Node",
      render: (pod) => (
        <RelationLink
          label={pod.node}
          ariaLabel={`Open Node ${pod.node}`}
          onClick={
            onOpenResource && pod.node
              ? () => onOpenResource({ kind: "Node", namespace: null, name: pod.node })
              : undefined
          }
        />
      ),
    },
    { key: "ready", header: "Ready", render: (p) => p.ready },
    {
      key: "cpu",
      header: "CPU",
      render: (p) => (p.cpu != null ? (p.cpu / 1000).toFixed(3) : "—"),
    },
    {
      key: "memory",
      header: "Memory",
      render: (p) => (p.memory != null ? `${p.memory} Mi` : "—"),
    },
    {
      key: "status",
      header: "Status",
      render: (p) => <StatusPill status={p.phase} kind={phaseKind(p.phase)} />,
    },
  ];

  return (
    <Section title="Pods">
      <Table
        columns={columns}
        data={rows}
        getRowKey={(p) => p.name}
        onRowClick={
          onOpenResource
            ? (pod) => onOpenResource({ kind: "Pod", namespace: pod.namespace || namespace, name: pod.name })
            : undefined
        }
        emptyText="No pods"
      />
    </Section>
  );
}
