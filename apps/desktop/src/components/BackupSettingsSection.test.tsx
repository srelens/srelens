import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const {
  exportSetupBundleMock,
  pickSetupBundleMock,
  previewSetupBundleMock,
  importSetupBundleMock,
  notifyMock,
} = vi.hoisted(() => ({
  exportSetupBundleMock: vi.fn(),
  pickSetupBundleMock: vi.fn(),
  previewSetupBundleMock: vi.fn(),
  importSetupBundleMock: vi.fn(),
  notifyMock: { success: vi.fn(), error: vi.fn(), info: vi.fn() },
}));

vi.mock("@srelens/core", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@srelens/core")>()),
  exportSetupBundle: exportSetupBundleMock,
  pickSetupBundle: pickSetupBundleMock,
  previewSetupBundle: previewSetupBundleMock,
  importSetupBundle: importSetupBundleMock,
  notify: notifyMock,
}));

import { BackupSettingsSection, reportLines } from "./BackupSettingsSection";
import type { BundleSummary, ImportReport } from "@srelens/core";

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

beforeEach(() => {
  exportSetupBundleMock.mockReset().mockResolvedValue("/tmp/srelens-setup.srelens");
  pickSetupBundleMock.mockReset().mockResolvedValue("/tmp/srelens-setup.srelens");
  previewSetupBundleMock.mockReset().mockResolvedValue(SUMMARY);
  importSetupBundleMock.mockReset().mockResolvedValue(EMPTY_REPORT);
  notifyMock.success.mockReset();
  notifyMock.error.mockReset();
  notifyMock.info.mockReset();
});

/** Fill the export form with a valid, confirmed passphrase. */
async function fillExportPassphrase(user: ReturnType<typeof userEvent.setup>, value = "correct horse") {
  await user.type(screen.getByLabelText("Passphrase"), value);
  await user.type(screen.getByLabelText("Confirm passphrase"), value);
}

describe("BackupSettingsSection", () => {
  it("says the file is encrypted and the passphrase cannot be recovered", () => {
    // Someone who exports with a passphrase they don't record has a file that
    // is permanently useless. That has to be said before the field, not after
    // the failure.
    render(<BackupSettingsSection />);
    expect(screen.getByText(/always encrypted/i)).toBeDefined();
    expect(screen.getByText(/no way to recover the file without it/i)).toBeDefined();
  });

  it("will not export until the passphrase is long enough and confirmed", async () => {
    const user = userEvent.setup();
    render(<BackupSettingsSection />);
    const button = screen.getByRole("button", { name: /export setup/i });
    expect(button).toHaveProperty("disabled", true);

    await user.type(screen.getByLabelText("Passphrase"), "short");
    expect(screen.getByText(/at least 8 characters/i)).toBeDefined();
    expect(button).toHaveProperty("disabled", true);

    // Long enough, but the confirmation disagrees — still refused, and said.
    await user.clear(screen.getByLabelText("Passphrase"));
    await user.type(screen.getByLabelText("Passphrase"), "correct horse");
    await user.type(screen.getByLabelText("Confirm passphrase"), "correct hoarse");
    expect(screen.getByText(/don't match/i)).toBeDefined();
    expect(button).toHaveProperty("disabled", true);
  });

  it("leaves the API keys out unless they are asked for", async () => {
    // The default has to be the narrower one: a reader who ticks nothing
    // should not find their provider keys in a file they mail to themselves.
    const user = userEvent.setup();
    render(<BackupSettingsSection />);
    await fillExportPassphrase(user);
    await user.click(screen.getByRole("button", { name: /export setup/i }));

    expect(exportSetupBundleMock).toHaveBeenCalledWith(
      expect.objectContaining({ includeSecrets: false }),
    );

    await user.click(screen.getByRole("checkbox", { name: /api keys and the mcp token/i }));
    await fillExportPassphrase(user);
    await user.click(screen.getByRole("button", { name: /export setup/i }));
    expect(exportSetupBundleMock).toHaveBeenLastCalledWith(
      expect.objectContaining({ includeSecrets: true }),
    );
  });

  it("does not claim a save that the reader cancelled", async () => {
    // `null` is the cancelled dialog. Toasting "Setup exported to null" is
    // the failure this guards.
    exportSetupBundleMock.mockResolvedValue(null);
    const user = userEvent.setup();
    render(<BackupSettingsSection />);
    await fillExportPassphrase(user);
    await user.click(screen.getByRole("button", { name: /export setup/i }));

    expect(notifyMock.success).not.toHaveBeenCalled();
    expect(notifyMock.error).not.toHaveBeenCalled();
  });

  it("surfaces a failed export instead of leaving the button to go quiet", async () => {
    exportSetupBundleMock.mockRejectedValue(new Error("write /tmp/x: permission denied"));
    const user = userEvent.setup();
    render(<BackupSettingsSection />);
    await fillExportPassphrase(user);
    await user.click(screen.getByRole("button", { name: /export setup/i }));

    expect(notifyMock.error).toHaveBeenCalledWith(expect.stringMatching(/permission denied/));
    // The form is still usable: a failed export must not eat the passphrase
    // and leave the reader retyping it blind.
    expect(screen.getByRole("button", { name: /export setup/i })).toHaveProperty("disabled", false);
  });

  it("surfaces a failed import beside the form, not only as a toast", async () => {
    importSetupBundleMock.mockRejectedValue(new Error("create /config/kubeconfigs: read-only"));
    const user = userEvent.setup();
    render(<BackupSettingsSection />);
    await user.click(screen.getByRole("button", { name: /choose file/i }));
    await user.type(screen.getByLabelText("Bundle passphrase"), "pw");
    await user.click(screen.getByRole("button", { name: "Open" }));
    await user.click(screen.getByRole("button", { name: /import selected/i }));

    expect(screen.getByRole("alert").textContent).toMatch(/read-only/);
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("does not leave the last import's report standing beside a new failure", async () => {
    // Import once successfully, then again into a failure. The first report
    // describes writes the second attempt did not make, so it goes with the
    // error rather than sitting beside it.
    importSetupBundleMock.mockResolvedValueOnce({
      ...EMPTY_REPORT,
      kubeconfigsAdded: ["prod.yaml"],
    });
    const user = userEvent.setup();
    render(<BackupSettingsSection />);
    await user.click(screen.getByRole("button", { name: /choose file/i }));
    await user.type(screen.getByLabelText("Bundle passphrase"), "pw");
    await user.click(screen.getByRole("button", { name: "Open" }));
    await user.click(screen.getByRole("button", { name: /import selected/i }));
    expect(screen.getByRole("status").textContent).toMatch(/Added 1 kubeconfig/);

    importSetupBundleMock.mockRejectedValueOnce(new Error("create /config: read-only"));
    await user.click(screen.getByRole("button", { name: /import selected/i }));

    expect(screen.getByRole("alert").textContent).toMatch(/read-only/);
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("clears a stale error and report when a different file is chosen", async () => {
    // Choosing a second file after a wrong passphrase must not leave the
    // first file's error under the new one's name.
    previewSetupBundleMock.mockRejectedValueOnce(new Error("that passphrase does not open this bundle"));
    const user = userEvent.setup();
    render(<BackupSettingsSection />);
    await user.click(screen.getByRole("button", { name: /choose file/i }));
    await user.type(screen.getByLabelText("Bundle passphrase"), "wrong");
    await user.click(screen.getByRole("button", { name: "Open" }));
    expect(screen.getByRole("alert")).toBeDefined();

    await user.click(screen.getByRole("button", { name: /choose file/i }));
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("shows what a bundle holds before anything is written", async () => {
    const user = userEvent.setup();
    render(<BackupSettingsSection />);
    await user.click(screen.getByRole("button", { name: /choose file/i }));
    await user.type(screen.getByLabelText("Bundle passphrase"), "correct horse");
    await user.click(screen.getByRole("button", { name: "Open" }));

    expect(screen.getByText(/2 kubeconfig files/i)).toBeDefined();
    expect(screen.getByText(/6 preferences/i)).toBeDefined();
    expect(importSetupBundleMock).not.toHaveBeenCalled();
  });

  it("offers no checkbox for a group the bundle does not carry", async () => {
    // A "Assistant skills — 0 skills" row invites a reader to select
    // something that will do nothing.
    const user = userEvent.setup();
    render(<BackupSettingsSection />);
    await user.click(screen.getByRole("button", { name: /choose file/i }));
    await user.type(screen.getByLabelText("Bundle passphrase"), "pw");
    await user.click(screen.getByRole("button", { name: "Open" }));

    expect(screen.queryByRole("checkbox", { name: /mcp prompts/i })).toBeNull();
    expect(screen.getByRole("checkbox", { name: /assistant skills/i })).toBeDefined();
  });

  it("reports a wrong passphrase verbatim instead of rendering an empty bundle", async () => {
    previewSetupBundleMock.mockRejectedValue("that passphrase does not open this bundle");
    const user = userEvent.setup();
    render(<BackupSettingsSection />);
    await user.click(screen.getByRole("button", { name: /choose file/i }));
    await user.type(screen.getByLabelText("Bundle passphrase"), "wrong");
    await user.click(screen.getByRole("button", { name: "Open" }));

    expect(screen.getByRole("alert").textContent).toMatch(/does not open this bundle/i);
    expect(screen.queryByRole("button", { name: /import selected/i })).toBeNull();
  });

  it("imports only the groups still ticked", async () => {
    const user = userEvent.setup();
    render(<BackupSettingsSection />);
    await user.click(screen.getByRole("button", { name: /choose file/i }));
    await user.type(screen.getByLabelText("Bundle passphrase"), "pw");
    await user.click(screen.getByRole("button", { name: "Open" }));

    await user.click(screen.getByRole("checkbox", { name: /settings/i }));
    await user.click(screen.getByRole("button", { name: /import selected/i }));

    expect(importSetupBundleMock).toHaveBeenCalledWith({
      path: "/tmp/srelens-setup.srelens",
      passphrase: "pw",
      groups: ["kubeconfigs", "skills"],
    });
  });

  it("says plainly when an import had nothing left to do", async () => {
    // An import that writes nothing because the machine is already set up
    // looks identical to one that silently failed, unless it says so.
    const user = userEvent.setup();
    render(<BackupSettingsSection />);
    await user.click(screen.getByRole("button", { name: /choose file/i }));
    await user.type(screen.getByLabelText("Bundle passphrase"), "pw");
    await user.click(screen.getByRole("button", { name: "Open" }));
    await user.click(screen.getByRole("button", { name: /import selected/i }));

    expect(screen.getByRole("status").textContent).toMatch(/already has everything/i);
    expect(notifyMock.success).not.toHaveBeenCalled();
  });

  it("does not tell the reader to reload when the import wrote nothing", async () => {
    // A report of nothing but skips is not empty, so keying the reload advice
    // off the line count promised changes that were never made.
    importSetupBundleMock.mockResolvedValue({
      ...EMPTY_REPORT,
      kubeconfigsAlreadyPresent: ["config"],
      skillsKeptLocal: ["triage.md"],
    });
    const user = userEvent.setup();
    render(<BackupSettingsSection />);
    await user.click(screen.getByRole("button", { name: /choose file/i }));
    await user.type(screen.getByLabelText("Bundle passphrase"), "pw");
    await user.click(screen.getByRole("button", { name: "Open" }));
    await user.click(screen.getByRole("button", { name: /import selected/i }));

    const status = screen.getByRole("status").textContent ?? "";
    expect(status).toMatch(/1 kubeconfig was already here: config/);
    expect(status).toMatch(/already has everything/);
    expect(status).not.toMatch(/Reload srelens/);
  });

  it("cannot import a manifest that belongs to a file the reader has left", async () => {
    // argon2id is deliberately slow, so a second Choose file… can land while
    // the first preview is still decrypting. The stale manifest must not
    // become importable under the new file's name.
    let releaseFirst: (summary: BundleSummary) => void = () => {};
    previewSetupBundleMock.mockImplementationOnce(
      () => new Promise<BundleSummary>((resolve) => (releaseFirst = resolve)),
    );
    const user = userEvent.setup();
    render(<BackupSettingsSection />);
    await user.click(screen.getByRole("button", { name: /choose file/i }));
    await user.type(screen.getByLabelText("Bundle passphrase"), "pw");
    await user.click(screen.getByRole("button", { name: "Open" }));

    pickSetupBundleMock.mockResolvedValue("/tmp/other.srelens");
    await user.click(screen.getByRole("button", { name: /choose file/i }));

    releaseFirst(SUMMARY);
    await Promise.resolve();

    expect(screen.queryByRole("button", { name: /import selected/i })).toBeNull();
    expect(importSetupBundleMock).not.toHaveBeenCalled();
  });

  it("does not claim the machine already has a file it actually refused", async () => {
    // A rejected file was not skipped because it is already here — it was
    // refused. "this machine already has everything in that bundle" is a claim
    // about the machine, and nothing checked it.
    importSetupBundleMock.mockResolvedValue({
      ...EMPTY_REPORT,
      kubeconfigsRejected: ["notes.yaml"],
    });
    const user = userEvent.setup();
    render(<BackupSettingsSection />);
    await user.click(screen.getByRole("button", { name: /choose file/i }));
    await user.type(screen.getByLabelText("Bundle passphrase"), "pw");
    await user.click(screen.getByRole("button", { name: "Open" }));
    await user.click(screen.getByRole("button", { name: /import selected/i }));

    const status = screen.getByRole("status").textContent ?? "";
    expect(status).toMatch(/not a kubeconfig and was not imported: notes\.yaml/);
    expect(status).toMatch(/Nothing was imported\./);
    expect(status).not.toMatch(/already has everything/);
    expect(notifyMock.info).toHaveBeenCalledWith("Nothing was imported.");
  });

  it("stays usable when a second file is chosen while the first is still opening", async () => {
    // The superseded attempt's `finally` is guarded by its own token, so it
    // never resets `opening` — without clearing it in `choose`, the Open
    // button on the newly picked file was disabled for good.
    let releaseFirst: (summary: BundleSummary) => void = () => {};
    previewSetupBundleMock.mockImplementationOnce(
      () => new Promise<BundleSummary>((resolve) => (releaseFirst = resolve)),
    );
    const user = userEvent.setup();
    render(<BackupSettingsSection />);
    await user.click(screen.getByRole("button", { name: /choose file/i }));
    await user.type(screen.getByLabelText("Bundle passphrase"), "pw");
    await user.click(screen.getByRole("button", { name: "Open" }));
    expect(screen.getByRole("button", { name: /opening/i })).toBeDefined();

    pickSetupBundleMock.mockResolvedValue("/tmp/other.srelens");
    await user.click(screen.getByRole("button", { name: /choose file/i }));
    releaseFirst(SUMMARY);
    await Promise.resolve();

    const open = screen.getByRole("button", { name: "Open" });
    expect(open).toHaveProperty("disabled", false);
    await user.click(open);
    expect(screen.getByRole("button", { name: /import selected/i })).toBeDefined();
    expect(previewSetupBundleMock).toHaveBeenLastCalledWith("/tmp/other.srelens", "pw");
  });

  it("does not make a bundle-wide claim after a partial import", async () => {
    // Only the ticked groups were imported, so "everything in that bundle"
    // says something about groups this import never looked at.
    importSetupBundleMock.mockResolvedValue(EMPTY_REPORT);
    const user = userEvent.setup();
    render(<BackupSettingsSection />);
    await user.click(screen.getByRole("button", { name: /choose file/i }));
    await user.type(screen.getByLabelText("Bundle passphrase"), "pw");
    await user.click(screen.getByRole("button", { name: "Open" }));
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
    previewSetupBundleMock.mockImplementationOnce(
      () => new Promise<BundleSummary>((resolve) => (releaseFirst = resolve)),
    );
    const user = userEvent.setup();
    render(<BackupSettingsSection />);
    await user.click(screen.getByRole("button", { name: /choose file/i }));
    await user.type(screen.getByLabelText("Bundle passphrase"), "pw");
    await user.click(screen.getByRole("button", { name: "Open" }));

    pickSetupBundleMock.mockResolvedValue(null); // cancelled
    await user.click(screen.getByRole("button", { name: /choose file/i }));

    releaseFirst(SUMMARY);
    await Promise.resolve();
    expect(await screen.findByRole("button", { name: /import selected/i })).toBeDefined();
  });

  it("will not let a second file be chosen while an import is running", async () => {
    // The token suppresses a stale UI update; it does not cancel
    // `bundle_import`. A second import over a backend that writes files,
    // settings and secrets in separate steps has no lock to protect it.
    let releaseImport: (report: ImportReport) => void = () => {};
    importSetupBundleMock.mockImplementationOnce(
      () => new Promise<ImportReport>((resolve) => (releaseImport = resolve)),
    );
    const user = userEvent.setup();
    render(<BackupSettingsSection />);
    await user.click(screen.getByRole("button", { name: /choose file/i }));
    await user.type(screen.getByLabelText("Bundle passphrase"), "pw");
    await user.click(screen.getByRole("button", { name: "Open" }));
    await user.click(screen.getByRole("button", { name: /import selected/i }));

    expect(screen.getByRole("button", { name: /choose file/i })).toHaveProperty("disabled", true);

    releaseImport(EMPTY_REPORT);
    await Promise.resolve();
    expect(await screen.findByRole("status")).toBeDefined();
    expect(screen.getByRole("button", { name: /choose file/i })).toHaveProperty("disabled", false);
    expect(importSetupBundleMock).toHaveBeenCalledTimes(1);
  });


  it("does not change an import's verdict when the checkboxes are touched afterwards", async () => {
    // The group boxes stay editable after an import. A verdict recomputed from
    // them changed retroactively: a no-op partial import said "Nothing was
    // imported.", and ticking the group that had been left out flipped the
    // same block to a claim about an import that never ran.
    importSetupBundleMock.mockResolvedValue(EMPTY_REPORT);
    const user = userEvent.setup();
    render(<BackupSettingsSection />);
    await user.click(screen.getByRole("button", { name: /choose file/i }));
    await user.type(screen.getByLabelText("Bundle passphrase"), "pw");
    await user.click(screen.getByRole("button", { name: "Open" }));
    await user.click(screen.getByRole("checkbox", { name: /^Clusters/ }));
    await user.click(screen.getByRole("button", { name: /import selected/i }));
    expect(screen.getByRole("status").textContent ?? "").toMatch(/Nothing was imported./);

    // Tick it back on WITHOUT importing again.
    await user.click(screen.getByRole("checkbox", { name: /^Clusters/ }));

    const status = screen.getByRole("status").textContent ?? "";
    expect(status).toMatch(/Nothing was imported./);
    expect(status).not.toMatch(/already has everything/);
  });

  it("tells the reader that apps are not imported, and where to get them", async () => {
    previewSetupBundleMock.mockResolvedValue({
      ...SUMMARY,
      extensions: [{ id: "acme.logs", name: "Logs", version: "1.2.0" }],
    });
    const user = userEvent.setup();
    render(<BackupSettingsSection />);
    await user.click(screen.getByRole("button", { name: /choose file/i }));
    await user.type(screen.getByLabelText("Bundle passphrase"), "pw");
    await user.click(screen.getByRole("button", { name: "Open" }));

    expect(screen.getByText(/apps are not imported/i).textContent).toMatch(/Logs/);
    expect(screen.queryByRole("checkbox", { name: /^apps/i })).toBeNull();
  });
});

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
