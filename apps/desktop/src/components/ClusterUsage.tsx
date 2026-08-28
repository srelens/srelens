import React, { useEffect, useState } from "react";
import { nodeMetrics } from "@srelens/core";

/** Format MiB compactly: GiB once it's large enough. */
function fmtMem(mib: number): string {
  return mib >= 1024 ? `${(mib / 1024).toFixed(1)} GiB` : `${mib} MiB`;
}

/**
 * Live cluster CPU/memory usage for the status bar — sums node metrics and
 * polls. Renders nothing until a sample arrives (e.g. no metrics-server).
 * `nodeMetricsFn` is injectable for testing.
 */
export function ClusterUsage({
  context,
  intervalMs = 10000,
  nodeMetricsFn = nodeMetrics,
}: {
  context: string;
  intervalMs?: number;
  nodeMetricsFn?: typeof nodeMetrics;
}) {
  const [usage, setUsage] = useState<{ cpu: number; mem: number } | null>(null);

  useEffect(() => {
    let active = true;
    setUsage(null);
    async function tick() {
      const out = await nodeMetricsFn(context);
      if (!active) return;
      if (out.metrics && out.metrics.length) {
        setUsage({
          cpu: out.metrics.reduce((s, m) => s + m.cpuMillicores, 0),
          mem: out.metrics.reduce((s, m) => s + m.memoryMiB, 0),
        });
      }
    }
    void tick();
    const id = setInterval(() => void tick(), intervalMs);
    return () => {
      active = false;
      clearInterval(id);
    };
  }, [context, intervalMs, nodeMetricsFn]);

  if (!usage) return null;
  return (
    <span className="flex items-center gap-3 tabular-nums">
      <span title="Cluster CPU usage">CPU {usage.cpu}m</span>
      <span title="Cluster memory usage">Mem {fmtMem(usage.mem)}</span>
    </span>
  );
}
