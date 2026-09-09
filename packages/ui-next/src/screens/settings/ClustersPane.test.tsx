import { beforeEach, expect, it, vi } from "vitest";
import { render, screen, within, fireEvent } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { loadClusterNamespaces, loadContextProfiles, loadContextOrder, saveClusterNamespaces, saveContextProfiles, saveContextOrder, type ClusterContext } from "@srelens/core";
import { setContexts } from "../../lib/clusters";
import { loadMarks } from "../../lib/marks";
import { ClustersPane } from "./ClustersPane";
import { activeCluster, setState } from "../../lib/tabsStore";
import { defaultState } from "../../lib/tabs";
import { getView, loadNamespaces, setNamespaces } from "../../lib/workspace";
const backend = vi.hoisted(() => ({ deleteContext: vi.fn(), listContexts: vi.fn(), isTauri: vi.fn(() => true) }));
vi.mock("@srelens/core", async (original) => ({ ...await original<object>(), ...backend }));
const contexts: ClusterContext[] = ["prod", "staging"].map(name => ({ name, stableId: name + "-id", cluster: name, server: "https://" + name, sourceFile: "/config", authKind: "token", isCurrent: false }));
beforeEach(() => {
  localStorage.clear(); vi.clearAllMocks(); backend.isTauri.mockReturnValue(true);
  loadNamespaces();
  saveContextProfiles({ "prod-id": { displayName: "Production Europe", shortName: "PE", logo: "cloud", color: "#123456" } });
  saveContextOrder(["staging-id", "prod-id"]); loadMarks(); setContexts(contexts);
});
it("edits classic identity and orders contexts without changing their real names", async () => {
  const user = userEvent.setup(); render(<ClustersPane />);
  expect(screen.getAllByRole("listitem").map(row => row.getAttribute("data-context"))).toEqual(["staging", "prod"]);
  await user.click(screen.getByRole("button", { name: "Edit Production Europe" }));
  expect((screen.getByLabelText("Display name") as HTMLInputElement).value).toBe("Production Europe");
  expect((screen.getByLabelText(/Short text/) as HTMLInputElement).value).toBe("PE");
  expect((screen.getByRole("radio", {name: "Cloud"}) as HTMLInputElement).checked).toBe(true);
  await user.clear(screen.getByLabelText("Display name"));
  await user.type(screen.getByLabelText("Display name"), "Production Europe with a long descriptive name");
  expect(loadContextProfiles()["prod-id"].displayName).toBe("Production Europe with a long descriptive name");
  await user.clear(screen.getByLabelText(/Short text/));
  await user.type(screen.getByLabelText(/Short text/), "EU");
  expect(loadContextProfiles()["prod-id"].shortName).toBe("EU");
  const handle = screen.getByRole("button", { name: "Drag Production Europe with a long descriptive name to reorder" });
  handle.focus();
  await user.keyboard("{ArrowUp}");
  expect(screen.queryByRole("button", { name: /^Move .* (up|down)$/ })).toBeNull();
  expect(loadContextOrder()).toEqual(["prod-id", "staging-id"]);
  expect(screen.getByText("https://prod")).toBeTruthy();
  await user.click(screen.getByRole("button", {name: "Reset"}));
  expect(loadContextProfiles()["prod-id"]).toBeUndefined();
});
it("confirms removal and sends the raw context name to the backend", async () => {
  backend.deleteContext.mockResolvedValue({ success: true });
  backend.listContexts.mockResolvedValue({ contexts: [contexts[1]] });
  const user = userEvent.setup(); render(<ClustersPane />);
  await user.click(screen.getByRole("button", {name: "Edit Production Europe"}));
  await user.click(screen.getByRole("button", {name: "Remove context"}));
  expect(backend.deleteContext).not.toHaveBeenCalled();
  await user.click(within(screen.getByRole("dialog")).getByRole("button", {name: "Remove context"}));
  expect(backend.deleteContext).toHaveBeenCalledWith("prod");
  expect(await screen.findByRole("button", {name: "Edit staging"})).toBeTruthy();
  expect(screen.queryByRole("button", {name: "Edit Production Europe"})).toBeNull();
});
it("clears both namespace stores after confirmed removal", async () => {
  backend.deleteContext.mockResolvedValue({ success: true });
  backend.listContexts.mockResolvedValue({ contexts: [contexts[1]] });
  saveClusterNamespaces({ "prod-id": "payments", "staging-id": "default" });
  setNamespaces("prod-id", ["payments"]);
  setNamespaces("staging-id", ["default"]);
  const user = userEvent.setup(); render(<ClustersPane />);
  await user.click(screen.getByRole("button", {name: "Edit Production Europe"}));
  await user.click(screen.getByRole("button", {name: "Remove context"}));
  await user.click(within(screen.getByRole("dialog")).getByRole("button", {name: "Remove context"}));

  expect(loadClusterNamespaces()).toEqual({ "staging-id": "default" });
  expect(getView().namespaces).toEqual({ "staging-id": ["default"] });
});
it("keeps identity and shows the reason when removal fails", async () => {
  backend.deleteContext.mockRejectedValue(new Error("permission denied"));
  const user = userEvent.setup(); render(<ClustersPane />);
  await user.click(screen.getByRole("button", {name: "Edit Production Europe"}));
  await user.click(screen.getByRole("button", {name: "Remove context"}));
  await user.click(within(screen.getByRole("dialog")).getByRole("button", {name: "Remove context"}));
  expect((await screen.findByRole("alert")).textContent).toMatch(/permission denied/i);
  expect(loadContextProfiles()["prod-id"].displayName).toBe("Production Europe");
});
it("retains the remaining contexts when relisting after removal fails", async () => {
  const added = { ...contexts[1], name: "new-context", stableId: "new-id" };
  backend.deleteContext.mockImplementation(async () => {
    setContexts([...contexts, added]);
    return { success: true };
  });
  backend.listContexts.mockResolvedValue({ error: "kubeconfig became unreadable" });
  setState(defaultState(contexts));
  setContexts(contexts, "previous listing failed");
  const user = userEvent.setup(); render(<ClustersPane />);
  await user.click(screen.getByRole("button", {name: "Edit Production Europe"}));
  await user.click(screen.getByRole("button", {name: "Remove context"}));
  await user.click(within(screen.getByRole("dialog")).getByRole("button", {name: "Remove context"}));

  expect(await screen.findByRole("button", {name: "Edit staging"})).toBeTruthy();
  expect(screen.getByRole("button", {name: "Edit new-context"})).toBeTruthy();
  expect(screen.queryByRole("button", {name: "Edit Production Europe"})).toBeNull();
  expect(activeCluster()).toBe("staging-id");
  expect(loadContextOrder()).toEqual(["staging-id"]);
  expect(screen.getByRole("alert").textContent).toMatch(/kubeconfig became unreadable/);
  await user.click(screen.getByRole("button", { name: "Retry" }));
  expect(await screen.findByRole("button", {name: "Edit staging"})).toBeTruthy();
  expect(screen.queryByRole("button", {name: "Edit Production Europe"})).toBeNull();
});
it("does not let a post-removal relist overwrite a concurrent context update", async () => {
  const added = { ...contexts[1], name: "new-context", stableId: "new-id" };
  backend.deleteContext.mockResolvedValue({ success: true });
  backend.listContexts.mockImplementation(async () => {
    setContexts([contexts[1], added]);
    return { contexts: [contexts[1]] };
  });
  const user = userEvent.setup(); render(<ClustersPane />);
  await user.click(screen.getByRole("button", {name: "Edit Production Europe"}));
  await user.click(screen.getByRole("button", {name: "Remove context"}));
  await user.click(within(screen.getByRole("dialog")).getByRole("button", {name: "Remove context"}));

  expect(await screen.findByRole("button", {name: "Edit new-context"})).toBeTruthy();
  expect(screen.queryByRole("button", {name: "Edit Production Europe"})).toBeNull();
});
it("filters by saved short name and distinguishes list failure from an empty list", async () => {
  const user = userEvent.setup();
  const view = render(<ClustersPane />);
  await user.type(screen.getByLabelText("Filter contexts"), "PE");
  expect(screen.getAllByRole("listitem")).toHaveLength(1);
  view.unmount();
  setContexts([], "kubeconfig permission denied");
  const added = { ...contexts[1], name: "new-context", stableId: "new-id" };
  backend.listContexts.mockImplementation(async () => {
    setContexts([...contexts, added]);
    return { contexts };
  });
  render(<ClustersPane />);
  expect(screen.getByRole("alert").textContent).toMatch(/permission denied/);
  expect(screen.queryByText(/No contexts configured/)).toBeNull();
  await user.click(screen.getByRole("button", { name: "Retry" }));
  expect(await screen.findByRole("button", { name: "Edit staging" })).toBeTruthy();
  expect(screen.getByRole("button", { name: "Edit new-context" })).toBeTruthy();
});

it("reorders through the pointer drag handle and cancels without saving", async () => {
  const user = userEvent.setup(); render(<ClustersPane />);
  const handle = screen.getByRole("button", { name: "Drag staging to reorder" });
  const target = screen.getByRole("button", { name: "Edit Production Europe" }).closest("li")!;
  const hitTest = vi.fn(() => target);
  Object.defineProperty(document, "elementFromPoint", { configurable: true, value: hitTest });
  fireEvent.pointerDown(handle, { button: 0, pointerId: 1 });
  fireEvent.pointerMove(handle, { clientX: 10, clientY: 90, pointerId: 1 });
  fireEvent.pointerUp(handle, { pointerId: 1 });
  expect(loadContextOrder()).toEqual(["prod-id", "staging-id"]);
  const second = screen.getByRole("button", { name: "Drag Production Europe to reorder" });
  hitTest.mockReturnValue(handle.closest("li")!);
  fireEvent.pointerDown(second, { button: 0, pointerId: 2 });
  fireEvent.pointerMove(second, { clientX: 10, clientY: 90, pointerId: 2 });
  fireEvent.pointerCancel(second, { pointerId: 2 });
  expect(loadContextOrder()).toEqual(["prod-id", "staging-id"]);
  handle.focus();
  await user.keyboard("{ArrowUp}");
  expect(loadContextOrder()).toEqual(["staging-id", "prod-id"]);
});

it("keeps appearance editing available on web without offering kubeconfig deletion", async () => {
  backend.isTauri.mockReturnValue(false);
  const user = userEvent.setup(); render(<ClustersPane />);
  await user.click(screen.getByRole("button", { name: "Edit Production Europe" }));
  expect(screen.queryByRole("button", { name: "Remove context" })).toBeNull();
  await user.clear(screen.getByLabelText("Display name"));
  await user.type(screen.getByLabelText("Display name"), "Web production");
  expect(loadContextProfiles()["prod-id"].displayName).toBe("Web production");
  expect(backend.deleteContext).not.toHaveBeenCalled();
});
it("falls back to the context name in the list while retaining an editable blank name", async () => {
  const user = userEvent.setup(); render(<ClustersPane />);
  await user.click(screen.getByRole("button", { name: "Edit Production Europe" }));
  const input = screen.getByLabelText("Display name") as HTMLInputElement;
  await user.clear(input);
  expect(input.value).toBe("");
  expect(screen.getByRole("button", { name: "Edit prod" })).toBeTruthy();
  expect(screen.getByRole("button", { name: "Drag prod to reorder" })).toBeTruthy();
  await user.type(input, "  Production Europe  ");
  expect(input.value).toBe("  Production Europe  ");
  expect(screen.getByRole("button", { name: "Edit Production Europe" }).textContent).toContain("Production Europe");
});
