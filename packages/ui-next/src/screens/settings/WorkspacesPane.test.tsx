import { beforeEach, expect, it } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ClusterContext } from "@srelens/core";
import { setContexts } from "../../lib/clusters";
import { loadMarks } from "../../lib/marks";
import { defaultState } from "../../lib/tabs";
import { createWorkspace, getState, openTab, setState, switchWorkspace } from "../../lib/tabsStore";
import { WorkspacesPane } from "./WorkspacesPane";

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

it("offers only the clusters a workspace does not have, and adds the one picked", async () => {
  const user = paint();
  const picker = screen.getByRole("combobox", { name: "Add a cluster to Team" }) as HTMLSelectElement;
  const offered = [...picker.options].filter((o) => !o.disabled).map((o) => o.textContent);
  expect(offered).toEqual(["staging", "dev"]);
  await user.selectOptions(picker, "dev-id");
  expect(stored(team)?.clusters).toEqual(["prod-id", "dev-id"]);
});

it("disables the picker when every cluster is already in", () => {
  paint();
  expect((screen.getByRole("combobox", { name: "Add a cluster to Default" }) as HTMLSelectElement).disabled).toBe(true);
});

it("removes a cluster from a workspace", async () => {
  const user = paint();
  expect(within(screen.getByRole("list", { name: "Clusters in Team" })).getByText("prod")).toBeTruthy();
  await user.click(screen.getByRole("button", { name: "Remove prod from Team" }));
  expect(stored(team)?.clusters).toEqual([]);
});

it("deletes a workspace after saying what it holds", async () => {
  const user = paint();
  await user.click(screen.getByRole("button", { name: "Delete Team" }));
  const dialog = screen.getByRole("dialog", { name: "Delete Team?" });
  expect(dialog.textContent).toContain("1 cluster and 2 open tabs");
  await user.click(within(dialog).getByRole("button", { name: "Delete workspace" }));
  expect(stored(team)).toBeUndefined();
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
