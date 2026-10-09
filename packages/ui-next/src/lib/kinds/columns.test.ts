import { afterEach, describe, it, expect } from "vitest";
import { createElement, type ReactElement } from "react";
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Table, filterTableData } from "@srelens/ui-kit";
import { cronJobStatus, jobStatus, nodeUsage, podUsage, scaledStatus } from "@srelens/core";
import {
  ownHistory,
  podColumns,
  deploymentColumns,
  statefulSetColumns,
  daemonSetColumns,
  jobColumns,
  cronJobColumns,
  nodeColumns,
  namespaceColumns,
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
import type { CrdRef, NodeTaint } from "@srelens/core";

/** Every typed column set columns.tsx exports — the design titles every one
 *  of these "Name", never the kind. Namespace Status is the single deliberate
 *  per-column funnel; all other searching stays in the shared search box. */
const ALL_TYPED_SETS = [
  podColumns,
  deploymentColumns,
  statefulSetColumns,
  daemonSetColumns,
  jobColumns,
  cronJobColumns,
  nodeColumns,
  namespaceColumns,
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

describe("namespace columns", () => {
  const namespace = {
    name: "legacy-billing",
    phase: "Terminating",
    labels: {
      "kubernetes.io/metadata.name": "legacy-billing",
      zone: "west",
      team: "payments",
      env: "prod",
      tier: "backend",
    },
    age: "17m",
  };

  it("puts Status and Labels between Name and Age, with only Status filterable", () => {
    expect(namespaceColumns.map((column) => column.key)).toEqual([
      "name",
      "phase",
      "labels",
      "age",
    ]);
    expect(namespaceColumns.find((column) => column.key === "phase")?.filterable).toBe(true);
    expect(namespaceColumns.find((column) => column.key === "labels")?.sortable).toBe(false);
  });

  it("renders Active green and Terminating amber through the shared phase verdict", () => {
    const status = namespaceColumns.find((column) => column.key === "phase")!;
    const terminating = status.render!(namespace) as { props: { status: string; kind: string } };
    const active = status.render!({ ...namespace, phase: "Active" }) as {
      props: { status: string; kind: string };
    };
    expect(terminating.props).toMatchObject({ status: "Terminating", kind: "warning" });
    expect(active.props).toMatchObject({ status: "Active", kind: "success" });
  });

  it("renders an unavailable legacy phase neutrally instead of inventing a cluster failure", () => {
    const status = namespaceColumns.find((column) => column.key === "phase")!;
    const unavailable = status.render!({ ...namespace, phase: "-" }) as {
      props: { status: string; kind: string };
    };

    expect(unavailable.props).toMatchObject({ status: "-", kind: "neutral" });
  });

  it("shows two user labels plus an overflow, suppresses the automatic label, and exposes all user labels by keyboard", async () => {
    const labels = namespaceColumns.find((column) => column.key === "labels")!;
    const view = render(labels.render!(namespace) as ReactElement);

    expect(view.container.textContent).toBe("env=prodteam=payments+2");
    expect(view.container.textContent).not.toContain("kubernetes.io/metadata.name");
    await userEvent.tab();
    expect((await screen.findByRole("tooltip")).textContent).toBe(
      "env=prod, team=payments, tier=backend, zone=west",
    );
  });

  it("searches every user label as key=value, including those behind +N", () => {
    const labels = namespaceColumns.find((column) => column.key === "labels")!;
    expect(labels.getValue!(namespace)).toBe(
      "env=prod team=payments tier=backend zone=west",
    );
  });

  it("renders a dash when the namespace has no user labels", () => {
    const labels = namespaceColumns.find((column) => column.key === "labels")!;
    const view = render(labels.render!({ ...namespace, labels: {} }) as ReactElement);
    expect(view.container.textContent).toBe("—");
  });
});

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

  /** The amount a usage cell prints, whatever else is drawn beside it. */
  const amount = (column: (typeof podColumns)[number], row: PodRow) => {
    const view = render(column.render!(row) as ReactElement).container;
    const text = view.querySelector(".num")?.textContent ?? view.textContent;
    cleanup();
    return text;
  };

  it("shows an em dash where metrics-server left no reading, not a bare zero", () => {
    const cpu = podColumns.find((c) => c.key === "cpu")!;
    expect(amount(cpu, pod())).toBe("—");
    expect(amount(cpu, pod({ cpu: 12, memory: 1 }))).toBe("12m");
  });

  it("sorts a missing reading below every real one", () => {
    const cpu = podColumns.find((c) => c.key === "cpu")!;
    expect(Number(cpu.getSortValue!(pod()))).toBeLessThan(Number(cpu.getSortValue!(pod({ cpu: 0 }))));
  });

  it("groups a four-digit CPU reading with a thin space, not a bare run of digits", () => {
    const cpu = podColumns.find((c) => c.key === "cpu")!;
    expect(amount(cpu, pod({ cpu: 2410, memory: 1 }))).toBe("2 410m");
    expect(amount(cpu, pod({ cpu: 241, memory: 1 }))).toBe("241m");
  });

  it("puts a space before Mi, and scales at or above 1024 Mi to one-decimal Gi", () => {
    const memory = podColumns.find((c) => c.key === "memory")!;
    expect(amount(memory, pod({ cpu: 1, memory: 988 }))).toBe("988 Mi");
    expect(amount(memory, pod({ cpu: 1, memory: 412 }))).toBe("412 Mi");
    expect(amount(memory, pod({ cpu: 1, memory: 3174 }))).toBe("3.1 Gi");
    expect(amount(memory, pod({ cpu: 1, memory: 2969 }))).toBe("2.9 Gi");
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

  /**
   * #864: `241m` does not say whether a pod is idle, climbing, or about to be
   * throttled. The CPU cell draws the last ten minutes as a small graph,
   * against what the pod was given — its limit, or its request where it has
   * no whole limit. Memory, which sits or creeps rather than moves, is a bar:
   * see "the memory bar" below.
   */
  describe("the CPU graph", () => {
    afterEach(cleanup);
    const cpu = podColumns.find((c) => c.key === "cpu")!;
    const memory = podColumns.find((c) => c.key === "memory")!;
    const draw = (column: typeof cpu, row: PodRow) => render(column.render!(row) as ReactElement).container;
    const graph = () => screen.getByRole("img");
    /** The y of each point on the line, the top of the box being 0. */
    const heights = () =>
      [...(graph().querySelector("path[fill='none']")?.getAttribute("d") ?? "").matchAll(/[ML][\d.]+,([\d.]+)/g)].map(
        (m) => Number(m[1]),
      );
    const colour = () => graph().querySelector("path[fill='none']")?.getAttribute("stroke");
    /** Readings of CPU taken `every` apart, the last of them at `T`. */
    const T = Date.UTC(2026, 9, 9, 10, 0, 0);
    const past = (values: number[], every = 10_000) =>
      values.map((cpu, i) => ({ at: T - (values.length - 1 - i) * every, cpu, memory: 0 }));
    /** The x of each point on the line. */
    const places = () =>
      [...(graph().querySelector("path[fill='none']")?.getAttribute("d") ?? "").matchAll(/[ML]([\d.]+),[\d.]+/g)].map(
        (m) => Number(m[1]),
      );
    const limited = pod({
      cpu: 250, memory: 300,
      usageHistory: past([100, 180, 250]),
      cpuReqMillicores: 100, cpuLimMillicores: 500, cpuLimAll: true,
      memReqMiB: 128, memLimMiB: 400, memLimAll: true,
    });

    it("draws the pod's readings as a line, one point for each", () => {
      draw(cpu, limited);
      expect(heights()).toHaveLength(3);
      // Climbing: each point higher in the box than the last.
      const ys = heights();
      expect(ys[1]).toBeLessThan(ys[0]);
      expect(ys[2]).toBeLessThan(ys[1]);
    });

    it("names the graph for the pod, the resource, what it covers, and what it is a share of", () => {
      // Sixty-one readings ten seconds apart: the whole ten minutes.
      draw(cpu, { ...limited, usageHistory: past(Array.from({ length: 61 }, () => 250)) });
      expect(graph().getAttribute("aria-label")).toBe(
        "web-0 CPU over the last 10 minutes, now 250m, 50% of 500m limit",
      );
    });

    it("says how much past it is really showing, while it is still filling", () => {
      // Three readings: twenty seconds, and not called ten minutes.
      draw(cpu, limited);
      expect(graph().getAttribute("aria-label")).toBe(
        "web-0 CPU over the last 20 seconds, now 250m, 50% of 500m limit",
      );
      cleanup();
      draw(cpu, { ...limited, usageHistory: past([100, 250], 60_000) });
      expect(graph().getAttribute("aria-label")).toContain("over the last 1 minute,");
      cleanup();
      // One reading covers no time at all, and claims none.
      draw(cpu, { ...limited, usageHistory: past([250]) });
      expect(graph().getAttribute("aria-label")).toBe("web-0 CPU, now 250m, 50% of 500m limit");
    });

    it("places each reading by when it was taken, so a gap in them is as wide as it lasted", () => {
      // Two readings ten seconds apart, then nothing for five minutes.
      const gapped = [
        { at: T - 310_000, cpu: 100, memory: 0 },
        { at: T - 300_000, cpu: 110, memory: 0 },
        { at: T, cpu: 250, memory: 0 },
      ];
      draw(cpu, { ...limited, usageHistory: gapped });
      const xs = places();
      // The middle reading is a thirty-first of the way along, not half.
      expect(xs[1] / xs[2]).toBeCloseTo(10 / 310, 3);
    });

    /**
     * History is kept by name. A StatefulSet's `web-0` deleted and created
     * again is a new pod under the old name, and must not be drawn with the
     * load of the one before it.
     */
    it("draws nothing from before the pod was created", () => {
      const inherited = past([400, 420, 440, 30, 40]);
      // Created between the third reading and the fourth.
      const created = new Date(inherited[3].at - 1000).toISOString();
      draw(cpu, { ...limited, cpu: 40, created, usageHistory: inherited });
      expect(heights()).toHaveLength(2);
      expect(ownHistory({ created, usageHistory: inherited }).map((s) => s.cpu)).toEqual([30, 40]);
    });

    it("draws everything it holds for a pod whose creation time is not known", () => {
      const held = past([100, 180, 250]);
      expect(ownHistory({ created: null, usageHistory: held })).toHaveLength(3);
      expect(ownHistory({ created: "not a date", usageHistory: held })).toHaveLength(3);
      expect(ownHistory({ usageHistory: undefined })).toEqual([]);
    });

    it("is the CPU column's alone: memory draws no graph", () => {
      draw(memory, limited);
      expect(screen.queryByRole("img")).toBeNull();
      expect(screen.getByRole("meter")).toBeDefined();
      cleanup();
      draw(cpu, limited);
      expect(screen.queryByRole("meter")).toBeNull();
    });

    it("keeps the amount and the share beside the graph, and says the same in the tooltip", () => {
      const view = draw(cpu, limited);
      const figures = [...view.querySelectorAll(".num")].map((n) => n.textContent);
      expect(figures).toEqual(["250m", "50%"]);
      expect((view.firstElementChild as HTMLElement).title).toBe("250m, 50% of 500m limit");
    });

    it("draws against the limit, so a pod far under it is a low line and one near it a high one", () => {
      draw(cpu, { ...limited, cpu: 40, usageHistory: past([30, 35, 40]) });
      // 8% of the limit: in the bottom quarter of the 18px box.
      expect(Math.min(...heights())).toBeGreaterThan(18 * 0.75);
      cleanup();
      draw(cpu, { ...limited, cpu: 490, usageHistory: past([470, 480, 490]) });
      expect(Math.max(...heights())).toBeLessThan(18 * 0.25);
    });

    it("measures against the request, and says so, when a container has no limit", () => {
      const view = draw(cpu, { ...limited, cpuLimAll: false });
      expect(graph().getAttribute("aria-label")).toContain("250% of 100m request");
      expect((view.firstElementChild as HTMLElement).title).toBe("250m, 250% of 100m request");
    });

    it("colours the line by load against a limit, and not at all against a request", () => {
      const stroke = (row: PodRow) => {
        draw(cpu, row);
        const c = colour();
        cleanup();
        return c;
      };
      const nearLimit = stroke({ ...limited, cpu: 480 });
      const idleLimit = stroke({ ...limited, cpu: 10 });
      expect(nearLimit).not.toBe(idleLimit);
      // Past its request by any margin: one quiet tone, the same at 50% as at 900%.
      const overRequest = stroke({ ...limited, cpuLimAll: false, cpu: 900 });
      const underRequest = stroke({ ...limited, cpuLimAll: false, cpu: 50 });
      expect(overRequest).toBe(underRequest);
      expect(overRequest).not.toBe(nearLimit);
    });

    it("agrees with core: the share is podUsage's", () => {
      const over = { ...limited, cpu: 700 };
      const view = draw(cpu, over);
      expect([...view.querySelectorAll(".num")].map((n) => n.textContent)).toEqual(["700m", "140%"]);
      expect(podUsage(over, { cpuMillicores: 700, memoryMiB: 300 }).cpu?.percent).toBe(140);
    });

    it("draws a dash and no graph when there is no reading — an empty graph would say the pod is idle", () => {
      const view = draw(cpu, { ...limited, cpu: undefined, memory: undefined });
      expect(view.textContent).toBe("—");
      expect(screen.queryByRole("img")).toBeNull();
    });

    it("draws a pod with neither a limit nor a request to its own peak, with no share, and says why", () => {
      const view = draw(cpu, pod({ cpu: 250, memory: 300, usageHistory: past([50, 250]) }));
      expect([...view.querySelectorAll(".num")].map((n) => n.textContent)).toEqual(["250m", ""]);
      expect((view.firstElementChild as HTMLElement).title).toBe("250m, no request or limit set");
      expect(graph().getAttribute("aria-label")).toContain("no request or limit set");
      // Its own peak is the top of the box.
      expect(Math.min(...heights())).toBeLessThan(18 * 0.25);
    });

    it("draws the one reading it has as a flat line, when nothing older is held", () => {
      // The list has just been opened: no past yet, and none invented.
      draw(cpu, { ...limited, usageHistory: undefined });
      const d = graph().querySelector("path[fill='none']")?.getAttribute("d") ?? "";
      const ys = [...d.matchAll(/[ML][\d.]+,([\d.]+)/g)].map((m) => Number(m[1]));
      expect(new Set(ys).size).toBe(1);
      expect(d).not.toContain("NaN");
    });

    it("has no frame of its own: it is part of the row", () => {
      const view = draw(cpu, limited);
      const svg = view.querySelector("svg")!;
      for (const node of [svg, svg.parentElement!, view.firstElementChild as HTMLElement]) {
        expect(node.getAttribute("class") ?? "").not.toMatch(/\b(border|ring|shadow|bg-)/);
        expect((node as HTMLElement).style?.border ?? "").toBe("");
      }
      expect(svg.querySelector("rect")).toBeNull();
    });

    it("asks for the same room with or without a graph, as the Nodes list does", () => {
      const withGraph = (draw(cpu, limited).firstElementChild as HTMLElement).className;
      cleanup();
      const without = (draw(cpu, pod()).firstElementChild as HTMLElement).className;
      expect(withGraph).toBe(without);
      expect(withGraph).toMatch(/min-w-\[/);
      // And cannot be dragged narrower than that room.
      const nodeCpu = nodeColumns.find((c) => c.key === "cpu")!;
      expect(cpu.minWidth).toBe(nodeCpu.minWidth);
      expect(memory.minWidth).toBe(nodeCpu.minWidth);
    });

    it("still sorts by the amount in use, with no reading last", () => {
      const big = pod({ cpu: 900, memory: 1 });
      const tight = { ...limited, cpu: 450 };
      expect(cpu.getSortValue!(big)).toBeGreaterThan(cpu.getSortValue!(tight) as number);
      expect(cpu.getSortValue!(pod())).toBeLessThan(cpu.getSortValue!(pod({ cpu: 0 })) as number);
    });
  });

  /**
   * Memory is a bar, as a node's is. It mostly sits where it is, and what a
   * reader asks of it is how full the pod is — which a bar answers at a
   * glance and a nearly flat line does not.
   */
  describe("the memory bar", () => {
    afterEach(cleanup);
    const memory = podColumns.find((c) => c.key === "memory")!;
    const draw = (row: PodRow) => render(memory.render!(row) as ReactElement).container;
    const limited = pod({
      cpu: 250, memory: 300,
      cpuReqMillicores: 100, cpuLimMillicores: 500, cpuLimAll: true,
      memReqMiB: 128, memLimMiB: 400, memLimAll: true,
    });

    it("draws the share of the pod's limit, named for the pod, the resource and the bound", () => {
      draw(limited);
      // 300 Mi of 400 Mi.
      const bar = screen.getByRole("meter", { name: "web-0 memory, of limit" });
      expect(bar.getAttribute("aria-valuenow")).toBe("75");
      expect(bar.getAttribute("aria-valuetext")).toBe("75%");
    });

    it("keeps the amount beside the bar, and says what it is a share of", () => {
      const view = draw(limited);
      expect(view.querySelector(".num")?.textContent).toBe("300 Mi");
      expect((view.firstElementChild as HTMLElement).title).toBe("300 Mi, 75% of 400 Mi limit");
    });

    it("measures against the request, and says so, when a container has no limit", () => {
      const view = draw({ ...limited, memLimAll: false });
      // 300 Mi of the 128 Mi requested: the bar is full, the words say the real figure.
      const bar = screen.getByRole("meter", { name: "web-0 memory, of request" });
      expect(bar.getAttribute("aria-valuenow")).toBe("100");
      expect(bar.getAttribute("aria-valuetext")).toBe("234%");
      expect((view.firstElementChild as HTMLElement).title).toBe("300 Mi, 234% of 128 Mi request");
    });

    it("tints by load against a limit, and not at all against a request", () => {
      const fill = (row: PodRow) => {
        draw(row);
        const colour = (screen.getByRole("meter").firstElementChild as HTMLElement).style.background;
        cleanup();
        return colour;
      };
      // Near its ceiling is near an OOM kill: a warning worth a colour.
      const nearLimit = fill({ ...limited, memory: 390 });
      const idleLimit = fill({ ...limited, memory: 20 });
      expect(nearLimit).not.toBe(idleLimit);
      const overRequest = fill({ ...limited, memLimAll: false, memory: 900 });
      const underRequest = fill({ ...limited, memLimAll: false, memory: 50 });
      expect(overRequest).toBe(underRequest);
      expect(overRequest).not.toBe(nearLimit);
    });

    it("agrees with core: the percentage is podUsage's, unrounded and unclamped", () => {
      const over = { ...limited, memory: 560 };
      draw(over);
      expect(screen.getByRole("meter").getAttribute("aria-valuetext")).toBe("140%");
      expect(podUsage(over, { cpuMillicores: 250, memoryMiB: 560 }).memory?.percent).toBe(140);
    });

    it("draws a dash and no bar when there is no reading — an empty bar would say the pod is idle", () => {
      const view = draw({ ...limited, cpu: undefined, memory: undefined });
      expect(view.textContent).toBe("—");
      expect(screen.queryByRole("meter")).toBeNull();
    });

    it("draws the amount and no bar for a pod with neither a limit nor a request, and says why", () => {
      const view = draw(pod({ cpu: 250, memory: 300 }));
      expect(view.querySelector(".num")?.textContent).toBe("300 Mi");
      expect(screen.queryByRole("meter")).toBeNull();
      expect((view.firstElementChild as HTMLElement).title).toBe("300 Mi, no request or limit set");
    });

    it("asks for the same room with or without a bar, and as much as the CPU column", () => {
      const withBar = (draw(limited).firstElementChild as HTMLElement).className;
      cleanup();
      const without = (draw(pod()).firstElementChild as HTMLElement).className;
      expect(withBar).toBe(without);
      expect(withBar).toMatch(/min-w-\[/);
      expect(memory.minWidth).toBe(podColumns.find((c) => c.key === "cpu")!.minWidth);
    });

    it("still sorts by the amount in use, with no reading last", () => {
      expect(memory.getSortValue!(pod({ cpu: 1, memory: 900 }))).toBeGreaterThan(
        memory.getSortValue!({ ...limited, memory: 390 }) as number,
      );
      expect(memory.getSortValue!(pod())).toBeLessThan(memory.getSortValue!(pod({ memory: 0 })) as number);
    });
  });

  /**
   * #878: the Ready column printed `1/2` — how many containers are ready, and
   * not which one is not, or why. The Containers column draws one square per
   * container instead. How each square looks is `containerBlocks.test.tsx`'s
   * to pin; this is the column around them.
   */
  describe("the Containers column", () => {
    afterEach(cleanup);
    const containers = podColumns.find((c) => c.key === "containers")!;
    const one = (over: object = {}) => ({
      name: "api", kind: "app" as const, state: "running" as const, reason: "", exitCode: null,
      ready: true, restarts: 0, image: "acme/api:1", ...over,
    });
    const stuck = one({ name: "worker", state: "waiting", reason: "CrashLoopBackOff", ready: false, restarts: 14 });

    it("stands where Ready stood, and there is no Ready column any more", () => {
      expect(podColumns.some((c) => c.key === "ready")).toBe(false);
      expect(containers.header).toBe("Containers");
    });

    it("draws a square for each container", () => {
      render(containers.render!(pod({ containers: [one(), stuck] })) as ReactElement);
      expect(screen.getAllByRole("img")).toHaveLength(2);
    });

    it("prints the ready count it always had for a row with no per-container state", () => {
      // A summary from before the backend sent it: the figure, not a blank.
      const view = render(containers.render!(pod({ ready: "1/2" })) as ReactElement).container;
      expect(view.textContent).toBe("1/2");
      expect(screen.queryByRole("img")).toBeNull();
    });

    it("sorts the pods with a container in trouble ahead of the healthy ones", () => {
      const healthy = pod({ containers: [one(), one({ name: "b" })] });
      const troubled = pod({ containers: [one(), stuck] });
      expect(containers.getSortValue!(troubled)).toBeGreaterThan(containers.getSortValue!(healthy) as number);
      expect(containers.getSortValue!(pod())).toBeLessThan(containers.getSortValue!(healthy) as number);
    });

    it("is searched by what the squares stand for, since a square holds no text", () => {
      const rows = [pod({ name: "a", containers: [one(), stuck] }), pod({ name: "b", containers: [one()] })];
      const found = (query: string) => filterTableData(rows, podColumns, query, "containers").map((r) => r.name);
      expect(found("CrashLoopBackOff")).toEqual(["a"]);
      expect(found("worker")).toEqual(["a"]);
      expect(found("acme/api")).toEqual(["a", "b"]);
      // And the old figure still finds a row that has only the figure.
      expect(filterTableData([pod({ ready: "0/3" })], podColumns, "0/3", "containers")).toHaveLength(1);
    });
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

  it("reads that same pod BETWEEN restarts the way kubectl does", () => {
    // The other moments of the very pod above, with no waiting reason and the
    // phase still "Running". The row carries kubectl's word as `status`.
    const phase = podColumns.find((c) => c.key === "phase")!;
    const pillOf = (row: PodRow) => (phase.render!(row) as { props: { status: string; kind: string } }).props;
    const between = { name: "checkout-api-7d", namespace: "d", phase: "Running", ready: "0/1", restarts: 7, node: "n", age: "1d", image: "acme/checkout-api:4f2a1c", waitingReason: "" };

    // Exited and not yet backed off: kubectl says `Error`, and it stays red
    // with its dot, as it was while backing off.
    const exited = { ...between, status: "Error" };
    expect(podFlagged(exited)).toBe(true);
    expect(pillOf(exited)).toEqual({ status: "Error", kind: "danger" });

    // Up again for a moment: kubectl says `Running`, and so does the row,
    // green and without a dot. Matching kubectl was chosen over the earlier
    // `NotReady` rule that held the dot through this moment.
    const up = { ...between, status: "Running" };
    expect(podFlagged(up)).toBe(false);
    expect(pillOf(up)).toEqual({ status: "Running", kind: "success" });
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

  it("uses the same UI typography for pod identifiers and preserves unscheduled nodes", () => {
    const { container } = render(createElement(Table<PodRow>, {
      columns: podColumns,
      data: [pod({ node: "worker-15-k8s.example.test" }), pod({ name: "pending", node: "" })],
      getRowKey: (row) => row.name,
    }));
    const rows = container.querySelectorAll("tbody tr");
    expect(rows[0].textContent).toContain("worker-15-k8s.example.test");
    expect(rows[0].querySelector(".font-mono, .code")).toBeNull();
    expect(rows[1].querySelectorAll("td")[2].textContent).toBe("—");
  });

  it("keeps Node in the table contract so a Node detail can open this list prefiltered", () => {
    const node = podColumns.find((c) => c.key === "node")!;
    expect(node.header).toBe("Node");
    expect(
      filterTableData(
        [pod({ name: "on-worker-2", node: "worker-2" }), pod({ name: "elsewhere", node: "worker-3" })],
        podColumns,
        "worker-2",
        "node",
      ).map((row) => row.name),
    ).toEqual(["on-worker-2"]);
  });

  it("keeps Image last, matching the design mock's row order", () => {
    expect(podColumns.map((c) => c.key)).toEqual([
      "name", "namespace", "node", "containers", "phase", "restarts", "cpu", "memory", "age", "image",
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

  const node = {
    name: "n1", status: "Ready", roles: "worker", version: "1.30", age: "9d", taints: 0, taintDetails: [], unschedulable: false,
    allocatableCpuMillicores: 4000, allocatableMemoryMiB: 8192, allocatablePods: 110, instanceType: "",
  };
  const cpu = nodeColumns.find((c) => c.key === "cpu")!;
  const memory = nodeColumns.find((c) => c.key === "memory")!;
  /** Draw one cell and hand back its container. */
  const draw = (column: typeof cpu, row: NodeRow) => render(column.render!(row) as ReactElement).container;

  it("shows node CPU in cores while preserving memory units", () => {
    const amount = (column: typeof cpu, row: NodeRow) => {
      const view = draw(column, row);
      const text = view.querySelector(".num")?.textContent ?? view.textContent;
      cleanup();
      return text;
    };
    expect(amount(cpu, { ...node, cpu: 2410, memory: 1 })).toBe("2.41 cores");
    expect(amount(cpu, { ...node, cpu: 10399, memory: 1 })).toBe("10.4 cores");
    expect(amount(cpu, { ...node, cpu: 964, memory: 1 })).toBe("0.964 cores");
    expect(amount(cpu, { ...node, cpu: 1000, memory: 1 })).toBe("1 core");
    expect(amount(cpu, { ...node, cpu: 0, memory: 1 })).toBe("0 cores");
    expect(amount(cpu, node)).toBe("—");
    expect(amount(memory, { ...node, cpu: 1, memory: 3174 })).toBe("3.1 Gi");
  });

  /**
   * #830: the amount alone does not say whether a node is busy. The bar is the
   * share of the node's allocatable capacity in use — the one Overview draws.
   */
  describe("the usage bar", () => {
    afterEach(cleanup);
    const loaded: NodeRow = { ...node, cpu: 1000, memory: 6144 };

    it("draws the share of the node's allocatable capacity, named for the node and the resource", () => {
      draw(cpu, loaded);
      const bar = screen.getByRole("meter", { name: "n1 CPU" });
      // 1000m of 4000m.
      expect(bar.getAttribute("aria-valuenow")).toBe("25");
      expect(bar.getAttribute("aria-valuetext")).toBe("25%");
      cleanup();

      draw(memory, loaded);
      // 6144 Mi of 8192 Mi.
      expect(screen.getByRole("meter", { name: "n1 memory" }).getAttribute("aria-valuetext")).toBe("75%");
    });

    it("keeps the amount beside the bar, and says what it is a share of", () => {
      const view = draw(cpu, loaded);
      expect(view.querySelector(".num")?.textContent).toBe("1 core");
      expect((view.firstElementChild as HTMLElement).title).toBe("1 core of 4 cores");
      cleanup();
      expect((draw(memory, loaded).firstElementChild as HTMLElement).title).toBe("6.0 Gi of 8.0 Gi");
    });

    it("agrees with Overview: the percentage is core's nodeUsage, unrounded and unclamped", () => {
      const over: NodeRow = { ...node, cpu: 5600, memory: 1 };
      draw(cpu, over);
      const bar = screen.getByRole("meter", { name: "n1 CPU" });
      // 140% of allocatable: the bar is full, the words say the real figure.
      expect(bar.getAttribute("aria-valuenow")).toBe("100");
      expect(bar.getAttribute("aria-valuetext")).toBe("140%");
      expect(nodeUsage(over, { name: "n1", cpuMillicores: 5600, memoryMiB: 1 }, undefined).cpuPercent).toBe(140);
    });

    it("draws a dash and no bar when there is no reading — an empty bar would say the node is idle", () => {
      const view = draw(cpu, node);
      expect(view.textContent).toBe("—");
      expect(screen.queryByRole("meter")).toBeNull();
    });

    it("draws the amount and no bar when the node reports no allocatable capacity", () => {
      const view = draw(cpu, { ...node, allocatableCpuMillicores: 0, cpu: 500, memory: 1 });
      expect(view.querySelector(".num")?.textContent).toBe("0.5 cores");
      expect(screen.queryByRole("meter")).toBeNull();
      expect((view.firstElementChild as HTMLElement).title).toBe("");
    });

    it("asks for the same room with or without a bar, so the column is not pinned at a dash's width", () => {
      const withBar = (draw(cpu, loaded).firstElementChild as HTMLElement).className;
      cleanup();
      const without = (draw(cpu, node).firstElementChild as HTMLElement).className;
      expect(withBar).toBe(without);
      expect(withBar).toMatch(/min-w-\[/);
    });

    it("cannot be dragged narrower than the cell will shrink to (PR #840 review)", () => {
      // A table cell does not clip: at the default 72px floor the bar ran on
      // into the column beside it. The floor is the cell's own 13rem AND the
      // 0.7rem of padding the table puts either side of it — 208px alone
      // would leave the bar overflowing by the padding.
      const REM = 16;
      const needed = 13 * REM + 2 * 0.7 * REM; // 230.4
      for (const column of [cpu, memory]) {
        expect(column.minWidth).toBeGreaterThanOrEqual(needed);
      }
      expect(cpu.minWidth).toBe(memory.minWidth);
    });

    it("sorts by the share in use, which is what the bar shows, with no reading last", () => {
      const small: NodeRow = { ...node, name: "small", allocatableCpuMillicores: 1000, cpu: 900, memory: 1 };
      const large: NodeRow = { ...node, name: "large", allocatableCpuMillicores: 16000, cpu: 3200, memory: 1 };
      // large uses more cores (3.2 against 0.9) and is the less loaded (20% against 90%).
      expect(cpu.getSortValue!(small)).toBeGreaterThan(cpu.getSortValue!(large) as number);
      expect(cpu.getSortValue!(node)).toBeLessThan(cpu.getSortValue!(large) as number);

      const tight: NodeRow = { ...node, allocatableMemoryMiB: 2048, cpu: 1, memory: 1843 };
      const roomy: NodeRow = { ...node, allocatableMemoryMiB: 65536, cpu: 1, memory: 6554 };
      expect(memory.getSortValue!(tight)).toBeGreaterThan(memory.getSortValue!(roomy) as number);
    });
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
    taints: 0, taintDetails: [], unschedulable: false,
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
    // "(2)" until #426 put the count on every tainted node's badge, singular
    // included, and gave it an accessible name that spells the number out.
    expect(view.container.textContent).toContain("Tainted \u00b7 2");
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

  it("titles the identifier column Name for every typed set", () => {
    for (const set of ALL_TYPED_SETS) {
      expect(set[0].key).toBe("name");
      expect(set[0].header).toBe("Name");
    }
  });

  it("keeps the namespace Status funnel as the one deliberate typed-list exception", () => {
    for (const set of ALL_TYPED_SETS.filter((columns) => columns !== namespaceColumns)) {
      expect(set.some((c) => c.filterable)).toBe(false);
    }
    expect(namespaceColumns.filter((column) => column.filterable).map((column) => column.key)).toEqual([
      "phase",
    ]);
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
      name: "n1", roles: "worker", version: "1.30", age: "9d", taints: 0, taintDetails: [],
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
   *  the set must NOT be. Covers every typed set, not just the workloads.
   *  Typed on just `key`/`align`: the sets differ in row type, and alignment
   *  is the only thing this test needs to see. */
  const CASES: [{ key: string; align?: "start" | "end" }[], string[]][] = [
    // Not `cpu`/`memory`, for the reason given at `nodeColumns` below (#864).
    [podColumns, ["restarts", "age"]],
    [deploymentColumns, ["ready", "upToDate", "available", "age"]],
    [statefulSetColumns, ["ready", "updated", "age"]],
    [daemonSetColumns, ["desired", "current", "ready", "upToDate", "available", "age"]],
    [jobColumns, ["completions", "duration", "age"]],
    [cronJobColumns, ["active", "age"]],
    // Not `cpu`/`memory`: a node's are an amount AND a bar (#830), which reads
    // left to right like Overview's, not as a bare figure to line up on its
    // last digit. The amount inside the cell is still right-aligned.
    [nodeColumns, ["taints", "age"]],
    [namespaceColumns, ["age"]],
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

// Whole-branch review (FIX 6): every test above is scoped to the typed
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

/**
 * #426 — the Nodes list answered "is this node tainted?" and not "how many?".
 * One taint and five drew the identical pill, so the normal control-plane
 * taint and a node with three pressure taints read the same.
 */
describe("a node's taint count", () => {
  const taint = (key: string, effect: string, value = ""): NodeTaint => ({ key, value, effect });
  const node = (over: Partial<NodeRow>): NodeRow => ({
    name: "n1", status: "Ready", roles: "worker", version: "1.30", age: "9d",
    taints: 0, taintDetails: [], unschedulable: false,
    allocatableCpuMillicores: 4000, allocatableMemoryMiB: 8192, allocatablePods: 110, instanceType: "",
    ...over,
  });
  const statusColumn = nodeColumns.find((c) => c.key === "status")!;
  const taintsColumn = nodeColumns.find((c) => c.key === "taints")!;
  const status = (n: NodeRow) => render(statusColumn.render!(n) as ReactElement);

  it("draws no badge at all for a node with none — unchanged", () => {
    expect(status(node({})).container.textContent).not.toContain("Tainted");
  });

  it("counts the one taint a fresh control-plane node carries", () => {
    const view = status(node({ taints: 1, taintDetails: [taint("node-role.kubernetes.io/control-plane", "NoSchedule")] }));
    expect(view.container.textContent).toContain("Tainted · 1");
  });

  it("counts N, so three pressure taints cannot read as the normal one", () => {
    const view = status(
      node({
        taints: 3,
        taintDetails: [
          taint("node.kubernetes.io/memory-pressure", "NoSchedule"),
          taint("node.kubernetes.io/disk-pressure", "NoSchedule"),
          taint("team", "NoExecute", "payments"),
        ],
      }),
    );
    expect(view.container.textContent).toContain("Tainted · 3");
  });

  it("names the count for a screen reader rather than leaving it as '· 3'", () => {
    const view = status(node({ taints: 3, taintDetails: [taint("a", "NoSchedule"), taint("b", "NoSchedule"), taint("c", "NoExecute")] }));
    expect(view.container.querySelector("[aria-label]")?.getAttribute("aria-label")).toBe("3 taints");
  });

  it("says '1 taint', not '1 taints'", () => {
    const view = status(node({ taints: 1, taintDetails: [taint("a", "NoSchedule")] }));
    expect(view.container.querySelector("[aria-label]")?.getAttribute("aria-label")).toBe("1 taint");
  });

  it("lists every taint on hover, NoExecute first, and stays reachable by keyboard", () => {
    const view = status(
      node({
        taints: 2,
        taintDetails: [taint("node-role.kubernetes.io/control-plane", "NoSchedule"), taint("team", "NoExecute", "payments")],
      }),
    );
    const host = view.container.querySelector("[title]")!;
    expect(host.getAttribute("title")).toBe(
      "team=payments:NoExecute\nnode-role.kubernetes.io/control-plane=:NoSchedule",
    );
    expect(host.getAttribute("tabindex")).toBe("0");
  });

  it("offers a Taints column that starts hidden, so the default view is unchanged", () => {
    expect(taintsColumn.defaultHidden).toBe(true);
    expect(taintsColumn.sortable).toBe(true);
  });

  it("renders the per-effect tally, and a real 0 / 0 / 0 for a node with none", () => {
    expect(render(taintsColumn.render!(node({})) as ReactElement).container.textContent).toBe("0 / 0 / 0");
    const busy = node({
      taints: 3,
      taintDetails: [taint("a", "NoSchedule"), taint("b", "NoSchedule"), taint("c", "NoExecute")],
    });
    expect(render(taintsColumn.render!(busy) as ReactElement).container.textContent).toBe("2 / 0 / 1");
  });

  it("explains the three numbers on a node that has none, and lists them on one that does", () => {
    const empty = render(taintsColumn.render!(node({})) as ReactElement);
    expect(empty.container.querySelector("[title]")?.getAttribute("title")).toBe(
      "NoSchedule / PreferNoSchedule / NoExecute",
    );
    const one = render(
      taintsColumn.render!(node({ taints: 1, taintDetails: [taint("dedicated", "NoSchedule")] })) as ReactElement,
    );
    expect(one.container.querySelector("[title]")?.getAttribute("title")).toBe("dedicated=:NoSchedule");
  });

  it("sorts on the count, bringing the most-constrained nodes to the top", () => {
    const rows = [
      node({ name: "one", taints: 1, taintDetails: [taint("a", "NoSchedule")] }),
      node({ name: "none" }),
      node({ name: "three", taints: 3, taintDetails: [taint("a", "NoSchedule"), taint("b", "NoSchedule"), taint("c", "NoExecute")] }),
    ];
    const sorted = [...rows].sort(
      (a, b) => (taintsColumn.getSortValue!(b) as number) - (taintsColumn.getSortValue!(a) as number),
    );
    expect(sorted.map((n) => n.name)).toEqual(["three", "one", "none"]);
  });
});
