import React, { useEffect, useRef, useState } from "react";
import { podMetrics, podsForSelector } from "@srelens/core";
import { nodeMetrics } from "@srelens/core";
import { Sparkline } from "../ui";

interface Sample {
  t: number; // epoch ms
  cpu: number; // millicores
  mem: number; // MiB
}

/** Kinds that can show a metrics timeline. Workload kinds are aggregated across their pods. */
export type MetricsKind = "Pod" | "Node" | "Deployment" | "StatefulSet" | "DaemonSet" | "ReplicaSet" | "Job";

const WORKLOAD_KINDS: MetricsKind[] = ["Deployment", "StatefulSet", "DaemonSet", "ReplicaSet", "Job"];
const isWorkload = (kind: MetricsKind) => WORKLOAD_KINDS.includes(kind);

export type MetricsRange = "5m" | "10m" | "30m" | "1h";

// Each range picks a sampling cadence that keeps the series near ~30 points, so
// longer windows don't accumulate unbounded samples. metrics-server only reports
// the current value, so the window fills in over time rather than back-filling.
const RANGES: { id: MetricsRange; label: string; windowMs: number; intervalMs: number }[] = [
  { id: "5m", label: "5m", windowMs: 5 * 60_000, intervalMs: 10_000 },
  { id: "10m", label: "10m", windowMs: 10 * 60_000, intervalMs: 20_000 },
  { id: "30m", label: "30m", windowMs: 30 * 60_000, intervalMs: 60_000 },
  { id: "1h", label: "1h", windowMs: 60 * 60_000, intervalMs: 120_000 },
];

function sumMetrics(rows: { cpuMillicores: number; memoryMiB: number }[]): { cpu: number; mem: number } {
  return rows.reduce((acc, m) => ({ cpu: acc.cpu + m.cpuMillicores, mem: acc.mem + m.memoryMiB }), { cpu: 0, mem: 0 });
}

/**
 * A live CPU/memory timeline for a Pod, Node, or workload controller. The
 * Metrics Server only reports the current value, so — like Lens — we poll and
 * build the series over time. Workload kinds (Deployment/StatefulSet/…) sum the
 * usage of their pods (matched by label selector). A time-range filter chooses
 * the retention window and sampling cadence.
 *
 * The *Fn props are injectable for testing.
 */
export function MetricsPanel({
  kind,
  context,
  namespace,
  name,
  selector,
  intervalMs,
  range: initialRange = "5m",
  podMetricsFn = podMetrics,
  nodeMetricsFn = nodeMetrics,
  podsForSelectorFn = podsForSelector,
}: {
  kind: MetricsKind;
  context: string;
  namespace: string | null;
  name: string;
  /** Label selector for workload kinds — the pods whose usage is summed. */
  selector?: Record<string, string>;
  /** Override the range's sampling cadence (used by tests to pin ticks). */
  intervalMs?: number;
  range?: MetricsRange;
  podMetricsFn?: typeof podMetrics;
  nodeMetricsFn?: typeof nodeMetrics;
  podsForSelectorFn?: typeof podsForSelector;
}) {
  const [range, setRange] = useState<MetricsRange>(initialRange);
  const [series, setSeries] = useState<Sample[]>([]);
  const [status, setStatus] = useState<"loading" | "ok" | "unavailable">("loading");
  const gotData = useRef(false);

  const cfg = RANGES.find((r) => r.id === range) ?? RANGES[0];
  const effectiveInterval = intervalMs ?? cfg.intervalMs;
  const selectorKey = JSON.stringify(selector ?? null);

  useEffect(() => {
    let active = true;
    gotData.current = false;
    setSeries([]);
    setStatus("loading");

    async function sample(): Promise<{ cpu: number; mem: number } | null> {
      if (kind === "Node") {
        const out = await nodeMetricsFn(context);
        const m = out.metrics?.find((x) => x.name === name);
        return m ? { cpu: m.cpuMillicores, mem: m.memoryMiB } : null;
      }
      if (kind === "Pod") {
        const out = await podMetricsFn(context, namespace ?? "");
        const m = out.metrics?.find((x) => x.name === name);
        return m ? { cpu: m.cpuMillicores, mem: m.memoryMiB } : null;
      }
      // Workload: sum the usage of the controller's pods (matched by selector).
      if (!selector || Object.keys(selector).length === 0) return null;
      const ns = namespace ?? "";
      const [podsOut, metricsOut] = await Promise.all([
        podsForSelectorFn(context, ns, selector),
        podMetricsFn(context, ns),
      ]);
      const metrics = metricsOut.metrics ?? [];
      if (metrics.length === 0) return null; // metrics-server unavailable / no data
      const names = new Set((podsOut.pods ?? []).map((p) => p.name));
      return sumMetrics(metrics.filter((m) => names.has(m.name)));
    }

    async function tick() {
      const s = await sample();
      if (!active) return;
      if (s === null) {
        // A transient miss shouldn't wipe an existing series; only show the
        // empty state if we've never received a sample.
        if (!gotData.current) setStatus("unavailable");
        return;
      }
      gotData.current = true;
      setStatus("ok");
      const now = Date.now();
      setSeries((prev) =>
        [...prev, { t: now, cpu: s.cpu, mem: s.mem }].filter((x) => now - x.t <= cfg.windowMs),
      );
    }

    void tick();
    const timer = setInterval(() => void tick(), effectiveInterval);
    return () => {
      active = false;
      clearInterval(timer);
    };
  }, [kind, context, namespace, name, selectorKey, effectiveInterval, cfg.windowMs, podMetricsFn, nodeMetricsFn, podsForSelectorFn]);

  const rangePicker = (
    <div className="fl-metrics__range" role="group" aria-label="Metrics time range">
      {RANGES.map((r) => (
        <button
          key={r.id}
          type="button"
          className={range === r.id ? "is-active" : undefined}
          aria-pressed={range === r.id}
          onClick={() => setRange(r.id)}
        >
          {r.label}
        </button>
      ))}
    </div>
  );

  if (status === "unavailable") {
    return (
      <section className="fl-metrics">
        <div className="fl-metrics__header">
          <h4 className="fl-detail-section__title">Metrics</h4>
          {rangePicker}
        </div>
        <p className="fl-detail-empty" style={{ margin: 0 }}>
          No metrics available — the cluster needs the Kubernetes Metrics Server.
        </p>
      </section>
    );
  }

  const latest = series[series.length - 1];
  const cores = latest ? (latest.cpu / 1000).toFixed(3) : "—";

  return (
    <section className="fl-metrics">
      <div className="fl-metrics__header">
        <h4 className="fl-detail-section__title">Metrics</h4>
        {rangePicker}
      </div>
      <p className="fl-metrics__source">
        {isWorkload(kind) ? "Aggregated across pods · " : ""}Live from the Kubernetes Metrics Server
      </p>

      <div className="fl-metric">
        <div className="fl-metric__head">
          <span className="fl-metric__name">CPU</span>
          <span className="fl-metric__value">{latest ? `${cores} cores` : "—"}</span>
        </div>
        <Sparkline values={series.map((s) => s.cpu)} color="var(--fl-color-accent)" ariaLabel="CPU usage" />
      </div>

      <div className="fl-metric">
        <div className="fl-metric__head">
          <span className="fl-metric__name">Memory</span>
          <span className="fl-metric__value">{latest ? `${latest.mem} MiB` : "—"}</span>
        </div>
        <Sparkline values={series.map((s) => s.mem)} color="#7aa2f7" ariaLabel="Memory usage" />
      </div>
    </section>
  );
}
