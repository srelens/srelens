import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ClusterContext } from "@srelens/core";
import { resetContexts, setContexts } from "../../lib/clusters";
import { loadRecentLogSubjects, rememberLogSubject } from "../../lib/logRecents";
import { loadMarks } from "../../lib/marks";
import { defaultState } from "../../lib/tabs";
import { activeCluster, activeRoute, closeTab, currentWorkspace, openTab, setActiveCluster, setState } from "../../lib/tabsStore";
import { logsRoute } from "../Logs";
import { PickUp } from "./PickUp";

const { listResource } = vi.hoisted(() => ({ listResource: vi.fn() }));
vi.mock("@srelens/core", async (original) => ({ ...(await original<typeof import("@srelens/core")>()), listResource }));

const ctx = (stableId: string, name: string): ClusterContext => ({
  stableId, key: stableId, name, cluster: name, server: `https://${stableId}.example`, isCurrent: false,
  sourceFile: "/mock/config", authKind: "client certificate",
});
const PROD = ctx("prod-id", "prod");
const STAGE = ctx("stage-id", "staging");

function memory() {
  const held = new Map<string, string>();
  return { getItem: (k: string) => held.get(k) ?? null, setItem: (k: string, v: string) => void held.set(k, v), removeItem: (k: string) => void held.delete(k) };
}

/** Open `route` on `cluster`, then close it, so it is in the closed stack labelled for that cluster. */
function closeOn(route: string, cluster?: ClusterContext) {
  if (cluster) setActiveCluster(cluster.stableId, cluster.name);
  openTab(route, { clusterName: cluster?.name });
  closeTab(currentWorkspace().activeId);
}

beforeEach(() => {
  listResource.mockReset().mockResolvedValue({ items: [] });
  localStorage.clear(); loadMarks(); resetContexts();
  loadRecentLogSubjects(memory());
  setContexts([PROD, STAGE]);
  setState(defaultState([PROD, STAGE]));
});

describe("PickUp", () => {
  it("reopens a recently closed tab on the cluster it was about", async () => {
    closeOn("/events", STAGE);
    setActiveCluster(PROD.stableId, PROD.name);
    render(<PickUp targets={[PROD, STAGE]} />);
    await userEvent.click(screen.getByRole("button", { name: "Reopen Events on staging" }));
    expect(activeCluster()).toBe(STAGE.stableId);
    expect(activeRoute()).toBe("/events");
    expect(currentWorkspace().closed).toEqual([]);
  });

  it("reopens an app-wide tab as it was, and leaves out a tab whose cluster is gone", async () => {
    closeOn("/settings");
    closeOn("/helm", ctx("gone-id", "decommissioned"));
    render(<PickUp targets={[PROD]} />);
    // Not even as an app-wide "Reopen Helm": reopened, it would read whichever cluster is in focus.
    expect(screen.queryByRole("button", { name: /^Reopen Helm/ })).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Reopen Settings" }));
    expect(activeRoute()).toBe("/settings");
  });

  it("offers a route closed twice once, as its most recent tab", async () => {
    closeOn("/settings");
    closeOn("/events", STAGE);
    closeOn("/settings");
    render(<PickUp targets={[PROD, STAGE]} />);
    expect(screen.getAllByRole("button", { name: /^Reopen / }).map((b) => b.getAttribute("aria-label")))
      .toEqual(["Reopen Settings", "Reopen Events on staging"]);
    await userEvent.click(screen.getByRole("button", { name: "Reopen Settings" }));
    expect(currentWorkspace().closed.map((t) => t.route)).toEqual(["/events", "/settings"]);
  });

  it("offers logs followed on any connected cluster, checked first, and follows each on its own cluster", async () => {
    const storage = memory();
    rememberLogSubject({ cluster: STAGE.stableId, kind: "Deployment", namespace: "checkout", name: "web" }, storage);
    rememberLogSubject({ cluster: PROD.stableId, kind: "Deployment", namespace: "billing", name: "api" }, storage);
    listResource.mockImplementation(async (context: string) => ({ items: context === "staging" ? [{ name: "web" }] : [] }));
    render(<PickUp targets={[PROD, STAGE]} />);
    const follow = await screen.findByRole("button", { name: "Follow logs of Deployment checkout/web on staging" });
    expect((screen.getByRole("button", { name: "Follow logs of Deployment billing/api on prod" }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByText("no longer on this cluster")).toBeTruthy();
    await userEvent.click(follow);
    expect(activeCluster()).toBe(STAGE.stableId);
    expect(activeRoute()).toBe(logsRoute("Deployment", "checkout", "web"));
  });

  it("does not read a cluster it was not given, so a paused cluster's logs are not checked", async () => {
    rememberLogSubject({ cluster: STAGE.stableId, kind: "Deployment", namespace: "checkout", name: "web" }, memory());
    render(<PickUp targets={[PROD]} />);
    await act(async () => { await Promise.resolve(); });
    expect(listResource).not.toHaveBeenCalled();
    expect(screen.queryByRole("button", { name: /^Follow logs/ })).toBeNull();
  });

  it("checks no followed logs while Home is paused, and checks them once it is not", async () => {
    rememberLogSubject({ cluster: PROD.stableId, kind: "Deployment", namespace: "checkout", name: "web" }, memory());
    listResource.mockResolvedValue({ items: [{ name: "web" }] });
    const view = render(<PickUp targets={[PROD]} paused />);
    await act(async () => { await Promise.resolve(); });
    expect(listResource).not.toHaveBeenCalled();
    view.rerender(<PickUp targets={[PROD]} paused={false} />);
    expect(await screen.findByRole("button", { name: "Follow logs of Deployment checkout/web on prod" })).toBeTruthy();
    expect(listResource).toHaveBeenCalledTimes(1);
  });

  it("asks again for a check a pause cut off, once Home resumes", async () => {
    rememberLogSubject({ cluster: PROD.stableId, kind: "Deployment", namespace: "checkout", name: "web" }, memory());
    let answer!: (value: unknown) => void;
    listResource.mockImplementationOnce(() => new Promise((done) => { answer = done; })).mockResolvedValue({ items: [{ name: "web" }] });
    const view = render(<PickUp targets={[PROD]} paused={false} />);
    await act(async () => { await Promise.resolve(); });
    view.rerender(<PickUp targets={[PROD]} paused />);
    await act(async () => { answer({ items: [{ name: "web" }] }); await Promise.resolve(); });
    view.rerender(<PickUp targets={[PROD]} paused={false} />);
    expect(await screen.findByRole("button", { name: "Follow logs of Deployment checkout/web on prod" })).toBeTruthy();
    expect(listResource).toHaveBeenCalledTimes(2);
  });

  it("says there is nothing to pick up yet rather than drawing an empty list", () => {
    render(<PickUp targets={[PROD]} />);
    expect(screen.getByRole("heading", { name: "Pick up where you left off", level: 2 })).toBeTruthy();
    expect(screen.getByText("Nothing to pick up yet")).toBeTruthy();
  });
});
