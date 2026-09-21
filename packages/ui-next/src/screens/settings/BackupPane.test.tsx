import { describe, it, expect, vi, afterEach, beforeEach } from "vitest";
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
  kubeconfigsRejected: [],
  skillsAdded: [],
  skillsKeptLocal: [],
  promptsAdded: [],
  promptsKeptLocal: [],
  secretsWritten: [],
};

/** A passphrase that is long enough and is not a word anyone would type. */
const PASSPHRASE = "aaaa1111aaaa";

const notifications = { success: vi.fn(), error: vi.fn(), info: vi.fn() };

/**
 * `setNotifier` replaces a module global. Without restoring it, these spies
 * stay installed for every later suite sharing this worker, and a failure
 * there would point at whatever ran next rather than here.
 */
let restoreNotifier: () => void = () => {};
afterEach(() => restoreNotifier());

beforeEach(() => {
  core.exportSetupBundle.mockReset().mockResolvedValue("/tmp/srelens-setup.srelens");
  core.pickSetupBundle.mockReset().mockResolvedValue("/tmp/srelens-setup.srelens");
  core.previewSetupBundle.mockReset().mockResolvedValue(SUMMARY);
  core.importSetupBundle.mockReset().mockResolvedValue(EMPTY_REPORT);
  notifications.success.mockReset();
  notifications.error.mockReset();
  notifications.info.mockReset();
  restoreNotifier = setNotifier({
    success: notifications.success,
    error: notifications.error,
    info: notifications.info,
    updateAvailable: () => {},
    clusterSignIn: () => {},
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
    // Not only that the command was called with the right payload: the pane
    // has to tell the reader the file was written, and where.
    expect(notifications.success.mock.calls.at(-1)?.[0]).toContain("/tmp/srelens-setup.srelens");
  });

  it("renders the report a successful import came back with", async () => {
    // A pane that sends the right request and then shows nothing is a pane
    // that looks broken, and a test asserting only the mock call passes it.
    core.importSetupBundle.mockResolvedValue({
      ...EMPTY_REPORT,
      kubeconfigsAdded: ["prod.yaml"],
      settingsWritten: ["srelens.defaultNamespace"],
    });
    render(<BackupPane />);
    const user = await openTheBundle();
    await user.click(screen.getByRole("button", { name: /import selected/i }));

    const status = screen.getByRole("status").textContent ?? "";
    expect(status).toMatch(/Added 1 kubeconfig: prod\.yaml/);
    expect(status).toMatch(/Applied 1 preference/);
    expect(status).toMatch(/Reload srelens/);
    expect(notifications.success).toHaveBeenCalled();
  });

  it("does not tell the reader to reload when the import wrote nothing", async () => {
    // A report of nothing but skips is not empty, so keying the reload advice
    // off the line count promised changes that were never made.
    core.importSetupBundle.mockResolvedValue({
      ...EMPTY_REPORT,
      kubeconfigsAlreadyPresent: ["config"],
      skillsKeptLocal: ["triage.md"],
    });
    render(<BackupPane />);
    const user = await openTheBundle();
    await user.click(screen.getByRole("button", { name: /import selected/i }));

    const status = screen.getByRole("status").textContent ?? "";
    expect(status).toMatch(/1 kubeconfig was already here: config/);
    expect(status).toMatch(/Kept your own version of 1 skill/);
    expect(status).toMatch(/already has everything/);
    expect(status).not.toMatch(/Reload srelens/);
    expect(notifications.success).not.toHaveBeenCalled();
  });

  it("names the step that failed, not the file, when an import is refused", async () => {
    // One title over every failure blamed the bundle for a read-only config
    // directory.
    core.importSetupBundle.mockRejectedValue(new Error("create /config: read-only"));
    render(<BackupPane />);
    const user = await openTheBundle();
    await user.click(screen.getByRole("button", { name: /import selected/i }));

    expect(screen.getByText("The setup could not be imported")).toBeDefined();
    expect(screen.queryByText("That bundle could not be opened")).toBeNull();
    // The backend's own words survive the titling.
    expect(screen.getByText(/read-only/)).toBeDefined();
  });

  it("cannot import a manifest that belongs to a file the reader has left", async () => {
    // argon2id is deliberately slow, so a second Choose file… can land while
    // the first preview is still decrypting. The stale manifest must not
    // become importable under the new file's name.
    let releaseFirst: (summary: BundleSummary) => void = () => {};
    core.previewSetupBundle.mockImplementationOnce(
      () => new Promise<BundleSummary>((resolve) => (releaseFirst = resolve)),
    );
    const user = userEvent.setup();
    render(<BackupPane />);
    await user.click(screen.getByRole("button", { name: /choose file/i }));
    await user.type(screen.getByLabelText("Bundle passphrase"), PASSPHRASE);
    await user.click(screen.getByRole("button", { name: "Open" }));

    // The reader gives up and picks something else while that is in flight.
    core.pickSetupBundle.mockResolvedValue("/tmp/other.srelens");
    await user.click(screen.getByRole("button", { name: /choose file/i }));

    releaseFirst(SUMMARY);
    await Promise.resolve();

    expect(screen.queryByRole("button", { name: /import selected/i })).toBeNull();
    expect(core.importSetupBundle).not.toHaveBeenCalled();
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

  it("does not leave the last import's report standing beside a new failure", async () => {
    // Import once successfully, then again into a failure. The first report
    // describes writes the second attempt did not make.
    core.importSetupBundle.mockResolvedValueOnce({
      ...EMPTY_REPORT,
      kubeconfigsAdded: ["prod.yaml"],
    });
    render(<BackupPane />);
    const user = await openTheBundle();
    await user.click(screen.getByRole("button", { name: /import selected/i }));
    expect(screen.getByRole("status").textContent).toMatch(/Added 1 kubeconfig/);

    core.importSetupBundle.mockRejectedValueOnce(new Error("create /config: read-only"));
    await user.click(screen.getByRole("button", { name: /import selected/i }));

    expect(screen.getByText(/read-only/)).toBeDefined();
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("does not claim the machine already has a file it actually refused", async () => {
    // A rejected file was not skipped because it is already here — it was
    // refused. "this machine already has everything in that bundle" is a claim
    // about the machine, and nothing checked it.
    core.importSetupBundle.mockResolvedValue({
      ...EMPTY_REPORT,
      kubeconfigsRejected: ["notes.yaml"],
    });
    render(<BackupPane />);
    const user = await openTheBundle();
    await user.click(screen.getByRole("button", { name: /import selected/i }));

    const status = screen.getByRole("status").textContent ?? "";
    expect(status).toMatch(/not a kubeconfig and was not imported: notes\.yaml/);
    expect(status).toMatch(/Nothing was imported\./);
    expect(status).not.toMatch(/already has everything/);
    expect(status).not.toMatch(/Reload srelens/);
  });

  it("stays usable when a second file is chosen while the first is still opening", async () => {
    // The superseded attempt's `finally` is guarded by its own token, so it
    // never resets `opening` — without clearing it in `choose`, the Open
    // button on the newly picked file was disabled for good.
    let releaseFirst: (summary: BundleSummary) => void = () => {};
    core.previewSetupBundle.mockImplementationOnce(
      () => new Promise<BundleSummary>((resolve) => (releaseFirst = resolve)),
    );
    const user = userEvent.setup();
    render(<BackupPane />);
    await user.click(screen.getByRole("button", { name: /choose file/i }));
    await user.type(screen.getByLabelText("Bundle passphrase"), PASSPHRASE);
    await user.click(screen.getByRole("button", { name: "Open" }));
    expect(screen.getByRole("button", { name: /opening/i })).toBeDefined();

    core.pickSetupBundle.mockResolvedValue("/tmp/other.srelens");
    await user.click(screen.getByRole("button", { name: /choose file/i }));
    releaseFirst(SUMMARY);
    await Promise.resolve();

    // The replacement workflow runs to completion on the second file.
    const open = screen.getByRole("button", { name: "Open" });
    expect(open).toHaveProperty("disabled", false);
    await user.click(open);
    expect(screen.getByRole("button", { name: /import selected/i })).toBeDefined();
    expect(core.previewSetupBundle).toHaveBeenLastCalledWith("/tmp/other.srelens", PASSPHRASE);
  });

  it("does not make a bundle-wide claim after a partial import", async () => {
    // Only the ticked groups were imported, so "everything in that bundle"
    // says something about groups this import never looked at.
    core.importSetupBundle.mockResolvedValue(EMPTY_REPORT);
    render(<BackupPane />);
    const user = await openTheBundle();
    await user.click(screen.getByRole("checkbox", { name: /^Clusters/ }));
    await user.click(screen.getByRole("button", { name: /import selected/i }));

    const status = screen.getByRole("status").textContent ?? "";
    expect(status).toMatch(/Nothing was imported\./);
    expect(status).not.toMatch(/already has everything/);
  });

  it("stays usable when the file picker is cancelled mid-decrypt", async () => {
    // `choose` used to invalidate the pending open BEFORE knowing whether a
    // file was picked, and the cancel path returned before clearing
    // `opening` — so cancelling the picker stranded the Open button.
    let releaseFirst: (summary: BundleSummary) => void = () => {};
    core.previewSetupBundle.mockImplementationOnce(
      () => new Promise<BundleSummary>((resolve) => (releaseFirst = resolve)),
    );
    const user = userEvent.setup();
    render(<BackupPane />);
    await user.click(screen.getByRole("button", { name: /choose file/i }));
    await user.type(screen.getByLabelText("Bundle passphrase"), PASSPHRASE);
    await user.click(screen.getByRole("button", { name: "Open" }));

    core.pickSetupBundle.mockResolvedValue(null); // cancelled
    await user.click(screen.getByRole("button", { name: /choose file/i }));

    // The original open was never superseded, so it still lands.
    releaseFirst(SUMMARY);
    await Promise.resolve();
    expect(await screen.findByRole("button", { name: /import selected/i })).toBeDefined();
  });

  it("will not let a second file be chosen while an import is running", async () => {
    // The token suppresses a stale UI update; it does not cancel
    // `bundle_import`. A second import over a backend that writes files,
    // settings and secrets in separate steps has no lock to protect it.
    let releaseImport: (report: ImportReport) => void = () => {};
    core.importSetupBundle.mockImplementationOnce(
      () => new Promise<ImportReport>((resolve) => (releaseImport = resolve)),
    );
    render(<BackupPane />);
    const user = await openTheBundle();
    await user.click(screen.getByRole("button", { name: /import selected/i }));

    expect(screen.getByRole("button", { name: /choose file/i })).toHaveProperty("disabled", true);

    releaseImport(EMPTY_REPORT);
    await Promise.resolve();
    expect(await screen.findByRole("status")).toBeDefined();
    expect(screen.getByRole("button", { name: /choose file/i })).toHaveProperty("disabled", false);
    expect(core.importSetupBundle).toHaveBeenCalledTimes(1);
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
      kubeconfigsRejected: ["notes.yaml"],
      settingsWritten: ["srelens.defaultNamespace", "srelens.contextProfiles"],
      skillsKeptLocal: ["triage.md"],
      secretsWritten: ["anthropic", "MCP token"],
    }).join("\n");

    expect(lines).toMatch(/Added 1 kubeconfig: prod\.yaml/);
    expect(lines).toMatch(/1 kubeconfig was already here: config/);
    expect(lines).toMatch(/1 bundled file is not a kubeconfig and was not imported: notes\.yaml/);
    // A rejected file is not "already here" — that would be a claim about this
    // machine that nothing checked.
    expect(lines).not.toMatch(/notes\.yaml.*already here/);
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
