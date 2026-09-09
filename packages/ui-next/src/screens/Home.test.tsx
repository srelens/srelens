import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { saveContextOrder, type ClusterContext } from "@srelens/core";
import { screenFor } from "../lib/routes";
import { Home } from "./Home";
import { resetContexts, setContexts } from "../lib/clusters";
import { defaultMark, loadMarks, setMark } from "../lib/marks";
import { activeCluster, activeRoute, currentWorkspace, setState } from "../lib/tabsStore";
import { defaultState } from "../lib/tabs";
import { resetView, setLink } from "../lib/workspace";

const ctx = (stableId: string, name: string): ClusterContext => ({
  stableId, name, cluster: name, server: `https://${stableId}.example`, isCurrent: false,
  sourceFile: "/mock/config", authKind: "client certificate",
});
const { listContexts } = vi.hoisted(() => ({ listContexts: vi.fn() }));
vi.mock("@srelens/core", async original => ({ ...(await original<typeof import("@srelens/core")>()), listContexts }));
const PROD = ctx("prod-id", "prod");
const STAGE = ctx("stage-id", "staging");
beforeEach(() => {
  listContexts.mockReset();
  localStorage.clear(); loadMarks(); resetContexts(); resetView();
  setState(defaultState([]));
});

describe("Home", () => {
  it("offers a working first step when no clusters are configured", async () => {
    setContexts([]);
    render(<Home />);
    expect(screen.getByRole("heading", { name: "Home", level: 1 })).toBeTruthy();
    expect(screen.getByText("Your Kubernetes workspace")).toBeTruthy();
    expect(screen.getByText("No clusters configured")).toBeTruthy();
    expect(screen.queryByText(/not in the new design/)).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Connect a cluster" }));
    expect(activeRoute()).toBe("/connect");
  });

  it("preserves cluster order and custom identities, and opens the chosen overview", async () => {
    setContexts([PROD, STAGE]);
    saveContextOrder([STAGE.stableId, PROD.stableId]);
    setMark(PROD.stableId, { ...defaultMark(PROD.name), name: "Production Europe", short: "PE" });
    setLink(PROD.stableId, "connected");
    render(<Home />);
    expect(screen.getAllByRole("button", { name: /^Open cluster / }).map(row => row.getAttribute("aria-label")))
      .toEqual(["Open cluster staging", "Open cluster Production Europe"]);
    expect(screen.getByText("PE")).toBeTruthy();
    expect(screen.getByText("Connected")).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: "Open cluster Production Europe" }));
    expect(activeCluster()).toBe(PROD.stableId);
    expect(currentWorkspace().clusters).toContain(PROD.stableId);
    expect(activeRoute()).toBe("/overview");
  });

  it("filters by custom names and original context names without discarding the list", async () => {
    setContexts([PROD, STAGE]);
    setMark(PROD.stableId, { ...defaultMark(PROD.name), name: "Europe" });
    render(<Home />);
    const filter = screen.getByRole("searchbox", { name: "Find a cluster" });
    await userEvent.type(filter, "prod");
    expect(screen.getByRole("button", { name: "Open cluster Europe" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Open cluster staging" })).toBeNull();
    await userEvent.clear(filter);
    await userEvent.type(filter, "missing");
    expect(screen.getByText("No matching clusters")).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: "Clear search" }));
    expect(screen.getAllByRole("button", { name: /^Open cluster / })).toHaveLength(2);
    act(() => setMark(PROD.stableId, { ...defaultMark(PROD.name), name: "Production renamed" }));
    expect(screen.getByRole("button", { name: "Open cluster Production renamed" })).toBeTruthy();
  });

  it("distinguishes loading and failed discovery from an empty configuration", () => {
    const view = render(<Home />);
    expect(screen.getByText("Loading clusters…")).toBeTruthy();
    expect(screen.queryByText("No clusters configured")).toBeNull();
    act(() => setContexts([], "kubeconfig unreadable"));
    view.rerender(<Home />);
    expect(screen.getByText("Could not load all clusters")).toBeTruthy();
    expect(screen.queryByText("No clusters configured")).toBeNull();
  });

  it("keeps readable contexts available after a partial discovery failure", () => {
    setContexts([PROD], "another kubeconfig is unreadable");
    setLink(PROD.stableId, "error", "connection refused");
    render(<Home />);
    expect(screen.getByText("Could not load all clusters")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Open cluster prod" })).toBeTruthy();
    expect(screen.getByText("Unreachable")).toBeTruthy();
  });

  it.each([["Manage connections", "/connections"], ["Settings", "/settings"], ["Release notes", "/notes"]])(
    "opens %s", async (name, route) => {
      setContexts([]);
      render(<Home />);
      await userEvent.click(screen.getByRole("button", { name }));
      expect(activeRoute()).toBe(route);
      expect(screenFor(activeRoute())).not.toBeNull();
    },
  );
});

it("retries a failed discovery and retains readable clusters if retry fails", async () => {
  setContexts([PROD], "source unavailable");
  listContexts.mockRejectedValueOnce(Error("still unavailable")).mockResolvedValueOnce({ contexts: [PROD, STAGE] });
  render(<Home />);
  await userEvent.click(screen.getByRole("button", { name: "Retry" }));
  expect(screen.getByRole("button", { name: "Open cluster prod" })).toBeTruthy();
  expect(screen.getByText("Could not load all clusters")).toBeTruthy();
  await userEvent.click(screen.getByRole("button", { name: "Retry" }));
  expect(screen.getByRole("button", { name: "Open cluster staging" })).toBeTruthy();
  expect(screen.queryByText("Could not load all clusters")).toBeNull();
});

it("does not let a delayed retry replace a newer context inventory", async () => {
  setContexts([PROD], "source unavailable");
  let resolve!: (value: unknown) => void;
  listContexts.mockReturnValue(new Promise(done => { resolve = done; }));
  render(<Home />);
  await userEvent.click(screen.getByRole("button", { name: "Retry" }));
  act(() => setContexts([STAGE]));
  await act(async () => resolve({ contexts: [PROD] }));
  expect(screen.getByRole("button", { name: "Open cluster staging" })).toBeTruthy();
  expect(screen.queryByRole("button", { name: "Open cluster prod" })).toBeNull();
});
