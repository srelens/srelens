import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const core = vi.hoisted(() => ({
  exportSetupBundle: vi.fn(),
  pickSetupBundle: vi.fn(),
  previewSetupBundle: vi.fn(),
  importSetupBundle: vi.fn(),
}));
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  ...core,
}));

import { setNotifier, type BundleSummary, type ImportReport } from "@srelens/core";
import { BackupPane, reportLines } from "./BackupPane";

/**
 * A bundle with two clusters and one skill, and deliberately no prompts and no
 * secrets — so "a group the bundle does not carry" is a real case in the same
 * fixture rather than a second one nobody keeps in step.
 */
const SUMMARY: BundleSummary = {
  created: "2026-09-21T10:00:00Z",
  appVersion: "0.15.0",
  settingsKeys: 6,
  kubeconfigs: ["config", "prod.yaml"],
  skills: ["triage.md"],
  prompts: [],
  extensions: [],
  secretKeys: [],
  hasMcpToken: false,
};

const EMPTY_REPORT: ImportReport = {
  settingsWritten: [],
  kubeconfigsAdded: [],
  kubeconfigsAlreadyPresent: [],
  skillsAdded: [],
  skillsKeptLocal: [],
  promptsAdded: [],
  promptsKeptLocal: [],
  secretsWritten: [],
};

/** A passphrase that is long enough and is not a word anyone would type. */
const PASSPHRASE = "aaaa1111aaaa";

const notifications = { success: vi.fn(), error: vi.fn(), info: vi.fn() };

beforeEach(() => {
  core.exportSetupBundle.mockReset().mockResolvedValue("/tmp/srelens-setup.srelens");
  core.pickSetupBundle.mockReset().mockResolvedValue("/tmp/srelens-setup.srelens");
  core.previewSetupBundle.mockReset().mockResolvedValue(SUMMARY);
  core.importSetupBundle.mockReset().mockResolvedValue(EMPTY_REPORT);
  notifications.success.mockReset();
  notifications.error.mockReset();
  notifications.info.mockReset();
  setNotifier({
    success: notifications.success,
    error: notifications.error,
    info: notifications.info,
  });
});

async function fillExportForm(user: ReturnType<typeof userEvent.setup>, value = PASSPHRASE) {
  await user.type(screen.getByLabelText("Passphrase"), value);
  await user.type(screen.getByLabelText("Confirm passphrase"), value);
}

/** Pick a file and open it, leaving the preview on screen. */
async function openTheBundle() {
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: /choose file/i }));
  await user.type(screen.getByLabelText("Bundle passphrase"), PASSPHRASE);
  await user.click(screen.getByRole("button", { name: "Open" }));
  return user;
}

describe("BackupPane", () => {
  it("says the file is encrypted and that the passphrase cannot be recovered", () => {
    // Someone who exports with a passphrase they did not record holds a file
    // that is permanently useless. That belongs above the field, not after the
    // failure.
    render(<BackupPane />);
    expect(screen.getByText(/always encrypted/i)).toBeDefined();
    expect(screen.getByText(/no way to recover the file without it/i)).toBeDefined();
  });

  it("refuses to export until the passphrase is long enough and confirmed", async () => {
    const user = userEvent.setup();
    render(<BackupPane />);
    const button = screen.getByRole("button", { name: /export setup/i });
    expect(button).toHaveProperty("disabled", true);

    await user.type(screen.getByLabelText("Passphrase"), "short");
    expect(screen.getByText(/at least 8 characters/i)).toBeDefined();
    expect(button).toHaveProperty("disabled", true);

    await user.clear(screen.getByLabelText("Passphrase"));
    await user.type(screen.getByLabelText("Passphrase"), PASSPHRASE);
    await user.type(screen.getByLabelText("Confirm passphrase"), "bbbb2222bbbb");
    expect(screen.getByText(/don't match/i)).toBeDefined();
    expect(button).toHaveProperty("disabled", true);
  });

  it("leaves the API keys out unless they are asked for", async () => {
    // The narrower option has to be the default: a reader who ticks nothing
    // must not find their provider keys in a file they mail to themselves.
    const user = userEvent.setup();
    render(<BackupPane />);
    await fillExportForm(user);
    await user.click(screen.getByRole("button", { name: /export setup/i }));
    expect(core.exportSetupBundle).toHaveBeenCalledWith(
      expect.objectContaining({ includeSecrets: false }),
    );

    await user.click(screen.getByRole("checkbox", { name: /api keys and the mcp token/i }));
    await fillExportForm(user);
    await user.click(screen.getByRole("button", { name: /export setup/i }));
    expect(core.exportSetupBundle).toHaveBeenLastCalledWith(
      expect.objectContaining({ includeSecrets: true }),
    );
  });

  it("does not announce a save the reader cancelled", async () => {
    // `null` is the cancelled dialog. "Setup exported to null" is the failure
    // this guards.
    core.exportSetupBundle.mockResolvedValue(null);
    const user = userEvent.setup();
    render(<BackupPane />);
    await fillExportForm(user);
    await user.click(screen.getByRole("button", { name: /export setup/i }));

    expect(notifications.success).not.toHaveBeenCalled();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("surfaces a failed export rather than letting the button go quiet", async () => {
    core.exportSetupBundle.mockRejectedValue(new Error("write /tmp/x: permission denied"));
    const user = userEvent.setup();
    render(<BackupPane />);
    await fillExportForm(user);
    await user.click(screen.getByRole("button", { name: /export setup/i }));

    expect(screen.getByText(/could not be exported/i)).toBeDefined();
    // Still usable: a failed export must not eat the passphrase and leave the
    // reader retyping it blind.
    expect(screen.getByRole("button", { name: /export setup/i })).toHaveProperty("disabled", false);
  });

  it("shows what a bundle holds before writing anything", async () => {
    render(<BackupPane />);
    await openTheBundle();

    expect(screen.getByText(/2 kubeconfig files/i)).toBeDefined();
    expect(screen.getByText(/6 preferences/i)).toBeDefined();
    expect(core.importSetupBundle).not.toHaveBeenCalled();
  });

  it("offers no checkbox for a group the bundle does not carry", async () => {
    // "MCP prompts — 0 prompts" invites a reader to select something that
    // would do nothing.
    render(<BackupPane />);
    await openTheBundle();

    expect(screen.queryByRole("checkbox", { name: /mcp prompts/i })).toBeNull();
    expect(screen.getByRole("checkbox", { name: /assistant skills/i })).toBeDefined();
  });

  it("reports a refused open verbatim instead of rendering an empty bundle", async () => {
    // A failed read must never become a claim about the file's contents. The
    // backend separates a wrong passphrase from a damaged file from one that
    // is not a bundle, and the remedies differ.
    core.previewSetupBundle.mockRejectedValue(
      new Error("that passphrase does not open this bundle"),
    );
    render(<BackupPane />);
    await openTheBundle();

    expect(screen.getByText(/does not open this bundle/i)).toBeDefined();
    expect(screen.queryByRole("button", { name: /import selected/i })).toBeNull();
  });

  it("clears a stale failure when another file is chosen", async () => {
    core.previewSetupBundle.mockRejectedValueOnce(new Error("this bundle is damaged"));
    const user = await openTheBundleIn(<BackupPane />);
    expect(screen.getByText(/damaged/i)).toBeDefined();

    await user.click(screen.getByRole("button", { name: /choose file/i }));
    expect(screen.queryByText(/damaged/i)).toBeNull();
  });

  it("imports only the groups still ticked", async () => {
    render(<BackupPane />);
    const user = await openTheBundle();

    await user.click(screen.getByRole("checkbox", { name: /^Settings/ }));
    await user.click(screen.getByRole("button", { name: /import selected/i }));

    expect(core.importSetupBundle).toHaveBeenCalledWith({
      path: "/tmp/srelens-setup.srelens",
      passphrase: PASSPHRASE,
      groups: ["kubeconfigs", "skills"],
    });
  });

  it("says plainly when an import had nothing left to do", async () => {
    // An import that wrote nothing because the machine is already set up looks
    // identical to one that silently failed, unless it says which.
    render(<BackupPane />);
    const user = await openTheBundle();
    await user.click(screen.getByRole("button", { name: /import selected/i }));

    expect(screen.getByRole("status").textContent).toMatch(/already has everything/i);
    expect(notifications.success).not.toHaveBeenCalled();
  });

  it("tells the reader apps are not imported, and where to get them", async () => {
    core.previewSetupBundle.mockResolvedValue({
      ...SUMMARY,
      extensions: [{ id: "acme.logs", name: "Logs", version: "1.2.0" }],
    });
    render(<BackupPane />);
    await openTheBundle();

    expect(screen.getByText(/Apps are not imported/i)).toBeDefined();
    expect(screen.getByText(/capability grants/i).textContent).toMatch(/Logs/);
    expect(screen.queryByRole("checkbox", { name: /^Apps/ })).toBeNull();
  });
});

/** Render then open, for the one test that needs the user before the render. */
async function openTheBundleIn(element: React.ReactElement) {
  render(element);
  return await openTheBundle();
}

describe("reportLines", () => {
  it("names what was written and what was left alone", () => {
    const lines = reportLines({
      ...EMPTY_REPORT,
      kubeconfigsAdded: ["prod.yaml"],
      kubeconfigsAlreadyPresent: ["config"],
      settingsWritten: ["srelens.defaultNamespace", "srelens.contextProfiles"],
      skillsKeptLocal: ["triage.md"],
      secretsWritten: ["anthropic", "MCP token"],
    }).join("\n");

    expect(lines).toMatch(/Added 1 kubeconfig: prod\.yaml/);
    expect(lines).toMatch(/1 kubeconfig was already here: config/);
    expect(lines).toMatch(/Applied 2 preferences/);
    expect(lines).toMatch(/Kept your own version of 1 skill: triage\.md/);
    expect(lines).toMatch(/Stored anthropic, MCP token/);
  });

  it("says nothing about a group that had nothing in it", () => {
    expect(reportLines(EMPTY_REPORT)).toEqual([]);
  });

  it("agrees in number with what it is counting", () => {
    const lines = reportLines({
      ...EMPTY_REPORT,
      kubeconfigsAdded: ["a", "b"],
      kubeconfigsAlreadyPresent: ["c", "d"],
      settingsWritten: ["one"],
    }).join("\n");
    expect(lines).toMatch(/Added 2 kubeconfigs: a, b/);
    expect(lines).toMatch(/2 kubeconfigs were already here/);
    expect(lines).toMatch(/Applied 1 preference\./);
  });
});
