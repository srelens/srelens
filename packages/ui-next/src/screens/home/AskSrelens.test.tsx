import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ClusterContext } from "@srelens/core";
import { ConsoleProvider, useConsole } from "../../console";
import type { AttentionItem, ClusterAttention } from "../../lib/attention";
import { resetContexts, setContexts } from "../../lib/clusters";
import { defaultState } from "../../lib/tabs";
import { activeCluster, activeRoute, setState } from "../../lib/tabsStore";
import { AskSrelens } from "./AskSrelens";

const { isTauri } = vi.hoisted(() => ({ isTauri: vi.fn(() => true) }));
vi.mock("@srelens/core", async (original) => ({ ...(await original<typeof import("@srelens/core")>()), isTauri }));

const ctx = (stableId: string, name: string): ClusterContext => ({
  stableId, key: stableId, name, cluster: name, server: `https://${stableId}.example`, isCurrent: false,
  sourceFile: "/mock/config", authKind: "client certificate",
});
const PROD = ctx("prod-id", "prod");
const STAGE = ctx("stage-id", "staging");
const item = (cluster: ClusterContext, over: Partial<AttentionItem>): AttentionItem => ({
  clusterId: cluster.stableId, cluster: cluster.name, kind: "Pod", namespace: "checkout", name: "web-7d4b",
  problem: "CrashLoopBackOff", cause: "crash", ...over,
});
const scan = (items: AttentionItem[]): ClusterAttention => ({ items, failures: [], truncated: false, eventsTruncated: false });

function Draft() {
  return <output aria-label="Assistant draft">{useConsole().draft}</output>;
}
function show(targets: ClusterContext[] = [], scans: Record<string, ClusterAttention> = {}) {
  return render(<ConsoleProvider><AskSrelens targets={targets} scans={scans} /><Draft /></ConsoleProvider>);
}
const draft = () => screen.getByRole("status", { name: "Assistant draft" }).textContent;

beforeEach(() => {
  isTauri.mockReturnValue(true);
  resetContexts();
  setContexts([PROD, STAGE]);
  setState(defaultState([PROD, STAGE]));
});

describe("AskSrelens", () => {
  it("opens the assistant with what was typed", async () => {
    show();
    expect(screen.getByRole("heading", { name: "Ask srelens", level: 2 })).toBeTruthy();
    expect((screen.getByRole("button", { name: "Ask" }) as HTMLButtonElement).disabled).toBe(true);
    await userEvent.type(screen.getByRole("textbox", { name: "Ask srelens" }), "Which deployments changed today?{Enter}");
    expect(activeRoute()).toBe("/agent");
    expect(activeCluster()).toBe(PROD.stableId);
    expect(draft()).toBe("Which deployments changed today?");
  });

  it("suggests up to three questions from what needs attention, worst first, each on its own cluster", async () => {
    show([PROD, STAGE], {
      [PROD.stableId]: scan([
        item(PROD, { kind: "Service", name: "front", problem: "FailedToUpdateEndpoint", cause: "warning" }),
        item(PROD, { kind: "Deployment", name: "api", problem: "1/3 ready", cause: "replicas" }),
      ]),
      [STAGE.stableId]: scan([item(STAGE, {}), item(STAGE, { name: "web-9f1c" })]),
    });
    const suggested = screen.getAllByRole("button", { name: /^Ask: / });
    expect(suggested.map((b) => b.textContent)).toEqual([
      "Why is checkout/web-7d4b crash-looping on staging?",
      "Why is checkout/web-9f1c crash-looping on staging?",
      "Why does Deployment checkout/api have unavailable replicas on prod?",
    ]);
    await userEvent.click(suggested[0]);
    expect(activeRoute()).toBe("/agent");
    expect(activeCluster()).toBe(STAGE.stableId);
    expect(draft()).toBe("Why is checkout/web-7d4b crash-looping on staging?");
  });

  it("suggests nothing when nothing needs attention", () => {
    show([PROD], { [PROD.stableId]: scan([]) });
    expect(screen.queryByRole("button", { name: /^Ask: / })).toBeNull();
  });

  it("is not drawn on the web host, where the assistant cannot run", () => {
    isTauri.mockReturnValue(false);
    show();
    expect(screen.queryByRole("heading", { name: "Ask srelens" })).toBeNull();
  });
});
