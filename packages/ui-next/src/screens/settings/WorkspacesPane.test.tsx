import { beforeEach, expect, it } from "vitest";
import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ClusterContext } from "@srelens/core";
import { setContexts } from "../../lib/clusters";
import { loadMarks, setMark } from "../../lib/marks";
import { defaultState } from "../../lib/tabs";
import { createWorkspace, getState, openTab, setActiveCluster, setState, setWorkspaceClusters, switchWorkspace } from "../../lib/tabsStore";
import { WorkspacesPane } from "./WorkspacesPane";

// jsdom has neither, and the add-cluster picker is a Radix popover over a cmdk
// list. Inert stubs: jsdom does no layout, so there is never a resize to report.
if (!("ResizeObserver" in globalThis)) {
  (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}
const proto = window.HTMLElement.prototype as unknown as Record<string, unknown>;
proto.scrollIntoView ??= () => {};
proto.hasPointerCapture ??= () => false;
proto.setPointerCapture ??= () => {};
proto.releasePointerCapture ??= () => {};

const contexts: ClusterContext[] = ["prod", "staging", "dev"].map((name) => ({
  name, stableId: `${name}-id`, key: `${name}-id`, cluster: name, server: `https://${name}`,
  sourceFile: "/config", authKind: "token", isCurrent: false,
}));

let team: string;
beforeEach(() => {
  localStorage.clear();
  loadMarks();
  setState(defaultState(contexts)); // Default: all three clusters, current
  setContexts(contexts);
  team = createWorkspace("Team", ["prod-id"]);
  openTab("/k/pods"); // Team: its home tab and this one
  switchWorkspace(getState().workspaces[0].id);
});

const row = (name: string) => screen.getByRole("listitem", { name });
const stored = (id: string) => getState().workspaces.find((w) => w.id === id);
const paint = () => {
  render(<WorkspacesPane />);
  return userEvent.setup();
};

it("lists every workspace and marks the current one", () => {
  paint();
  expect(row("Default").getAttribute("aria-current")).toBe("true");
  expect(within(row("Default")).getByText("Current")).toBeTruthy();
  expect(row("Team").getAttribute("aria-current")).toBeNull();
  expect(within(row("Team")).queryByText("Current")).toBeNull();
});

it("renames a workspace on Enter", async () => {
  const user = paint();
  const name = screen.getByRole("textbox", { name: "Name of Team" });
  await user.clear(name);
  await user.type(name, "Platform{Enter}");
  expect(stored(team)?.name).toBe("Platform");
});

it("renames a workspace when the field loses focus, and not while typing", async () => {
  const user = paint();
  const name = screen.getByRole("textbox", { name: "Name of Team" });
  await user.clear(name);
  await user.type(name, "Platform");
  expect(stored(team)?.name).toBe("Team");
  await user.tab();
  expect(stored(team)?.name).toBe("Platform");
});

it("keeps the old name when the new one is blank", async () => {
  const user = paint();
  const name = screen.getByRole("textbox", { name: "Name of Team" }) as HTMLInputElement;
  await user.clear(name);
  await user.type(name, "   {Enter}");
  expect(stored(team)?.name).toBe("Team");
  expect(name.value).toBe("Team");
});

it("sets and clears a workspace's colour", async () => {
  const user = paint();
  const group = screen.getByRole("radiogroup", { name: "Colour of Team" });
  expect((within(group).getByRole("radio", { name: "None" }) as HTMLInputElement).checked).toBe(true);
  await user.click(within(group).getByRole("radio", { name: "Teal" }));
  expect(stored(team)?.color).toBe("var(--mark-teal)");
  await user.click(within(group).getByRole("radio", { name: "None" }));
  expect(stored(team)).not.toHaveProperty("color");
});

/** The options the add-cluster picker offers, by name, once it is open. */
const offered = () => screen.getAllByRole("option").map((o) => o.getAttribute("aria-label"));

it("offers only the clusters a workspace does not have, searchably, and adds the one picked", async () => {
  const user = paint();
  await user.click(screen.getByRole("combobox", { name: "Add a cluster to Team" }));
  expect(offered()).toEqual(["staging", "dev"]);
  // Contexts can number in the hundreds, so they are searched (AGENTS.md, UI).
  await user.type(screen.getByPlaceholderText("Search clusters…"), "de");
  expect(offered()).toEqual(["dev"]);
  await user.click(screen.getByRole("option", { name: "dev" }));
  expect(stored(team)?.clusters).toEqual(["prod-id", "dev-id"]);
});

it("offers nothing to add when every cluster is already in", () => {
  paint();
  expect(within(row("Default")).queryByRole("combobox")).toBeNull();
  expect(within(row("Default")).getByText("Every cluster is in")).toBeTruthy();
});

it("removes a cluster from a workspace", async () => {
  const user = paint();
  expect(within(screen.getByRole("list", { name: "Clusters in Team" })).getByText("prod")).toBeTruthy();
  await user.click(screen.getByRole("button", { name: "Remove prod from Team" }));
  expect(stored(team)?.clusters).toEqual([]);
});

it("names a workspace's clusters by their mark, and follows a mark renamed while it is open", async () => {
  // A mark name that is not the context name, so a label read from the context fails.
  const mark = (name: string) => ({ name, short: name.slice(0, 2), color: "#123456", mark: "text" as const, withText: true });
  setMark("prod-id", mark("Production"));
  setMark("staging-id", mark("Pre-prod"));
  const user = paint();
  const chips = within(screen.getByRole("list", { name: "Clusters in Team" }));
  expect(chips.getByText("Production")).toBeTruthy();
  expect(screen.getByRole("button", { name: "Remove Production from Team" })).toBeTruthy();
  await user.click(screen.getByRole("combobox", { name: "Add a cluster to Team" }));
  expect(offered()).toEqual(["Pre-prod", "dev"]);
  await user.keyboard("{Escape}");
  // Renamed with the pane mounted: nothing in the workspace store changes, so only the mark subscription redraws it.
  act(() => setMark("prod-id", mark("Prod EU")));
  expect(chips.getByText("Prod EU")).toBeTruthy();
  expect(chips.queryByText("Production")).toBeNull();
  expect(screen.getByRole("button", { name: "Remove Prod EU from Team" })).toBeTruthy();
});

it("deletes a workspace after saying what it holds", async () => {
  const user = paint();
  await user.click(screen.getByRole("button", { name: "Delete Team" }));
  const dialog = screen.getByRole("dialog", { name: "Delete Team?" });
  expect(dialog.textContent).toContain("1 cluster and 2 open tabs");
  await user.click(within(dialog).getByRole("button", { name: "Delete workspace" }));
  expect(stored(team)).toBeUndefined();
});

it("counts the tabs of every cluster in the workspace, not only the one in focus", async () => {
  // Team's Pods were opened on prod. Focusing staging puts them away and opens
  // staging's overview: two tabs on screen, one put away, and all three go.
  setWorkspaceClusters(team, ["prod-id", "staging-id"]);
  switchWorkspace(team);
  setActiveCluster("staging-id", "staging");
  expect(stored(team)?.shelved?.["prod-id"].tabs).toHaveLength(1);
  switchWorkspace(getState().workspaces[0].id);
  const user = paint();
  await user.click(screen.getByRole("button", { name: "Delete Team" }));
  expect(screen.getByRole("dialog", { name: "Delete Team?" }).textContent).toContain("2 clusters and 3 open tabs");
});

it("keeps the workspace when the delete is cancelled", async () => {
  const user = paint();
  await user.click(screen.getByRole("button", { name: "Delete Team" }));
  await user.click(within(screen.getByRole("dialog", { name: "Delete Team?" })).getByRole("button", { name: "Cancel" }));
  expect(stored(team)).toBeDefined();
});

it("does not offer to delete the last workspace", () => {
  setState(defaultState(contexts));
  paint();
  expect((screen.getByRole("button", { name: "Delete Default" }) as HTMLButtonElement).disabled).toBe(true);
});
