import { describe, it, expect, vi, beforeEach } from "vitest";
import { WATCHABLE_KINDS, type ResourceKind } from "@srelens/core";

// Hoisted for the same reason resourceList.test.tsx hoists its doubles:
// `vi.mock` is lifted above every declaration in the file.
const { listNamespaces, podMetrics } = vi.hoisted(() => ({
  listNamespaces: vi.fn(),
  podMetrics: vi.fn(),
}));
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  listNamespaces,
  podMetrics,
}));

import { NAV_GROUPS } from "../tree";
import { descriptorFor, CLUSTER_SCOPED } from "./descriptors";
import { rowKey } from "./types";

/** Every kind the sidebar offers, minus Events, which routes to its own screen. */
const SIDEBAR_KINDS = NAV_GROUPS.flatMap((g) => g.kinds).filter((k) => k !== "events");

describe("descriptors", () => {
  // This asserts coverage only: no sidebar route resolves to `undefined`. It
  // cannot catch a kind that *should* have typed columns shipping the generic
  // three instead — `NAV_GROUPS.kinds` is typed `ResourceKind[]`, `tree.ts`
  // already routes the three screen-kinds elsewhere, and `descriptorFor` falls
  // back to the generic descriptor for anything absent from `TYPED`, so every
  // element of `SIDEBAR_KINDS` resolves by construction. That guard belongs at
  // the end of Task 5, once `TYPED` is populated and "resolves" and "resolves
  // to something typed" are different claims.
  it("resolves every kind the sidebar can reach, rather than leaving a route with no descriptor", () => {
    const missing = SIDEBAR_KINDS.filter((k) => !descriptorFor(k));
    expect(missing).toEqual([]);
  });

  // Genuine fallback coverage at this stage, before any kind is typed: `leases`
  // and `runtimeclasses` are outside `WATCHABLE_KINDS` and are not `nodes` —
  // the only kinds Tasks 4-5 (which only add entries for watchable kinds, plus
  // `nodes` as the one named exception) will ever move out of the generic set.
  it("gives a namespaced kind with no typed entry the generic columns", () => {
    expect(descriptorFor("leases")!.columns.map((c) => c.key)).toEqual(["name", "namespace", "age"]);
  });

  it("gives a cluster-scoped kind with no typed entry the generic columns, minus namespace", () => {
    expect(descriptorFor("runtimeclasses")!.columns.map((c) => c.key)).toEqual(["name", "age"]);
  });

  it("asks for no per-column funnel on the generic columns either — one search box, not per-column", () => {
    expect(descriptorFor("leases")!.columns.some((c) => c.filterable)).toBe(false);
  });

  it("streams exactly the kinds the backend can watch, and polls the rest", () => {
    for (const kind of SIDEBAR_KINDS) {
      const watchable = (WATCHABLE_KINDS as readonly string[]).includes(kind);
      expect(descriptorFor(kind)!.source).toBe(watchable ? "watch" : "poll");
    }
  });

  it("names the identifier column first, for every kind", () => {
    for (const kind of SIDEBAR_KINDS) {
      expect(descriptorFor(kind)!.columns[0].key).toBe("name");
    }
  });

  it("marks the cluster-scoped kinds, and only those", () => {
    for (const kind of SIDEBAR_KINDS) {
      const expected = (CLUSTER_SCOPED as readonly string[]).includes(kind) ? "cluster" : "namespaced";
      expect(descriptorFor(kind)!.scope).toBe(expected);
    }
  });

  it("has no namespace column on a cluster-scoped kind, which would always be blank", () => {
    for (const kind of CLUSTER_SCOPED) {
      const d = descriptorFor(kind);
      if (!d) continue;
      expect(d.columns.some((c) => c.key === "namespace")).toBe(false);
    }
  });

  it("does not answer for a slug that is not a kind", () => {
    expect(descriptorFor("overview")).toBeUndefined();
    expect(descriptorFor("widgets.example.com")).toBeUndefined();
    expect(descriptorFor("constructor")).toBeUndefined();
  });

  /**
   * The kinds that must have a typed view: every kind the backend streams, plus
   * nodes and the richer polled Namespace summary.
   * A kind added to that set without columns falls back to the generic table and
   * silently loses its detail — this is what catches that.
   */
  const MUST_BE_TYPED = [...WATCHABLE_KINDS, "nodes", "namespaces"].filter((k) => k !== "events");

  it("gives every kind that should have a typed view one, rather than the generic table", () => {
    const generic = ["name", "namespace", "age"];
    const genericCluster = ["name", "age"];
    const untyped = MUST_BE_TYPED.filter((kind) => {
      const keys = descriptorFor(kind)!.columns.map((c) => c.key);
      return keys.join() === generic.join() || keys.join() === genericCluster.join();
    });
    expect(untyped).toEqual([]);
  });

  it("polls typed namespace summaries without turning their server-computed Age into a frozen watch field", async () => {
    const summaries = [
      {
        name: "legacy-billing",
        phase: "Terminating",
        labels: { env: "prod", team: "payments" },
        age: "17m",
      },
    ];
    listNamespaces.mockResolvedValue({ summaries });
    const descriptor = descriptorFor("namespaces")!;

    expect(descriptor.source).toBe("poll");
    expect(descriptor.scope).toBe("cluster");
    expect(descriptor.columns.map((column) => column.key)).toEqual([
      "name",
      "phase",
      "labels",
      "age",
    ]);
    expect(await descriptor.load!("prod", "ignored")).toEqual({ rows: summaries });
    expect(listNamespaces).toHaveBeenCalledWith("prod");
  });

  describe("flagged rows — the design's unhealthy dot", () => {
    it("flags a Pod that is not Running", () => {
      const flagged = descriptorFor("pods")!.flagged!;
      const running = { name: "p", namespace: "d", phase: "Running", ready: "1/1", restarts: 0, node: "n", age: "1d" };
      const crashing = { ...running, phase: "CrashLoopBackOff" };
      expect(flagged(running)).toBe(false);
      expect(flagged(crashing)).toBe(true);
    });

    it("flags a Deployment, StatefulSet or DaemonSet short of its desired ready count", () => {
      const shortDeployment = { name: "d", namespace: "ns", ready: "2/3", upToDate: 3, available: 2, age: "1d" };
      const shortStatefulSet = { name: "s", namespace: "ns", ready: "0/1", updated: 1, service: "", age: "1d" };
      const shortDaemonSet = {
        name: "n", namespace: "ns", desired: 3, current: 3, ready: 2, upToDate: 3, available: 3, age: "1d",
      };
      expect(descriptorFor("deployments")!.flagged!(shortDeployment)).toBe(true);
      expect(descriptorFor("statefulsets")!.flagged!(shortStatefulSet)).toBe(true);
      expect(descriptorFor("daemonsets")!.flagged!(shortDaemonSet)).toBe(true);
    });

    it("leaves a kind with no sensible notion of unhealthy — a Secret — with no flagged rule", () => {
      expect(descriptorFor("secrets")!.flagged).toBeUndefined();
      expect(descriptorFor("services")!.flagged).toBeUndefined();
    });

    it("flags a Job with a failed pod, and leaves CronJob with no flagged rule at all", () => {
      const job = { name: "j", namespace: "ns", completions: "1/1", active: 0, failed: 0, duration: "1m", owner: "", age: "1d" };
      const failed = { ...job, failed: 1 };
      expect(descriptorFor("jobs")!.flagged!(job)).toBe(false);
      expect(descriptorFor("jobs")!.flagged!(failed)).toBe(true);
      expect(descriptorFor("cronjobs")!.flagged).toBeUndefined();
    });
  });

  describe("pod metrics enrichment", () => {
    beforeEach(() => {
      podMetrics.mockReset();
    });

    /**
     * The collision: in all-namespaces mode `podMetrics` returns every
     * namespace's pods, so a map keyed by name alone gives both `api-0`s the
     * reading of whichever arrived last — and the table sorts on it.
     */
    it("keys each pod's usage by namespace and name, so two namespaces' api-0 do not collide", async () => {
      podMetrics.mockResolvedValue({
        metrics: [
          { name: "api-0", namespace: "shop", cpuMillicores: 10, memoryMiB: 100 },
          { name: "api-0", namespace: "billing", cpuMillicores: 20, memoryMiB: 200 },
        ],
      });

      const out = await descriptorFor("pods")!.enrich!("prod", "");

      expect(out.size).toBe(2);
      expect(out.get(rowKey({ name: "api-0", namespace: "shop" }))).toEqual({ cpu: 10, memory: 100 });
      expect(out.get(rowKey({ name: "api-0", namespace: "billing" }))).toEqual({ cpu: 20, memory: 200 });
    });

    it("lists no reading at all when the metrics call answers with nothing", async () => {
      podMetrics.mockResolvedValue({});
      expect((await descriptorFor("pods")!.enrich!("prod", "shop")).size).toBe(0);
    });
  });
});
