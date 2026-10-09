import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ClusterContext } from "@srelens/core";
import { ConsoleProvider, useConsole } from "../../console";
import { resetContexts, setContexts } from "../../lib/clusters";
import { detailRoute } from "../../lib/detailRoute";
import { loadMarks } from "../../lib/marks";
import { defaultState } from "../../lib/tabs";
import { activeCluster, activeRoute, setState } from "../../lib/tabsStore";
import type { AttentionItem, ClusterAttention } from "../../lib/attention";
import { NeedsAttention } from "./NeedsAttention";

const { isTauri } = vi.hoisted(() => ({ isTauri: vi.fn(() => true) }));
vi.mock("@srelens/core", async (original) => ({ ...(await original<typeof import("@srelens/core")>()), isTauri }));

const ctx = (stableId: string, name: string): ClusterContext => ({
  stableId, key: stableId, name, cluster: name, server: `https://${stableId}.example`, isCurrent: false,
  sourceFile: "/mock/config", authKind: "client certificate",
});
const PROD = ctx("prod-id", "prod");
const STAGE = ctx("stage-id", "staging");

const item = (cluster: ClusterContext, over: Partial<AttentionItem> = {}): AttentionItem => ({
  clusterId: cluster.stableId, cluster: cluster.name, kind: "Pod", namespace: "checkout", name: "web-7d4b",
  problem: "CrashLoopBackOff", cause: "crash", ...over,
});
const answered = (items: AttentionItem[], failures: string[] = []): ClusterAttention => ({ items, failures, truncated: false });

function Draft() {
  return <output aria-label="Assistant draft">{useConsole().draft}</output>;
}
function show(targets: ClusterContext[], scans: Record<string, ClusterAttention>) {
  return render(
    <ConsoleProvider>
      <NeedsAttention targets={targets} scans={scans} />
      <Draft />
    </ConsoleProvider>,
  );
}

beforeEach(() => {
  isTauri.mockReturnValue(true);
  localStorage.clear(); loadMarks(); resetContexts();
  setContexts([PROD, STAGE]);
  setState(defaultState([PROD, STAGE]));
});

describe("NeedsAttention", () => {
  it("names each item's cluster and opens its detail on that cluster", async () => {
    show([PROD, STAGE], {
      [PROD.stableId]: answered([item(PROD)]),
      [STAGE.stableId]: answered([item(STAGE, { kind: "Deployment", name: "api", problem: "1/3 ready", cause: "replicas" })]),
    });
    expect(screen.getByRole("heading", { name: /Needs attention/, level: 2 })).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: "Open Deployment checkout/api on staging" }));
    expect(activeCluster()).toBe(STAGE.stableId);
    expect(activeRoute()).toBe(detailRoute("Deployment", "checkout", "api"));
  });

  it("opens the assistant on the item's cluster with its question filled in", async () => {
    setState(defaultState([STAGE, PROD]));
    show([PROD], { [PROD.stableId]: answered([item(PROD)]) });
    await userEvent.click(screen.getByRole("button", { name: "Ask about checkout/web-7d4b on prod" }));
    expect(activeRoute()).toBe("/agent");
    expect(activeCluster()).toBe(PROD.stableId);
    expect(screen.getByRole("status", { name: "Assistant draft" }).textContent).toBe("Why is checkout/web-7d4b crash-looping on prod?");
  });

  it("opens a cluster-scoped object by its name alone", async () => {
    show([PROD], { [PROD.stableId]: answered([item(PROD, { kind: "Node", namespace: "", name: "n1", problem: "Rebooted", cause: "warning" })]) });
    await userEvent.click(screen.getByRole("button", { name: "Open Node n1 on prod" }));
    expect(activeRoute()).toBe(detailRoute("Node", null, "n1"));
  });

  it("offers no Ask on the web host, where the assistant cannot run", () => {
    isTauri.mockReturnValue(false);
    show([PROD], { [PROD.stableId]: answered([item(PROD)]) });
    expect(screen.getByRole("button", { name: "Open Pod checkout/web-7d4b on prod" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: /^Ask about/ })).toBeNull();
  });

  it("says a cluster could not be checked, and never calls the workspace all clear", () => {
    show([PROD, STAGE], {
      [PROD.stableId]: answered([], ["events is forbidden: User \"dana\" cannot list resource \"events\" in API group \"\" at the cluster scope"]),
      [STAGE.stableId]: answered([]),
    });
    expect(screen.getByText("Could not check everything on prod")).toBeTruthy();
    expect(screen.queryByText("Nothing needs attention")).toBeNull();
  });

  it("says nothing needs attention only once every cluster has answered", () => {
    const view = show([PROD, STAGE], { [PROD.stableId]: answered([]) });
    expect(screen.queryByText("Nothing needs attention")).toBeNull();
    expect(screen.getByText("Still checking 1 cluster")).toBeTruthy();
    view.rerender(
      <ConsoleProvider>
        <NeedsAttention targets={[PROD, STAGE]} scans={{ [PROD.stableId]: answered([]), [STAGE.stableId]: answered([]) }} />
      </ConsoleProvider>,
    );
    expect(screen.getByText("Nothing needs attention")).toBeTruthy();
  });

  it("is checking, not empty, before any cluster answers", () => {
    show([PROD], {});
    expect(screen.getByText("Checking 1 cluster…")).toBeTruthy();
    expect(screen.queryByText("Nothing needs attention")).toBeNull();
  });

  it("says when there is no connected cluster to check", () => {
    show([], {});
    expect(screen.getByText("No connected clusters to check")).toBeTruthy();
    expect(screen.queryByText("Nothing needs attention")).toBeNull();
  });

  it("says a pod list that stopped short is short", () => {
    show([PROD], { [PROD.stableId]: { items: [], failures: [], truncated: true } });
    expect(screen.getByText("More pods on prod may need a look than this shows")).toBeTruthy();
    expect(screen.queryByText("Nothing needs attention")).toBeNull();
  });

  it("puts broken pods first and shows a long list in part until asked for all of it", async () => {
    const warnings = Array.from({ length: 11 }, (_, i) => item(PROD, { kind: "Service", name: `svc-${i}`, problem: "FailedToUpdateEndpoint", cause: "warning" }));
    show([PROD], { [PROD.stableId]: answered([...warnings, item(PROD)]) });
    const rows = () => screen.getAllByRole("button", { name: /^Open / });
    expect(rows()).toHaveLength(10);
    expect(rows()[0].getAttribute("aria-label")).toBe("Open Pod checkout/web-7d4b on prod");
    await userEvent.click(screen.getByRole("button", { name: "Show all 12" }));
    expect(rows()).toHaveLength(12);
  });
});
