import { describe, it, expect, vi, beforeEach } from "vitest";

const { invokeCommandMock } = vi.hoisted(() => ({ invokeCommandMock: vi.fn() }));
vi.mock("../transport/transport", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../transport/transport")>();
  return { ...actual, invokeCommand: invokeCommandMock };
});

import {
  BUNDLE_MIN_PASSPHRASE,
  defaultBundleFilename,
  exportSetupBundle,
  importSetupBundle,
  importWroteSomething,
  pickSetupBundle,
  previewSetupBundle,
  type BundleSummary,
  type ImportReport,
} from "./setupBundle";

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

describe("setupBundle", () => {
  beforeEach(() => invokeCommandMock.mockReset());

  it("suggests a dated file name the reader can tell two backups apart by", () => {
    // Month is zero-based in Date; a backup named `srelens-setup-2026-08-21`
    // on the 21st of September is worse than no date at all.
    expect(defaultBundleFilename(new Date(2026, 8, 21))).toBe("srelens-setup-2026-09-21.srelens");
    expect(defaultBundleFilename(new Date(2026, 0, 5))).toBe("srelens-setup-2026-01-05.srelens");
  });

  it("exports with the secrets choice the caller made, never a default of its own", async () => {
    invokeCommandMock.mockResolvedValue("/tmp/setup.srelens");

    await expect(
      exportSetupBundle({ passphrase: "correct horse", includeSecrets: true }),
    ).resolves.toBe("/tmp/setup.srelens");

    expect(invokeCommandMock).toHaveBeenCalledWith("bundle_export", {
      passphrase: "correct horse",
      includeSecrets: true,
      filename: defaultBundleFilename(),
    });
  });

  it("reports a cancelled save dialog as null, not as a saved file", async () => {
    // The backend answers `None` when the reader cancels. Anything that
    // turned that into a truthy value would toast a success for a file that
    // was never written.
    invokeCommandMock.mockResolvedValue(null);
    await expect(
      exportSetupBundle({ passphrase: "correct horse", includeSecrets: false }),
    ).resolves.toBeNull();

    invokeCommandMock.mockResolvedValue(undefined);
    await expect(pickSetupBundle()).resolves.toBeNull();
  });

  it("passes the path and passphrase straight through to the preview", async () => {
    const summary: BundleSummary = {
      created: "2026-09-21T10:00:00Z",
      appVersion: "0.15.0",
      settingsKeys: 4,
      kubeconfigs: ["config"],
      skills: [],
      prompts: [],
      extensions: [],
      secretKeys: [],
      hasMcpToken: false,
    };
    invokeCommandMock.mockResolvedValue(summary);

    await expect(previewSetupBundle("/tmp/a.srelens", "pw")).resolves.toEqual(summary);
    expect(invokeCommandMock).toHaveBeenCalledWith("bundle_preview", {
      path: "/tmp/a.srelens",
      passphrase: "pw",
    });
  });

  it("sends only the groups the reader selected", async () => {
    invokeCommandMock.mockResolvedValue(EMPTY_REPORT);

    await importSetupBundle({
      path: "/tmp/a.srelens",
      passphrase: "pw",
      groups: ["kubeconfigs", "settings"],
    });

    expect(invokeCommandMock).toHaveBeenCalledWith("bundle_import", {
      path: "/tmp/a.srelens",
      passphrase: "pw",
      groups: ["kubeconfigs", "settings"],
    });
  });

  it("does not let a failed preview resolve as an empty bundle", async () => {
    // A rejected `bundle_preview` must reach the caller as a rejection: the
    // one thing the UI must never do is render "this bundle is empty" for a
    // wrong passphrase.
    // Tauri rejects with a bare string, not an Error, so this catches by hand
    // rather than through a matcher that assumes an Error shape.
    invokeCommandMock.mockImplementationOnce(() =>
      Promise.reject("that passphrase does not open this bundle"),
    );
    let rejected: unknown;
    let resolved = false;
    try {
      await previewSetupBundle("/tmp/a.srelens", "wrong");
      resolved = true;
    } catch (error) {
      rejected = error;
    }
    expect(resolved).toBe(false);
    expect(String(rejected)).toMatch(/does not open this bundle/);
  });

  it("knows an import that wrote nothing from one that wrote something", () => {
    expect(importWroteSomething(EMPTY_REPORT)).toBe(false);
    // Files the import deliberately left alone are not writes: a report of
    // nothing but skips is still "nothing to import".
    expect(
      importWroteSomething({
        ...EMPTY_REPORT,
        kubeconfigsAlreadyPresent: ["config"],
        skillsKeptLocal: ["triage.md"],
      }),
    ).toBe(false);
    expect(importWroteSomething({ ...EMPTY_REPORT, kubeconfigsAdded: ["config"] })).toBe(true);
    expect(importWroteSomething({ ...EMPTY_REPORT, secretsWritten: ["anthropic"] })).toBe(true);
  });

  it("mirrors the backend's passphrase minimum", () => {
    // The backend is the enforcer; this constant exists so the form can say
    // the rule before a submission is refused. Drifting apart would show a
    // hint that contradicts the error.
    expect(BUNDLE_MIN_PASSPHRASE).toBe(8);
  });
});
