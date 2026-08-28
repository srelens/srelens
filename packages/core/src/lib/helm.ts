import { invokeCapability, invokeCommand, subscribe, type Invoker } from "../transport/transport";
import type { DiffRow } from "./manifest";

export interface HelmReleaseSummary {
  name: string;
  namespace: string;
  revision: number;
  status: string;
  chart: string;
  chartVersion: string;
  appVersion: string;
  updated: string;
}

export interface HelmRevision {
  revision: number;
  status: string;
  updated: string;
  chartVersion: string;
  description: string;
}

export interface HelmReleaseDetail extends HelmReleaseSummary {
  valuesYaml: string;
  manifest: string;
  notes: string;
  history: HelmRevision[];
}

/** List installed Helm releases (latest revision each) via `k8s.listHelmReleases`. */
export async function listHelmReleases(
  context: string,
  namespace: string | null = null,
  invoke: Invoker = invokeCapability,
): Promise<{ releases?: HelmReleaseSummary[]; error?: string }> {
  try {
    const out = await invoke<{ releases: HelmReleaseSummary[] }>("k8s.listHelmReleases", {
      context,
      namespace: namespace ?? "",
    });
    return { releases: out.releases };
  } catch (e) {
    return { error: String(e) };
  }
}

/** Fetch a Helm release's values, manifest, and history via `k8s.getHelmRelease`. */
export async function getHelmRelease(
  context: string,
  namespace: string,
  name: string,
  invoke: Invoker = invokeCapability,
): Promise<{ release?: HelmReleaseDetail; error?: string }> {
  try {
    const release = await invoke<HelmReleaseDetail>("k8s.getHelmRelease", {
      context,
      namespace,
      name,
    });
    return { release };
  } catch (e) {
    return { error: String(e) };
  }
}

export interface HelmChartReq {
  name: string;
  chart: string;
  namespace?: string | null;
  values?: string;
  version?: string;
}

export interface HelmChartRef {
  name: string;
  version: string;
  appVersion: string;
  description: string;
}

/** Resolve a chart's full ref(s) and available versions from configured repos
 * via `k8s.helmSearchRepo`. No repo match (OCI ref, local path, or the repo
 * isn't added) is a normal outcome — `entries` comes back empty, not an error. */
export async function helmSearchRepo(
  context: string,
  chart: string,
  invoke: Invoker = invokeCapability,
): Promise<{ entries?: HelmChartRef[]; error?: string }> {
  try {
    const out = await invoke<{ entries: HelmChartRef[] }>("k8s.helmSearchRepo", { context, chart });
    return { entries: out.entries };
  } catch (e) {
    return { error: String(e) };
  }
}

async function helmOp(
  id: string,
  args: Record<string, unknown>,
  invoke: Invoker,
): Promise<{ output?: string; error?: string }> {
  try {
    const out = await invoke<{ output: string }>(id, args);
    return { output: out.output };
  } catch (e) {
    return { error: String(e) };
  }
}

export async function helmVersion(
  context: string,
  invoke: Invoker = invokeCapability,
): Promise<{ version?: string; error?: string }> {
  try {
    const out = await invoke<{ version: string }>("k8s.helmVersion", { context });
    return { version: out.version };
  } catch (e) {
    return { error: String(e) };
  }
}

export const helmTemplate = (context: string, req: HelmChartReq, invoke: Invoker = invokeCapability) =>
  helmOp(
    "k8s.helmTemplate",
    { context, name: req.name, chart: req.chart, namespace: req.namespace ?? null, values: req.values ?? "", version: req.version ?? null },
    invoke,
  );

export const helmInstall = (context: string, req: HelmChartReq, invoke: Invoker = invokeCapability) =>
  helmOp(
    "k8s.helmInstall",
    { context, name: req.name, chart: req.chart, namespace: req.namespace ?? null, values: req.values ?? "", version: req.version ?? null },
    invoke,
  );

export const helmUpgrade = (context: string, req: HelmChartReq, invoke: Invoker = invokeCapability) =>
  helmOp(
    "k8s.helmUpgrade",
    { context, name: req.name, chart: req.chart, namespace: req.namespace ?? null, values: req.values ?? "", version: req.version ?? null },
    invoke,
  );

export const helmRollback = (
  context: string,
  req: { name: string; revision: number; namespace?: string | null },
  invoke: Invoker = invokeCapability,
) => helmOp("k8s.helmRollback", { context, name: req.name, revision: req.revision, namespace: req.namespace ?? null }, invoke);

export const helmUninstall = (
  context: string,
  req: { name: string; namespace?: string | null },
  invoke: Invoker = invokeCapability,
) => helmOp("k8s.helmUninstall", { context, name: req.name, namespace: req.namespace ?? null }, invoke);

export const helmRepoAdd = (
  context: string,
  req: { name: string; url: string },
  invoke: Invoker = invokeCapability,
) => helmOp("k8s.helmRepoAdd", { context, name: req.name, url: req.url }, invoke);

export const helmRepoUpdate = (context: string, invoke: Invoker = invokeCapability) =>
  helmOp("k8s.helmRepoUpdate", { context }, invoke);

let helmSeq = 0;

/** Run a streamed helm operation; `onData` gets each output line, `onExit`
 * fires with null on success or an error string. */
export async function startHelmOp(
  context: string,
  args: string[],
  onData: (line: string) => void,
  onExit: (err: string | null) => void,
  extraKubeconfigs: string[] = [],
  values: string = "",
): Promise<{ close: () => void }> {
  const channel = `helm-${helmSeq++}`;
  const disposeOut = await subscribe(`helm:out:${channel}`, (p) => onData(String(p)));
  const disposeExit = await subscribe(`helm:exit:${channel}`, (p) => onExit((p as string | null) ?? null));
  let session: number;
  try {
    session = await invokeCommand<number>("start_helm_op", { context, extraKubeconfigs, args, values, channel });
  } catch (e) {
    disposeOut();
    disposeExit();
    throw e;
  }
  return {
    close: () => {
      disposeOut();
      disposeExit();
      void invokeCommand("helm_op_close", { session });
    },
  };
}

/** Above this many lines on either side, fall back to an index-aligned diff —
 * the LCS matrix would be O(n·m) and can freeze/OOM the WebView. */
const MAX_LCS_LINES = 2000;

/** Line-level diff of two texts into DiffRow[] for <DiffView>. Longest-common-
 * subsequence over lines; runs of change become same/insert/delete rows (or,
 * past MAX_LCS_LINES, an index-aligned fallback that also emits "replace"). */
export function diffTextLines(left: string, right: string): DiffRow[] {
  const a = left.length ? left.split("\n") : [];
  const b = right.length ? right.split("\n") : [];
  if (a.length > MAX_LCS_LINES || b.length > MAX_LCS_LINES) {
    const rows: DiffRow[] = [];
    const max = Math.max(a.length, b.length);
    for (let i = 0; i < max; i++) {
      const l = i < a.length ? a[i] : null;
      const r = i < b.length ? b[i] : null;
      if (l !== null && r !== null) rows.push({ tag: l === r ? "same" : "replace", left: l, right: r });
      else if (l !== null) rows.push({ tag: "delete", left: l, right: null });
      else rows.push({ tag: "insert", left: null, right: r as string });
    }
    return rows;
  }
  const n = a.length;
  const m = b.length;
  const lcs: number[][] = Array.from({ length: n + 1 }, () => new Array(m + 1).fill(0));
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      lcs[i][j] = a[i] === b[j] ? lcs[i + 1][j + 1] + 1 : Math.max(lcs[i + 1][j], lcs[i][j + 1]);
    }
  }
  const rows: DiffRow[] = [];
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (a[i] === b[j]) {
      rows.push({ tag: "same", left: a[i], right: b[j] });
      i++;
      j++;
    } else if (lcs[i + 1][j] >= lcs[i][j + 1]) {
      rows.push({ tag: "delete", left: a[i], right: null });
      i++;
    } else {
      rows.push({ tag: "insert", left: null, right: b[j] });
      j++;
    }
  }
  while (i < n) rows.push({ tag: "delete", left: a[i++], right: null });
  while (j < m) rows.push({ tag: "insert", left: null, right: b[j++] });
  return rows;
}
