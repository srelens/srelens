import { describe, it, expect } from "vitest";
import type { ReactElement } from "react";
import { render } from "@testing-library/react";
import { cronJobStatus, jobStatus, scaledStatus } from "@srelens/core";
import {
  podColumns,
  deploymentColumns,
  statefulSetColumns,
  daemonSetColumns,
  jobColumns,
  cronJobColumns,
  nodeColumns,
  configMapColumns,
  secretColumns,
  resourceQuotaColumns,
  limitRangeColumns,
  serviceColumns,
  ingressColumns,
  endpointSliceColumns,
  networkPolicyColumns,
  pvcColumns,
  pvColumns,
  storageClassColumns,
  serviceAccountColumns,
  roleColumns,
  clusterRoleColumns,
  roleBindingColumns,
  clusterRoleBindingColumns,
  podFlagged,
  deploymentFlagged,
  statefulSetFlagged,
  daemonSetFlagged,
  jobFlagged,
  nodeFlagged,
  cronJobVerdict,
  nodeVerdict,
  daemonSetVerdict,
  deploymentVerdict,
  jobVerdict,
  statefulSetVerdict,
  type NodeRow,
  type PodRow,
} from "./columns";
import { customColumns } from "./custom";
import { genericClusterColumns, genericColumns } from "./generic";
import type { CrdRef } from "@srelens/core";

/** Every typed column set columns.tsx exports — the design mock titles every
 *  one of these "Name", never the kind, and none of them may ask for a
 *  per-column funnel (the mock has one search box, not 23). */
const ALL_TYPED_SETS = [
  podColumns,
  deploymentColumns,
  statefulSetColumns,
  daemonSetColumns,
  jobColumns,
  cronJobColumns,
  nodeColumns,
  configMapColumns,
  secretColumns,
  resourceQuotaColumns,
  limitRangeColumns,
  serviceColumns,
  ingressColumns,
  endpointSliceColumns,
  networkPolicyColumns,
  pvcColumns,
  pvColumns,
  storageClassColumns,
  serviceAccountColumns,
  roleColumns,
  clusterRoleColumns,
  roleBindingColumns,
  clusterRoleBindingColumns,
];

const pod = (over: Partial<PodRow> = {}): PodRow => ({
  name: "web-0", namespace: "default", phase: "Running", ready: "1/1",
  restarts: 0, node: "node-a", age: "3d", image: "acme/checkout-api:118a7e", ...over,
});

describe("pod columns", () => {
  it("sorts ages by duration, not by the text that renders them", () => {
    const age = podColumns.find((c) => c.key === "age")!;
    const older = age.getSortValue!(pod({ age: "1y" }));
    const newer = age.getSortValue!(pod({ age: "300d" }));
    expect(Number(older)).toBeGreaterThan(Number(newer));
  });

  it("shows an em dash where metrics-server left no reading, not a bare zero", () => {
    const cpu = podColumns.find((c) => c.key === "cpu")!;
    expect(cpu.render!(pod())).toBe("—");
    expect(cpu.render!(pod({ cpu: 12 }))).toBe("12m");
  });

  it("sorts a missing reading below every real one", () => {
    const cpu = podColumns.find((c) => c.key === "cpu")!;
    expect(Number(cpu.getSortValue!(pod()))).toBeLessThan(Number(cpu.getSortValue!(pod({ cpu: 0 }))));
  });

  it("groups a four-digit CPU reading with a thin space, not a bare run of digits", () => {
    const cpu = podColumns.find((c) => c.key === "cpu")!;
    expect(cpu.render!(pod({ cpu: 2410 }))).toBe("2 410m");
    expect(cpu.render!(pod({ cpu: 241 }))).toBe("241m");
  });

  it("puts a space before Mi, and scales at or above 1024 Mi to one-decimal Gi", () => {
    const memory = podColumns.find((c) => c.key === "memory")!;
    expect(memory.render!(pod({ memory: 988 }))).toBe("988 Mi");
    expect(memory.render!(pod({ memory: 412 }))).toBe("412 Mi");
    expect(memory.render!(pod({ memory: 3174 }))).toBe("3.1 Gi");
    expect(memory.render!(pod({ memory: 2969 }))).toBe("2.9 Gi");
  });

  it("sorts memory on the raw Mi value, never the Gi-scaled display text", () => {
    // The pair that breaks if the comparator is ever pointed at the rendered
    // string: "3.1 Gi" collates before "988 Mi" as text, backwards from the
    // 3174 Mi > 988 Mi it actually is.
    const memory = podColumns.find((c) => c.key === "memory")!;
    const gi = Number(memory.getSortValue!(pod({ memory: 3174 })));
    const mi = Number(memory.getSortValue!(pod({ memory: 988 })));
    expect(gi).toBeGreaterThan(mi);
    expect(gi).toBe(3174);
  });

  it("sorts a missing memory reading below every real one", () => {
    const memory = podColumns.find((c) => c.key === "memory")!;
    expect(Number(memory.getSortValue!(pod()))).toBeLessThan(Number(memory.getSortValue!(pod({ memory: 0 }))));
  });

  it("names the pod column Name, not the kind — the mock titles every list Name", () => {
    expect(podColumns[0].header).toBe("Name");
  });

  it("flags a pod that is not Running, and only that", () => {
    const running = { name: "web-0", namespace: "d", phase: "Running", ready: "1/1", restarts: 0, node: "n", age: "1d", image: "redis:7.4-alpine", waitingReason: "" };
    expect(podFlagged(running)).toBe(false);
    expect(podFlagged({ ...running, phase: "Pending" })).toBe(true);
  });

  it("flags a crash-looping pod, whose phase still reads Running", () => {
    // The defect: `status.phase` is "Running" for a pod whose only container
    // is restarting in a back-off loop, so a row that reads nothing but the
    // phase drew it green with no dot — while the detail header for the very
    // same pod said CrashLoopBackOff in red.
    const crashing = { name: "checkout-api-7d", namespace: "d", phase: "Running", ready: "0/1", restarts: 7, node: "n", age: "1d", image: "acme/checkout-api:4f2a1c", waitingReason: "CrashLoopBackOff" };
    expect(podFlagged(crashing)).toBe(true);
    const phase = podColumns.find((c) => c.key === "phase")!;
    const pill = phase.render!(crashing) as { props: { status: string; kind: string } };
    expect(pill.props.status).toBe("CrashLoopBackOff");
    expect(pill.props.kind).toBe("danger");
  });

  it("keeps flagging that same pod in the instant it is BETWEEN restarts", () => {
    // The second moment of the very pod above, and the one no fixture in this
    // repo used to hold: the container is briefly up, so the kubelet reports
    // no waiting reason at all and the phase is still "Running". The row used
    // to lose its dot for that instant and get it back a moment later — on a
    // real cluster, two of four consecutive screenshots.
    //
    // Only `waitingReason` differs from the row above. The ready ratio and the
    // restart count do not move between the two moments, which is exactly why
    // the verdict is now derived from them.
    const between = { name: "checkout-api-7d", namespace: "d", phase: "Running", ready: "0/1", restarts: 7, node: "n", age: "1d", image: "acme/checkout-api:4f2a1c", waitingReason: "" };
    expect(podFlagged(between)).toBe(true);
    const phase = podColumns.find((c) => c.key === "phase")!;
    const pill = phase.render!(between) as { props: { status: string; kind: string } };
    expect(pill.props.status).toBe("NotReady");
    expect(pill.props.kind).toBe("danger");
    // The dot and the tone are what must not move; the word legitimately does,
    // because between restarts there is no reason for the kubelet to name.
    const backingOff = { ...between, waitingReason: "CrashLoopBackOff" };
    expect(podFlagged(backingOff)).toBe(podFlagged(between));
    const other = phase.render!(backingOff) as { props: { status: string; kind: string } };
    expect(other.props.kind).toBe(pill.props.kind);
  });

  it("does not flag a pod that is merely starting up, restarts or no ready containers yet", () => {
    // The other half of the same rule, and the reason the restart count is
    // read at all: a container two seconds old that has not yet passed its
    // readiness probe is a normal pod mid-rollout, not a failing one. A row
    // carries no clock, so having died at least once is the only evidence in
    // the snapshot that separates them.
    const starting = { name: "web-9", namespace: "d", phase: "Running", ready: "0/1", restarts: 0, node: "n", age: "2s", image: "redis:7.4-alpine", waitingReason: "" };
    expect(podFlagged(starting)).toBe(false);
    const phase = podColumns.find((c) => c.key === "phase")!;
    const pill = phase.render!(starting) as { props: { status: string; kind: string } };
    expect(pill.props.status).toBe("Running");
    expect(pill.props.kind).toBe("success");
  });

  it("shows an image-pull failure the same way, and sorts the column on what it shows", () => {
    const pulling = { name: "web-0", namespace: "d", phase: "Pending", ready: "0/1", restarts: 0, node: "n", age: "1d", image: "acme/missing:1", waitingReason: "ImagePullBackOff" };
    const phase = podColumns.find((c) => c.key === "phase")!;
    const pill = phase.render!(pulling) as { props: { status: string; kind: string } };
    expect(pill.props.status).toBe("ImagePullBackOff");
    expect(pill.props.kind).toBe("danger");
    expect(podFlagged(pulling)).toBe(true);
    // Sorting the Status column on the raw phase would scatter every waiting
    // pod under "Pending"/"Running" instead of grouping what the reader sees.
    expect(phase.getSortValue!(pulling)).toBe("ImagePullBackOff");
  });

  it("leaves a healthy pod reading its phase, not an empty waiting reason", () => {
    const running = { name: "web-0", namespace: "d", phase: "Running", ready: "1/1", restarts: 0, node: "n", age: "1d", image: "redis:7.4-alpine", waitingReason: "" };
    const phase = podColumns.find((c) => c.key === "phase")!;
    const pill = phase.render!(running) as { props: { status: string; kind: string } };
    expect(pill.props.status).toBe("Running");
    expect(pill.props.kind).toBe("success");
    expect(phase.getSortValue!(running)).toBe("Running");
  });

  it("does not flag a Succeeded pod — phaseKind already renders it a green pill, so the dot must agree", () => {
    const succeeded = { name: "job-abc", namespace: "d", phase: "Succeeded", ready: "0/1", restarts: 0, node: "n", age: "1d", image: "redis:7.4-alpine", waitingReason: "" };
    expect(podFlagged(succeeded)).toBe(false);
    const phase = podColumns.find((c) => c.key === "phase")!;
    const pill = phase.render!(succeeded) as { props: { kind: string } };
    expect(pill.props.kind).toBe("success");
  });

  it("shows the pod's container image, comma-joined for a multi-container pod", () => {
    const image = podColumns.find((c) => c.key === "image")!;
    expect(image.header).toBe("Image");
    expect(image.render!(pod({ image: "acme/checkout-api:118a7e, envoyproxy/envoy:v1.30" }))).toBe(
      "acme/checkout-api:118a7e, envoyproxy/envoy:v1.30",
    );
  });

  it("falls back to an em dash for a pod with no containers, like the other optional text columns", () => {
    const image = podColumns.find((c) => c.key === "image")!;
    expect(image.render!(pod({ image: "" }))).toBe("—");
  });

  it("drops the Node column — the design does not show one for pods", () => {
    expect(podColumns.some((c) => c.key === "node")).toBe(false);
  });

  it("keeps Image last, matching the design mock's row order", () => {
    expect(podColumns.map((c) => c.key)).toEqual([
      "name", "namespace", "ready", "phase", "restarts", "cpu", "memory", "age", "image",
    ]);
  });

  it("does not mark Image sortable — a comma-joined image list has no single natural order, and the mock renders a plain header for it", () => {
    const image = podColumns.find((c) => c.key === "image")!;
    expect(image.sortable).toBe(false);
  });
});

describe("node columns", () => {
  it("keeps no namespace column, because a node has none", () => {
    expect(nodeColumns.some((c) => c.key === "namespace")).toBe(false);
  });

  it("formats CPU and memory exactly as pods do — the same two readings must not drift", () => {
    const cpu = nodeColumns.find((c) => c.key === "cpu")!;
    const memory = nodeColumns.find((c) => c.key === "memory")!;
    const node = {
      name: "n1", status: "Ready", roles: "worker", version: "1.30", age: "9d", taints: 0, unschedulable: false,
      allocatableCpuMillicores: 4000, allocatableMemoryMiB: 8192, allocatablePods: 110, instanceType: "",
    };
    const withCpu = { ...node, cpu: 2410 };
    const withMemory = { ...node, memory: 3174 };
    expect(cpu.render!(withCpu)).toBe("2 410m");
    expect(memory.render!(withMemory)).toBe("3.1 Gi");
  });
});

/**
 * The Nodes row's two channels, pinned together.
 *
 * `withRowAffordances` draws the unhealthy dot in a hard-coded danger tone off
 * `flagged`. So the moment `nodeFlagged` existed, the Status pill's own tone
 * became a SECOND reading of the same fact — and it was `phaseKind(n.status)`,
 * which calls a cordoned-but-Ready node green. A green "Ready" beside a red
 * dot is verbatim the pairing `k8sStatus`'s header exists to prevent. (#331)
 */
describe("a node's pill and its unhealthy dot are one verdict", () => {
  const node = (over: Partial<NodeRow>): NodeRow => ({
    name: "n1", status: "Ready", roles: "worker", version: "1.30", age: "9d",
    taints: 0, unschedulable: false,
    allocatableCpuMillicores: 4000, allocatableMemoryMiB: 8192, allocatablePods: 110, instanceType: "",
    ...over,
  });
  const statusColumn = nodeColumns.find((c) => c.key === "status")!;
  const pill = (n: NodeRow) => {
    const view = render(statusColumn.render!(n) as ReactElement);
    const el = view.container.querySelector(".status");
    return { status: el?.textContent ?? "", kind: el?.getAttribute("data-kind") };
  };

  it("tones a cordoned-but-Ready node's pill amber, beside the dot it now earns", () => {
    const cordoned = node({ unschedulable: true });
    expect(nodeFlagged(cordoned)).toBe(true);
    // Not "success": that is the green-word-beside-a-red-dot frame.
    expect(pill(cordoned)).toEqual({ status: "Ready", kind: nodeVerdict(cordoned).health });
    expect(nodeVerdict(cordoned).health).toBe("warning");
  });

  it("keeps the word the mock draws, and the SchedulingDisabled badge beside it", () => {
    const view = render(statusColumn.render!(node({ unschedulable: true, taints: 2 })) as ReactElement);
    // Only the TONE moved to the verdict — the pill still says the bare
    // readiness word, not core's combined "Ready,SchedulingDisabled" string,
    // and both badges the mock draws are still there.
    expect(view.container.querySelector(".status")?.textContent).toBe("Ready");
    expect(view.container.textContent).toContain("SchedulingDisabled");
    expect(view.container.textContent).toContain("Tainted (2)");
  });

  it("never pairs a healthy-toned pill with the dot, nor a danger-toned one without it", () => {
    for (const status of ["Ready", "NotReady", "Unknown", "SomethingNew"]) {
      for (const unschedulable of [false, true]) {
        const n = node({ status, unschedulable });
        const subject = `${status}${unschedulable ? " · cordoned" : ""}`;
        // The pill's tone IS the verdict's, so there is only one reading to
        // be wrong — asserted per case rather than assumed from the render.
        expect({ subject, kind: pill(n).kind }).toEqual({ subject, kind: nodeVerdict(n).health });
        if (nodeFlagged(n)) expect({ subject, kind: pill(n).kind }).not.toEqual({ subject, kind: "success" });
        if (!nodeFlagged(n)) expect({ subject, kind: pill(n).kind }).not.toEqual({ subject, kind: "danger" });
      }
    }
  });
});

describe("the rules every typed set follows", () => {
  it("shows a service's external IP, an em dash rather than a blank when it has none", () => {
    const external = serviceColumns.find((c) => c.key === "externalIP")!;
    expect(external.render!({ name: "s", namespace: "d", type: "ClusterIP", clusterIP: "10.0.0.1", externalIP: "", ports: "", age: "1d" })).toBe("—");
  });

  it("counts a secret's keys rather than showing them", () => {
    const keys = secretColumns.find((c) => c.key === "keys")!;
    expect(keys.render!({ name: "s", namespace: "d", type: "Opaque", keys: 3, age: "1d" })).toBe("3");
  });

  it("keeps no namespace column on a cluster-scoped kind", () => {
    expect(clusterRoleColumns.some((c) => c.key === "namespace")).toBe(false);
  });

  it("titles the identifier column Name for every one of the 23 typed sets", () => {
    for (const set of ALL_TYPED_SETS) {
      expect(set[0].key).toBe("name");
      expect(set[0].header).toBe("Name");
    }
  });

  it("asks for no per-column funnel anywhere — the mock has one search box, not 23", () => {
    for (const set of ALL_TYPED_SETS) {
      expect(set.some((c) => c.filterable)).toBe(false);
    }
  });
});

describe("flagged rows — the design's unhealthy dot, per kind", () => {
  it("flags a Deployment or StatefulSet whose ready count falls short of desired", () => {
    expect(deploymentFlagged({ name: "d", namespace: "ns", ready: "3/3", upToDate: 3, available: 3, age: "1d" })).toBe(false);
    expect(deploymentFlagged({ name: "d", namespace: "ns", ready: "2/3", upToDate: 3, available: 2, age: "1d" })).toBe(true);

    expect(statefulSetFlagged({ name: "s", namespace: "ns", ready: "1/1", updated: 1, service: "", age: "1d" })).toBe(false);
    expect(statefulSetFlagged({ name: "s", namespace: "ns", ready: "0/1", updated: 1, service: "", age: "1d" })).toBe(true);
  });

  it("flags a DaemonSet whose ready count falls short of desired", () => {
    const base = { name: "n", namespace: "ns", desired: 3, current: 3, upToDate: 3, available: 3, age: "1d" };
    expect(daemonSetFlagged({ ...base, ready: 3 })).toBe(false);
    expect(daemonSetFlagged({ ...base, ready: 2 })).toBe(true);
  });

  it("flags a Job with any failed pod, and only that — the same count already drives the Status pill", () => {
    const base = { name: "j", namespace: "ns", completions: "1/1", active: 0, failed: 0, duration: "1m", owner: "", age: "1d" };
    expect(jobFlagged(base)).toBe(false);
    expect(jobFlagged({ ...base, failed: 1 })).toBe(true);
    expect(jobFlagged({ ...base, active: 1 })).toBe(false);
  });

  // The row chip's question comes from this boolean and the detail footer's
  // from `resourceStatusLine`; a Node had no `flagged` at all, so a NotReady
  // node asked "what is it using?" from its row and "why is it unhealthy?"
  // from its own pane. Both now read core's `nodeStatus`.
  it("flags a Node that is NotReady or cordoned, and neither when it is healthy and schedulable", () => {
    const base = {
      name: "n1", roles: "worker", version: "1.30", age: "9d", taints: 0,
      allocatableCpuMillicores: 4000, allocatableMemoryMiB: 8192, allocatablePods: 110, instanceType: "",
    };
    expect(nodeFlagged({ ...base, status: "Ready", unschedulable: false })).toBe(false);
    expect(nodeFlagged({ ...base, status: "NotReady", unschedulable: false })).toBe(true);
    expect(nodeFlagged({ ...base, status: "Ready", unschedulable: true })).toBe(true);
    expect(nodeFlagged({ ...base, status: "Unknown", unschedulable: false })).toBe(true);
  });
});

/**
 * The pill a row draws and the word its own detail header draws are one
 * verdict, not two that agree. Asserted per kind against core directly, so a
 * second table of labels and tones cannot be reintroduced here without a
 * failure. (#331)
 */
describe("a row's status pill is core's verdict, not a local table", () => {
  it("reads Deployment, StatefulSet and DaemonSet through scaledStatus, zero word included", () => {
    expect(deploymentVerdict({ name: "d", namespace: "ns", ready: "1/3", upToDate: 1, available: 1, age: "1d" }))
      .toEqual(scaledStatus("Deployment", 1, 3));
    expect(deploymentVerdict({ name: "d", namespace: "ns", ready: "0/0", upToDate: 0, available: 0, age: "1d" }).status)
      .toBe("Scaled down");
    expect(statefulSetVerdict({ name: "s", namespace: "ns", ready: "1/2", updated: 1, service: "", age: "1d" }))
      .toEqual(scaledStatus("StatefulSet", 1, 2));
    const ds = { name: "n", namespace: "ns", desired: 0, current: 0, ready: 0, upToDate: 0, available: 0, age: "1d" };
    expect(daemonSetVerdict(ds)).toEqual(scaledStatus("DaemonSet", 0, 0));
    // The zero word is the kind's own: a DaemonSet matching no node is not
    // "Scaled down".
    expect(daemonSetVerdict(ds).status).toBe("Not scheduled");
  });

  it("reads Job and CronJob through their own core verdicts", () => {
    const job = { name: "j", namespace: "ns", completions: "1/1", active: 0, failed: 0, duration: "1m", owner: "", age: "1d" };
    expect(jobVerdict({ ...job, failed: 2, active: 1 })).toEqual(jobStatus(2, 1));
    expect(jobVerdict({ ...job, active: 1 })).toEqual(jobStatus(0, 1));
    const cron = { name: "c", namespace: "ns", schedule: "* * * * *", active: 0, lastSchedule: "", age: "1d" };
    expect(cronJobVerdict({ ...cron, suspended: true })).toEqual(cronJobStatus(true));
    expect(cronJobVerdict({ ...cron, suspended: false })).toEqual(cronJobStatus(false));
  });

  it("draws the pill from that verdict rather than a literal pair", () => {
    const statusColumn = jobColumns.find((c) => c.key === "status")!;
    const job = { name: "j", namespace: "ns", completions: "0/1", active: 0, failed: 1, duration: "1m", owner: "", age: "1d" };
    const rendered = render(statusColumn.render!(job) as ReactElement);
    expect(rendered.container.querySelector(".status")?.textContent).toBe(jobStatus(1, 0).status);
    expect(rendered.container.querySelector(".status")?.getAttribute("data-kind")).toBe(jobStatus(1, 0).health);
  });
});

describe("custom-resource columns ask for no per-column funnel either", () => {
  const crd = (over: Partial<CrdRef> = {}): CrdRef => ({
    name: "widgets.example.com", group: "example.com", version: "v1", plural: "widgets",
    kind: "Widget", namespaced: true,
    printerColumns: [{ name: "Phase", type: "string", jsonPath: ".status.phase" }],
    ...over,
  });

  it("has no filterable column — the same single search box as every typed list", () => {
    expect(customColumns(crd()).some((c) => c.filterable)).toBe(false);
  });
});

describe("column alignment — a count or a measurement is end-aligned, everything else stays default", () => {
  /** [set, the keys on it that must be `align: "end"`] — every other key on
   *  the set must NOT be. Covers all 23 typed sets, not just the workloads.
   *  Typed on just `key`/`align`: the sets differ in row type, and alignment
   *  is the only thing this test needs to see. */
  const CASES: [{ key: string; align?: "start" | "end" }[], string[]][] = [
    [podColumns, ["ready", "restarts", "cpu", "memory", "age"]],
    [deploymentColumns, ["ready", "upToDate", "available", "age"]],
    [statefulSetColumns, ["ready", "updated", "age"]],
    [daemonSetColumns, ["desired", "current", "ready", "upToDate", "available", "age"]],
    [jobColumns, ["completions", "duration", "age"]],
    [cronJobColumns, ["active", "age"]],
    [nodeColumns, ["cpu", "memory", "age"]],
    [configMapColumns, ["keys", "age"]],
    [secretColumns, ["keys", "age"]],
    [resourceQuotaColumns, ["resources", "age"]],
    [limitRangeColumns, ["limits", "age"]],
    [serviceColumns, ["age"]],
    [ingressColumns, ["age"]],
    [endpointSliceColumns, ["endpoints", "age"]],
    [networkPolicyColumns, ["ingress", "egress", "age"]],
    [pvcColumns, ["capacity", "age"]],
    [pvColumns, ["capacity", "age"]],
    [storageClassColumns, ["age"]],
    [serviceAccountColumns, ["secrets", "age"]],
    [roleColumns, ["rules", "age"]],
    [clusterRoleColumns, ["rules", "age"]],
    [roleBindingColumns, ["subjects", "age"]],
    [clusterRoleBindingColumns, ["subjects", "age"]],
  ];

  it("end-aligns exactly the count and measurement columns on every typed set", () => {
    for (const [set, endKeys] of CASES) {
      for (const column of set) {
        const expected = endKeys.includes(column.key) ? "end" : undefined;
        expect(
          column.align,
          `${column.key} on a set of [${set.map((c) => c.key).join(", ")}]`,
        ).toBe(expected);
      }
    }
  });

  it("never right-aligns identity, status or descriptive text — name, status, type, image and the like", () => {
    expect(podColumns.find((c) => c.key === "name")!.align).toBeUndefined();
    expect(podColumns.find((c) => c.key === "phase")!.align).toBeUndefined();
    expect(podColumns.find((c) => c.key === "image")!.align).toBeUndefined();
    expect(secretColumns.find((c) => c.key === "type")!.align).toBeUndefined();
  });
});

// Whole-branch review (FIX 6): every test above is scoped to the 23 typed
// sets, which is exactly why the generic and custom families drifted from
// the same two rules — custom.ts headered its first column with the CRD's
// kind, and neither generic.ts nor custom.ts end-aligned Age. Widened here so
// a future drift on either family fails a test, not just a reviewer's eye.
describe("the generic and custom families follow the same two rules as the 23 typed sets", () => {
  const crd = (over: Partial<CrdRef> = {}): CrdRef => ({
    name: "widgets.example.com", group: "example.com", version: "v1", plural: "widgets",
    kind: "Widget", namespaced: true,
    printerColumns: [{ name: "Phase", type: "string", jsonPath: ".status.phase" }],
    ...over,
  });

  it("titles the identifier column Name, never the kind — generic and custom included", () => {
    expect(genericColumns[0].header).toBe("Name");
    expect(genericClusterColumns[0].header).toBe("Name");
    expect(customColumns(crd())[0].header).toBe("Name");
  });

  it("end-aligns Age — generic and custom included", () => {
    expect(genericColumns.find((c) => c.key === "age")!.align).toBe("end");
    expect(genericClusterColumns.find((c) => c.key === "age")!.align).toBe("end");
    expect(customColumns(crd()).find((c) => c.key === "age")!.align).toBe("end");
  });
});
