import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
const core = vi.hoisted(() => ({ isTauri: vi.fn(() => true), checkForUpdate: vi.fn(), installUpdate: vi.fn(), appVersion: vi.fn(), relaunchApp: vi.fn(), openExternal: vi.fn(), loadUpdateNotes: vi.fn(), updateRequestTimeout: vi.fn() }));
vi.mock("@srelens/core", async (orig) => ({ ...(await orig<typeof import("@srelens/core")>()), ...core }));
import { getDefaultNamespace, getRequestTimeoutSecs, loadWorkspaceLayout, loadRestoreSession, loadUpdateChannel, settingsStorage } from "@srelens/core";
import { WorkspacePane, KubernetesPane, ApplicationLogsPane } from "./PreferencePanes";
import { UpdatesPane } from "./UpdatesPane";
import { loadPeekWidth, peekWidth } from "../../lib/peekWidth";
import { resetView } from "../../lib/workspace";
import { openTab } from "../../lib/tabsStore";
vi.mock("../../lib/tabsStore", () => ({ openTab: vi.fn() }));
const update = { version: "2.0.0", currentVersion: "1.0.0", notes: "## Fixed\nA useful fix", external: false, elevates: false };
beforeEach(() => {
  vi.clearAllMocks(); localStorage.clear(); loadPeekWidth(); resetView();
  core.isTauri.mockReturnValue(true); core.appVersion.mockResolvedValue("1.0.0");
  core.loadUpdateNotes.mockResolvedValue(update.notes);
  core.checkForUpdate.mockResolvedValue(update); core.installUpdate.mockResolvedValue(undefined);
  core.openExternal.mockResolvedValue(undefined); core.relaunchApp.mockResolvedValue(undefined); core.updateRequestTimeout.mockResolvedValue(30);
});
it("persists restore-session and applies the detail width to the live store", async () => {
  render(<WorkspacePane />);
  await userEvent.click(screen.getByRole("switch", { name: "Reopen tabs on launch" }));
  expect(loadRestoreSession()).toBe(false);
  fireEvent.change(screen.getByLabelText("Resource detail width"), { target: { value: "480" } });
  expect(peekWidth()).toBe(480);
  fireEvent.change(screen.getByLabelText("Left navigation width"), { target: { value: "300" } });
  expect(loadWorkspaceLayout().leftSidebarWidth).toBe(300);
  loadPeekWidth(); expect(peekWidth()).toBe(480);
});
it("saves a default namespace and commits only a nonempty timeout draft", async () => {
  render(<KubernetesPane />);
  fireEvent.change(screen.getByLabelText("Default namespace"), { target: { value: "team" } });
  await userEvent.click(screen.getByRole("button", { name: "Save namespace" }));
  expect(getDefaultNamespace()).toBe("team");
  const timeout = screen.getByLabelText("Request timeout (seconds)");
  fireEvent.change(timeout, { target: { value: "" } });
  expect(screen.getByRole("button", { name: "Save timeout" }).hasAttribute("disabled")).toBe(true);
  expect(core.updateRequestTimeout).not.toHaveBeenCalled();
  fireEvent.change(timeout, { target: { value: "30" } });
  await userEvent.click(screen.getByRole("button", { name: "Save timeout" }));
  expect(core.updateRequestTimeout).toHaveBeenCalledWith(30);
});
it("shows a failed timeout save without claiming it was applied", async () => {
  core.updateRequestTimeout.mockRejectedValue(new Error("backend unavailable"));
  render(<KubernetesPane />);
  fireEvent.change(screen.getByLabelText("Request timeout (seconds)"), { target: { value: "30" } });
  await userEvent.click(screen.getByRole("button", { name: "Save timeout" }));
  expect((await screen.findByRole("alert")).textContent).toContain("Could not save request timeout");
  expect(getRequestTimeoutSecs()).toBe(8);
});
it("keeps desktop timeout controls off the web", () => {
  core.isTauri.mockReturnValue(false); render(<KubernetesPane />);
  expect(screen.queryByLabelText("Request timeout (seconds)")).toBeNull();
});
it("opens the existing application log screen", async () => {
  render(<ApplicationLogsPane />); await userEvent.click(screen.getByRole("button", { name: "Open application logs" }));
  expect(openTab).toHaveBeenCalledWith("/applog");
});
it("checks the selected channel, installs with progress, and restarts", async () => {
  let finish!: () => void;
  core.installUpdate.mockImplementation((_channel, progress) => { progress(42); return new Promise<void>(resolve => { finish = resolve; }); });
  render(<UpdatesPane />);
  expect(await screen.findByText("1.0.0")).toBeTruthy();
  fireEvent.change(screen.getByLabelText("Update channel"), { target: { value: "dev" } });
  expect(loadUpdateChannel()).toBe("dev");
  await userEvent.click(screen.getByRole("button", { name: "Check for updates" }));
  expect(core.checkForUpdate).toHaveBeenCalledWith("dev");
  await userEvent.click(await screen.findByRole("button", { name: "Download & install" }));
  expect((await screen.findByRole("status")).textContent).toContain("42%");
  expect((screen.getByLabelText("Update channel") as HTMLSelectElement).disabled).toBe(true);
  await act(async () => finish());
  await userEvent.click(await screen.findByRole("button", { name: "Restart srelens" }));
  expect(core.relaunchApp).toHaveBeenCalledOnce();
});
it("distinguishes failed checks from up to date and allows retry", async () => {
  core.checkForUpdate.mockRejectedValueOnce(new Error("offline")).mockResolvedValueOnce(null);
  render(<UpdatesPane />);
  await userEvent.click(screen.getByRole("button", { name: "Check for updates" }));
  expect((await screen.findByRole("alert")).textContent).toContain("Could not check for updates");
  expect(screen.queryByText("srelens is up to date.")).toBeNull();
  await userEvent.click(screen.getByRole("button", { name: "Check for updates" }));
  expect(await screen.findByText("srelens is up to date.")).toBeTruthy();
});
it("offers package-manager guidance without an install button", async () => {
  core.checkForUpdate.mockResolvedValue({ ...update, external: true }); render(<UpdatesPane />);
  await userEvent.click(screen.getByRole("button", { name: "Check for updates" }));
  expect(await screen.findByText(/managed by your system package manager/)).toBeTruthy();
  expect(screen.queryByRole("button", { name: "Download & install" })).toBeNull();
});
it("retains the installed state when restarting fails", async () => {
  core.relaunchApp.mockRejectedValue(new Error("restart refused")); render(<UpdatesPane />);
  await userEvent.click(screen.getByRole("button", { name: "Check for updates" }));
  await userEvent.click(await screen.findByRole("button", { name: "Download & install" }));
  await userEvent.click(await screen.findByRole("button", { name: "Restart srelens" }));
  expect((await screen.findByRole("alert")).textContent).toContain("Could not restart srelens");
  await waitFor(() => expect((screen.getByRole("button", { name: "Restart srelens" }) as HTMLButtonElement).disabled).toBe(false));
  expect(core.installUpdate).toHaveBeenCalledOnce();
});

it("loads release notes when the updater manifest omitted them", async () => {
  core.checkForUpdate.mockResolvedValue({ ...update, notes: "" });
  core.loadUpdateNotes.mockResolvedValue("### Recent changes\n- Fixed settings navigation");
  render(<UpdatesPane />);
  await userEvent.click(screen.getByRole("button", { name: "Check for updates" }));
  expect(await screen.findByText("Fixed settings navigation")).toBeTruthy();
  expect(screen.getByRole("heading", { name: "Release notes" })).toBeTruthy();
  expect(document.querySelector(".card")).toBeNull();
});
it("offers a notes retry and release link while keeping install available", async () => {
  core.loadUpdateNotes.mockResolvedValueOnce("Current notes").mockRejectedValueOnce(new Error("HTTP 403")).mockResolvedValueOnce("Recovered notes");
  render(<UpdatesPane />);
  await userEvent.click(screen.getByRole("button", { name: "Check for updates" }));
  expect(await screen.findByRole("alert")).toHaveProperty("textContent", expect.stringContaining("Could not load release notes"));
  expect(screen.getByRole("link", { name: "View release on GitHub" }).getAttribute("href")).toContain("srelens-v2.0.0");
  expect(screen.getByRole("button", { name: "Download & install" })).toBeTruthy();
  await userEvent.click(screen.getByRole("button", { name: "Retry release notes" }));
  expect(await screen.findByText("Recovered notes")).toBeTruthy();
});
it("shows the installed release notes before checking, then displays the available release", async () => {
  core.loadUpdateNotes.mockImplementation(async ({ version }) => version === "1.0.0" ? "Installed release notes" : "Available release notes");
  render(<UpdatesPane />);
  expect(await screen.findByText("Installed release notes")).toBeTruthy();
  expect(core.checkForUpdate).not.toHaveBeenCalled();
  await userEvent.click(screen.getByRole("button", { name: "Check for updates" }));
  expect(await screen.findByText("Available release notes")).toBeTruthy();
  expect(screen.queryByText("Installed release notes")).toBeNull();
});
it("keeps current release notes after an up-to-date check", async () => {
  core.checkForUpdate.mockResolvedValue(null);
  core.loadUpdateNotes.mockResolvedValue("Installed release notes");
  render(<UpdatesPane />);
  expect(await screen.findByText("Installed release notes")).toBeTruthy();
  await userEvent.click(screen.getByRole("button", { name: "Check for updates" }));
  expect(await screen.findByText("srelens is up to date.")).toBeTruthy();
  expect(screen.getByText("Installed release notes")).toBeTruthy();
});
it("allows retrying an install without hiding its notes", async () => {
  core.installUpdate.mockRejectedValueOnce(new Error("download interrupted")).mockResolvedValueOnce(undefined);
  render(<UpdatesPane />);
  await userEvent.click(screen.getByRole("button", { name: "Check for updates" }));
  await userEvent.click(await screen.findByRole("button", { name: "Download & install" }));
  expect((await screen.findByRole("alert")).textContent).toContain("Could not install the update");
  expect(screen.getByText("A useful fix")).toBeTruthy();
  await userEvent.click(screen.getByRole("button", { name: "Download & install" }));
  expect(await screen.findByRole("button", { name: "Restart srelens" })).toBeTruthy();
});
it("reports an unavailable installed version and lets the reader retry", async () => {
  core.appVersion.mockRejectedValueOnce(new Error("host refused")).mockResolvedValueOnce("1.0.0");
  render(<UpdatesPane />);
  expect((await screen.findByRole("alert")).textContent).toContain("Could not load the installed version");
  expect(core.loadUpdateNotes).not.toHaveBeenCalled();
  await userEvent.click(screen.getByRole("button", { name: "Retry installed version" }));
  expect(await screen.findByText("A useful fix")).toBeTruthy();
});
it("shows the current version once, with an installed comparison only for a newer release", async () => {
  render(<UpdatesPane />);
  await screen.findByRole("heading", { name: "Version 1.0.0" });
  expect(document.body.textContent?.match(/1\.0\.0/g)).toHaveLength(1);
  await userEvent.click(screen.getByRole("button", { name: "Check for updates" }));
  await screen.findByRole("heading", { name: "Version 2.0.0" });
  expect(document.body.textContent?.match(/1\.0\.0/g)).toHaveLength(1);
  expect(document.body.textContent?.match(/2\.0\.0/g)).toHaveLength(1);
});

it("opens the displayed release through the desktop browser helper", async () => {
  render(<UpdatesPane />);
  await userEvent.click(await screen.findByRole("link", { name: "View release on GitHub" }));
  expect(core.openExternal).toHaveBeenCalledWith("https://github.com/srelens/srelens/releases/tag/srelens-v1.0.0");
  await userEvent.click(screen.getByRole("button", { name: "Check for updates" }));
  await screen.findByRole("heading", { name: "Version 2.0.0" });
  await userEvent.click(screen.getByRole("link", { name: "View release on GitHub" }));
  expect(core.openExternal).toHaveBeenLastCalledWith("https://github.com/srelens/srelens/releases/tag/srelens-v2.0.0");
});
it("reports browser-opening failures beside the release notes", async () => {
  core.openExternal.mockRejectedValue(new Error("No default browser"));
  render(<UpdatesPane />);
  await userEvent.click(await screen.findByRole("link", { name: "View release on GitHub" }));
  expect((await screen.findByRole("alert")).textContent).toContain("Could not open the release in your browser");
  expect(screen.getByText("A useful fix")).toBeTruthy();
});
