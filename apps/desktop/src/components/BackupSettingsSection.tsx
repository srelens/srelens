import { useRef, useState } from "react";
import { Button, TextInput } from "../ui";
import { notify } from "@srelens/core";
import {
  BUNDLE_MIN_PASSPHRASE,
  exportSetupBundle,
  importSetupBundle,
  importWroteSomething,
  pickSetupBundle,
  previewSetupBundle,
  type BundleGroup,
  type BundleSummary,
  type ImportReport,
} from "@srelens/core";

/** The groups an import offers, in the order they are applied. */
const GROUPS: Array<{ id: BundleGroup; label: string; describe: (s: BundleSummary) => string }> = [
  {
    id: "kubeconfigs",
    label: "Clusters",
    describe: (s) => `${s.kubeconfigs.length} kubeconfig ${plural(s.kubeconfigs.length, "file")}`,
  },
  {
    id: "settings",
    label: "Settings",
    describe: (s) =>
      `${s.settingsKeys} ${plural(s.settingsKeys, "preference")} — theme, layout, cluster names, colours and order`,
  },
  {
    id: "skills",
    label: "Assistant skills",
    describe: (s) => `${s.skills.length} ${plural(s.skills.length, "skill")}`,
  },
  {
    id: "prompts",
    label: "MCP prompts",
    describe: (s) => `${s.prompts.length} ${plural(s.prompts.length, "prompt")}`,
  },
  {
    id: "secrets",
    label: "API keys",
    describe: (s) =>
      [
        s.secretKeys.length > 0 ? `${s.secretKeys.join(", ")} API ${plural(s.secretKeys.length, "key")}` : "",
        s.hasMcpToken ? "the MCP token" : "",
      ]
        .filter(Boolean)
        .join(" and "),
  },
];

function plural(count: number, word: string): string {
  return count === 1 ? word : `${word}s`;
}

/**
 * A bundle that has been opened, together with the selection that opened it.
 * The path and passphrase are captured here rather than read back off the form
 * at import time: the fields stay editable while the preview is on screen, and
 * importing with a passphrase the reader has since retyped would send the new
 * one against the manifest the old one produced.
 */
interface Opened {
  path: string;
  passphrase: string;
  summary: BundleSummary;
}

/** Groups a bundle actually carries — an empty group is not worth a checkbox. */
function presentGroups(summary: BundleSummary): BundleGroup[] {
  const has: Record<BundleGroup, boolean> = {
    kubeconfigs: summary.kubeconfigs.length > 0,
    settings: summary.settingsKeys > 0,
    skills: summary.skills.length > 0,
    prompts: summary.prompts.length > 0,
    secrets: summary.secretKeys.length > 0 || summary.hasMcpToken,
  };
  return GROUPS.filter((g) => has[g.id]).map((g) => g.id);
}

/**
 * The one-line verdict under the detail lines, when nothing was written.
 *
 * "this machine already has everything in that bundle" is a claim about the
 * machine, and it is only true when every skip was a skip because the thing was
 * already here. A bundled file that did not parse as a kubeconfig was not
 * skipped for that reason — it was refused — so saying the machine already has
 * it is exactly the failed-read-rendered-as-a-fact this codebase forbids. The
 * rejection lines above already say what happened; the verdict stays neutral.
 */
export function noChangeVerdict(report: ImportReport, wholeBundle: boolean): string {
  // "everything in that bundle" is also a claim about the BUNDLE, and an
  // import only covers the groups that were ticked. A reader who unticks the
  // clusters and imports the settings alone has been told nothing about the
  // clusters, so the bundle-wide sentence is only available when every group
  // the bundle offered was selected.
  return report.kubeconfigsRejected.length > 0 || !wholeBundle
    ? "Nothing was imported."
    : "Nothing to import — this machine already has everything in that bundle.";
}

/**
 * A finished import, with the coverage it ran under.
 *
 * The coverage is recorded HERE rather than recomputed from the live
 * checkboxes at render time. The group boxes stay editable after an import
 * finishes, so a verdict derived from them changed retroactively: after a
 * no-op partial import said "Nothing was imported.", ticking the group that
 * had been left out flipped the same status block to "this machine already has
 * everything in that bundle" — a claim about an import that never ran.
 */
interface Outcome {
  report: ImportReport;
  wholeBundle: boolean;
}

/** Every line of an import report that has something to say. */
export function reportLines(report: ImportReport): string[] {
  const lines: string[] = [];
  const say = (items: string[], text: (count: number, list: string) => string) => {
    if (items.length > 0) lines.push(text(items.length, items.join(", ")));
  };
  say(report.kubeconfigsAdded, (n, list) => `Added ${n} ${plural(n, "kubeconfig")}: ${list}.`);
  say(
    report.kubeconfigsAlreadyPresent,
    (n, list) => `${n} ${plural(n, "kubeconfig")} ${n === 1 ? "was" : "were"} already here: ${list}.`,
  );
  say(
    report.kubeconfigsRejected,
    (n, list) =>
      `${n} bundled ${plural(n, "file")} ${n === 1 ? "is" : "are"} not a kubeconfig and ${n === 1 ? "was" : "were"} not imported: ${list}.`,
  );
  say(report.settingsWritten, (n) => `Applied ${n} ${plural(n, "preference")}.`);
  say(report.skillsAdded, (n, list) => `Added ${n} ${plural(n, "skill")}: ${list}.`);
  say(
    report.skillsKeptLocal,
    (n, list) => `Kept your own version of ${n} ${plural(n, "skill")}: ${list}.`,
  );
  say(report.promptsAdded, (n, list) => `Added ${n} ${plural(n, "prompt")}: ${list}.`);
  say(
    report.promptsKeptLocal,
    (n, list) => `Kept your own version of ${n} ${plural(n, "prompt")}: ${list}.`,
  );
  say(report.secretsWritten, (_n, list) => `Stored ${list}.`);
  return lines;
}

/**
 * Settings → Backup: move a whole srelens setup to another machine.
 *
 * The file is always encrypted, because a bundle worth carrying holds cluster
 * credentials — so the passphrase field is not optional and there is no
 * "export unencrypted" escape hatch to click past.
 *
 * Import is deliberately two steps with a preview between them. The manifest
 * lives inside the ciphertext, so opening the file is what reveals what it
 * holds; showing that list before anything is written is the only way the
 * reader can tell an import of their own backup from an import of the wrong
 * file. Nothing here deletes, and nothing overwrites a local file silently —
 * the report names what was kept.
 */
export function BackupSettingsSection() {
  const [passphrase, setPassphrase] = useState("");
  const [confirm, setConfirm] = useState("");
  const [includeSecrets, setIncludeSecrets] = useState(false);
  const [exporting, setExporting] = useState(false);

  const [path, setPath] = useState("");
  const [importPassphrase, setImportPassphrase] = useState("");
  const [opened, setOpened] = useState<Opened | null>(null);
  const [selected, setSelected] = useState<BundleGroup[]>([]);
  const [opening, setOpening] = useState(false);
  const [importing, setImporting] = useState(false);
  const [outcome, setOutcome] = useState<Outcome | null>(null);
  const [error, setError] = useState("");

  /**
   * Which selection the import panel is working on. Every async step captures
   * it and drops its own result if it has moved on — the reader can pick a
   * second file while the first is still being decrypted (argon2id is
   * deliberately slow), and without this the first file's manifest would land
   * under the second file's name, and Import would send the NEW path with the
   * OLD file's groups.
   */
  const attempt = useRef(0);
  const begin = () => (attempt.current += 1);
  const current = (token: number) => attempt.current === token;

  const summary = opened?.summary ?? null;
  const offered = summary === null ? [] : presentGroups(summary);
  /** Whether the import that produced `report` covered every group offered. */
  const wholeBundle = offered.length > 0 && offered.every((id) => selected.includes(id));

  const tooShort = passphrase.length > 0 && passphrase.length < BUNDLE_MIN_PASSPHRASE;
  const mismatch = confirm.length > 0 && confirm !== passphrase;
  const canExport =
    !exporting && passphrase.length >= BUNDLE_MIN_PASSPHRASE && confirm === passphrase;

  async function runExport() {
    setExporting(true);
    try {
      const saved = await exportSetupBundle({ passphrase, includeSecrets });
      // A cancelled save dialog is not a failure — and must not read as a
      // successful export either.
      if (saved === null) return;
      notify.success(`Setup exported to ${saved}`);
      setPassphrase("");
      setConfirm("");
    } catch (e) {
      notify.error(String(e));
    } finally {
      setExporting(false);
    }
  }

  async function choose() {
    // No `begin()` yet. Invalidating the pending operation BEFORE knowing
    // whether a file was actually picked meant a cancelled picker left an
    // in-flight `openBundle` unable to clear `opening` from its own guarded
    // `finally` — and nothing here cleared it either, because the cancel path
    // returns first. The token advances only once the selection really changes.
    let picked: string | null;
    try {
      picked = await pickSetupBundle();
    } catch (e) {
      notify.error(String(e));
      return;
    }
    if (picked === null) return;

    begin();
    setPath(picked);
    setOpened(null);
    setOutcome(null);
    setError("");
    // The superseded `openBundle`'s `finally` is guarded by its own token, so
    // it will NOT reset this — picking a second file mid-decrypt otherwise
    // left `opening` true for good and disabled the Open button on the file
    // the reader had just chosen.
    //
    // `importing` is deliberately NOT cleared here: this button is disabled
    // while an import is in flight (see the render). Clearing it would only
    // hide a `bundle_import` that is still running — the token suppresses the
    // stale UI update, it does not cancel the command — and would let a second
    // import start over a backend that applies files, settings and secrets in
    // separate steps with no import-wide lock.
    setOpening(false);
  }

  async function openBundle() {
    const token = begin();
    const selection = { path, passphrase: importPassphrase };
    setOpening(true);
    setError("");
    try {
      const summary = await previewSetupBundle(selection.path, selection.passphrase);
      // Dropped if the reader has since picked another file: argon2id is
      // deliberately slow, so the first file's manifest could otherwise land
      // under the second file's name and be imported with its groups.
      if (!current(token)) return;
      setOpened({ ...selection, summary });

      setSelected(presentGroups(summary));
    } catch (e) {
      if (!current(token)) return;
      // The backend distinguishes a wrong passphrase from a file that is not
      // a bundle at all; both reach the reader verbatim, because the fixes
      // are different.
      setOpened(null);
      setError(String(e));
    } finally {
      if (current(token)) setOpening(false);
    }
  }

  async function runImport() {
    if (opened === null) return;
    const token = begin();
    setImporting(true);
    setError("");
    // The previous attempt's report goes with the error. Left standing, a
    // second import that fails would leave the first one's "Added 2
    // kubeconfigs" beside the failure alert, describing writes this attempt
    // did not make.
    setOutcome(null);
    try {
      // The selection that produced the manifest, not whatever is in the
      // fields now.
      const result = await importSetupBundle({
        path: opened.path,
        passphrase: opened.passphrase,
        groups: selected,
      });
      if (!current(token)) return;
      setOutcome({ report: result, wholeBundle });
      if (importWroteSomething(result)) {
        notify.success("Setup imported. Reload srelens to see the imported settings.");
      } else {
        notify.info(noChangeVerdict(result, wholeBundle));
      }
    } catch (e) {
      if (current(token)) setError(String(e));
    } finally {
      if (current(token)) setImporting(false);
    }
  }

  function toggle(group: BundleGroup, on: boolean) {
    setSelected((current) =>
      on ? [...current, group] : current.filter((entry) => entry !== group),
    );
  }

  return (
    <div className="flex flex-col gap-6">
      <div>
        <h2 className="text-lg font-semibold">Backup</h2>
        <p className="text-sm text-muted-foreground">
          Move this setup to another machine: your clusters' kubeconfigs, their names, colours and
          order, your preferences, assistant skills and MCP prompts, in one encrypted file.
        </p>
      </div>

      <section className="flex flex-col gap-3">
        <h3 className="text-sm font-medium">Export</h3>
        <p className="text-sm text-muted-foreground">
          The file is always encrypted — it carries the credentials your clusters connect with.
          Choose a passphrase you can type on the other machine; there is no way to recover the
          file without it.
        </p>
        <div className="flex max-w-md flex-col gap-2">
          <TextInput
            type="password"
            aria-label="Passphrase"
            placeholder={`Passphrase (at least ${BUNDLE_MIN_PASSPHRASE} characters)`}
            value={passphrase}
            onValueChange={setPassphrase}
          />
          <TextInput
            type="password"
            aria-label="Confirm passphrase"
            placeholder="Confirm passphrase"
            value={confirm}
            onValueChange={setConfirm}
          />
          {tooShort && (
            <p className="text-sm text-amber-600 dark:text-amber-500">
              At least {BUNDLE_MIN_PASSPHRASE} characters.
            </p>
          )}
          {mismatch && (
            <p className="text-sm text-amber-600 dark:text-amber-500">
              The two passphrases don't match.
            </p>
          )}
          <label className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              className="accent-primary"
              checked={includeSecrets}
              onChange={(e) => setIncludeSecrets(e.target.checked)}
            />
            <span>Include the assistant's API keys and the MCP token</span>
          </label>
          <div>
            <Button disabled={!canExport} onClick={() => void runExport()}>
              {exporting ? "Exporting…" : "Export setup…"}
            </Button>
          </div>
        </div>
      </section>

      <section className="flex flex-col gap-3 border-t border-border pt-4">
        <h3 className="text-sm font-medium">Import</h3>
        <p className="text-sm text-muted-foreground">
          Importing only adds: a cluster you already have is not duplicated, and a skill or prompt
          you have edited here is kept as it is. Nothing is deleted.
        </p>
        <div className="flex max-w-md flex-col gap-2">
          <div className="flex items-center gap-2">
            <Button disabled={importing} onClick={() => void choose()}>
              Choose file…
            </Button>
            {path && (
              <span className="truncate text-sm text-muted-foreground" title={path}>
                {path}
              </span>
            )}
          </div>
          {path && !summary && (
            <>
              <TextInput
                type="password"
                aria-label="Bundle passphrase"
                placeholder="The passphrase this file was exported with"
                value={importPassphrase}
                onValueChange={setImportPassphrase}
              />
              <div>
                <Button
                  disabled={opening || importPassphrase.length === 0}
                  onClick={() => void openBundle()}
                >
                  {opening ? "Opening…" : "Open"}
                </Button>
              </div>
            </>
          )}
        </div>

        {error && (
          <p className="text-sm text-destructive" role="alert">
            {error}
          </p>
        )}

        {summary && (
          <div className="flex max-w-md flex-col gap-2">
            <p className="text-sm text-muted-foreground">
              Exported {summary.created || "at an unknown time"}
              {summary.appVersion && ` by srelens ${summary.appVersion}`}. Choose what to bring
              over:
            </p>
            {GROUPS.filter((group) => offered.includes(group.id)).map((group) => (
              <label key={group.id} className="flex items-start gap-2 text-sm">
                <input
                  type="checkbox"
                  className="accent-primary mt-1"
                  checked={selected.includes(group.id)}
                  onChange={(e) => toggle(group.id, e.target.checked)}
                />
                <span>
                  <strong>{group.label}</strong>
                  <span className="text-muted-foreground"> — {group.describe(summary)}</span>
                </span>
              </label>
            ))}
            {summary.extensions.length > 0 && (
              <p className="text-sm text-muted-foreground">
                This setup also had {summary.extensions.length}{" "}
                {plural(summary.extensions.length, "app")} installed (
                {summary.extensions.map((app) => app.name).join(", ")}). Apps are not imported —
                install them again from Settings → Apps so their permissions are reviewed on this
                machine.
              </p>
            )}
            <div>
              <Button
                disabled={importing || selected.length === 0}
                onClick={() => void runImport()}
              >
                {importing ? "Importing…" : "Import selected"}
              </Button>
            </div>
          </div>
        )}

        {outcome !== null && (
          <div className="flex flex-col gap-1 text-sm" role="status">
            {/* The detail lines and the verdict are separate questions.
                `reportLines` includes the skips — "already here", "kept your
                own version" — so a report made up entirely of them is not
                empty, but nothing was written. Keying the reload advice off
                the line count told such a reader to restart srelens to pick up
                changes that were never made. */}
            {reportLines(outcome.report).map((line) => (
              <p key={line}>{line}</p>
            ))}
            {importWroteSomething(outcome.report) ? (
              <p className="text-muted-foreground">
                Reload srelens (or restart it) to pick up the imported settings.
              </p>
            ) : (
              <p>{noChangeVerdict(outcome.report, outcome.wholeBundle)}</p>
            )}
          </div>
        )}
      </section>
    </div>
  );
}
