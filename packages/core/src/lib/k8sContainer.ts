import { asRecord, asArray, str } from "./k8sRaw";

/** The previous termination marks when Kubernetes last restarted a container. */
export function containerLastRestartTime(status: unknown): string {
  const st = asRecord(status);
  if (Number(st.restartCount ?? 0) < 1) return "";
  return str(asRecord(asRecord(st.lastState).terminated).finishedAt);
}

export function latestRestartTime(statuses: Record<string, unknown>[]): string {
  return statuses
    .map(containerLastRestartTime)
    .filter(Boolean)
    .sort((a, b) => Date.parse(b) - Date.parse(a))[0] ?? "";
}

/** Format a port as "name: port/protocol". */
export function portText(p: Record<string, unknown>): string {
  const name = str(p.name);
  const proto = str(p.protocol) || "TCP";
  return `${name ? `${name}: ` : ""}${str(p.containerPort)}/${proto}`;
}

/** Probe → chips: "tcp-socket :cluster delay=30s timeout=1s period=10s …". */
export function probeChips(probe: Record<string, unknown>): string[] {
  const chips: string[] = [];
  if (probe.httpGet) {
    const h = asRecord(probe.httpGet);
    chips.push(`http-get ${str(h.scheme || "HTTP").toLowerCase()}://:${str(h.port)}${str(h.path)}`);
  } else if (probe.tcpSocket) {
    chips.push(`tcp-socket :${str(asRecord(probe.tcpSocket).port)}`);
  } else if (probe.exec) {
    chips.push(`exec [${asArray(asRecord(probe.exec).command).map(str).join(" ")}]`);
  }
  if (probe.initialDelaySeconds != null) chips.push(`delay=${str(probe.initialDelaySeconds)}s`);
  if (probe.timeoutSeconds != null) chips.push(`timeout=${str(probe.timeoutSeconds)}s`);
  if (probe.periodSeconds != null) chips.push(`period=${str(probe.periodSeconds)}s`);
  if (probe.successThreshold != null) chips.push(`#success=${str(probe.successThreshold)}`);
  if (probe.failureThreshold != null) chips.push(`#failure=${str(probe.failureThreshold)}`);
  return chips;
}

export function resourceText(r: Record<string, unknown>): string {
  return `CPU: ${str(r.cpu) || "—"}, Memory: ${str(r.memory) || "—"}`;
}

/**
 * The same two fields as {@link resourceText}, compact: `250m · 512Mi`.
 *
 * Two forms of one fact, deliberately, and side by side so neither can be
 * written a third time somewhere else. `resourceText` names its halves because
 * it fills the value column of a key/value row, where "250m · 512Mi" beside
 * the word "Requests" says nothing about which half is which. A table cell
 * under a REQUESTS heading has the column heading doing that work and no room
 * to say it twice.
 */
export function resourceSummary(r: Record<string, unknown>): string {
  return `${str(r.cpu) || "—"} · ${str(r.memory) || "—"}`;
}

/**
 * Every container status a pod reports, in the order a reader meets them:
 * init containers, then the app containers, then any ephemeral debug
 * container attached later.
 *
 * Takes the pod's `status`, not the pod, because that is what every caller
 * already has in hand and because this module is about containers, not about
 * whole objects. An empty answer is a real one: no statuses at all means the
 * kubelet has not reported yet, which is an absence rather than a zero.
 */
export function podContainerStatuses(status: unknown): Record<string, unknown>[] {
  const st = asRecord(status);
  return [
    ...asArray(st.initContainerStatuses).map(asRecord),
    ...asArray(st.containerStatuses).map(asRecord),
    ...asArray(st.ephemeralContainerStatuses).map(asRecord),
  ];
}

/**
 * How many times a pod's containers have restarted, all told.
 *
 * A count that is not a number contributes nothing rather than poisoning the
 * total: `Number(undefined)` is NaN, and a NaN reaching a metric tile renders
 * as "NaN", which reads as a fault in srelens rather than in the payload.
 */
export function restartTotal(statuses: readonly Record<string, unknown>[]): number {
  return statuses.reduce((total, st) => {
    const n = Number(st.restartCount ?? 0);
    return Number.isFinite(n) ? total + n : total;
  }, 0);
}

/** "NAME=value" or "NAME=<secret/configMap/field>" for an env entry. */
export function envText(e: unknown): string {
  const r = asRecord(e);
  const name = str(r.name);
  if (r.value != null) return `${name}=${str(r.value)}`;
  const vf = asRecord(r.valueFrom);
  const src = vf.secretKeyRef
    ? "secret"
    : vf.configMapKeyRef
      ? "configMap"
      : vf.fieldRef
        ? "field"
        : vf.resourceFieldRef
          ? "resource"
          : "ref";
  return `${name}=<${src}>`;
}

/** "mountPath (ro) ← volume" for a volumeMount entry. */
export function mountText(m: unknown): string {
  const r = asRecord(m);
  const ro = r.readOnly === true ? " (ro)" : "";
  return `${str(r.mountPath)}${ro} ← ${str(r.name)}`;
}

/** A toleration as "key=value → effect" (or "key Exists → effect"). */
export function tolerationText(t: unknown): string {
  const r = asRecord(t);
  const key = str(r.key) || "(any taint)";
  const operator = str(r.operator) || "Equal";
  const effect = str(r.effect) || "all effects";
  const secs = r.tolerationSeconds != null ? ` for ${str(r.tolerationSeconds)}s` : "";
  const left = operator === "Exists" ? `${key} exists` : `${key}=${str(r.value)}`;
  return `${left} → ${effect}${secs}`;
}
