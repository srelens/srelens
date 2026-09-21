import { useRef, useState } from "react";
import {
  BUNDLE_MIN_PASSPHRASE,
  exportSetupBundle,
  importSetupBundle,
  importWroteSomething,
  notify,
  pickSetupBundle,
  previewSetupBundle,
  type BundleGroup,
  type BundleSummary,
  type ImportReport,
} from "@srelens/core";
import { Alert, Button, Checkbox, Field, Panel, TextInput } from "@srelens/ui-kit";
import { FailureAlert } from "../../lib/errorCopy";

/**
 * The `Backup` pane: move a whole srelens setup to another machine (#654).
 *
 * **The passphrase field is not optional and there is no way past it.** A
 * bundle worth carrying holds the credentials clusters connect with, so there
 * is no unencrypted form to offer and no "export anyway" to click. The pane
 * says that above the field rather than after the failure, and says the other
 * half too — that a lost passphrase is a lost file — because nothing here can
 * recover one.
 *
 * **Import is two steps with a preview between them, deliberately.** The
 * bundle's manifest lives inside its ciphertext (only the argon2id parameters
 * are readable), so opening the file is what reveals what it holds. Showing
 * that list before anything is written is the only way a reader can tell an
 * import of their own backup from an import of the wrong file — and it is why
 * `bundle_preview` takes the passphrase at all.
 *
 * **A wrong passphrase, a damaged file and a file that is not a bundle are
 * three answers, not one.** The backend already separates them
 * (`apps/desktop/src-tauri/src/bundle.rs`) because the fixes differ, and this
 * pane renders the string it is given rather than collapsing all three into
 * "Could not open". Same rule as everywhere else here: a failed call says what
 * failed; it never becomes a claim about the file's contents. Rendering an
 * empty preview for a refused read would be exactly that claim.
 *
 * **The report is rendered even when it is empty of writes.** An import that
 * wrote nothing because this machine already has everything looks identical to
 * one that silently failed, unless it says which it was.
 *
 * **Desktop-only, and the rail carries that** (see `SECTIONS` in
 * `../Settings.tsx`). `bundle_export`, `bundle_pick_file`, `bundle_preview`
 * and `bundle_import` are Tauri commands with no web half, and a bundle is
 * built from the desktop config directory and local kubeconfig files besides —
 * so on the web every control here would reject. The entry is not drawn, by
 * the same test `Security` is held to.
 */

/** The groups an import offers, in the order the backend applies them. */
const GROUPS: ReadonlyArray<{
  id: BundleGroup;
  label: string;
  describe: (summary: BundleSummary) => string;
  present: (summary: BundleSummary) => boolean;
}> = [
  {
    id: "kubeconfigs",
    label: "Clusters",
    describe: (s) => `${s.kubeconfigs.length} kubeconfig ${plural(s.kubeconfigs.length, "file")}`,
    present: (s) => s.kubeconfigs.length > 0,
  },
  {
    id: "settings",
    label: "Settings",
    describe: (s) =>
      `${s.settingsKeys} ${plural(s.settingsKeys, "preference")} — theme, layout, cluster names, colours and order`,
    present: (s) => s.settingsKeys > 0,
  },
  {
    id: "skills",
    label: "Assistant skills",
    describe: (s) => `${s.skills.length} ${plural(s.skills.length, "skill")}`,
    present: (s) => s.skills.length > 0,
  },
  {
    id: "prompts",
    label: "MCP prompts",
    describe: (s) => `${s.prompts.length} ${plural(s.prompts.length, "prompt")}`,
    present: (s) => s.prompts.length > 0,
  },
  {
    id: "secrets",
    label: "API keys",
    describe: (s) =>
      [
        s.secretKeys.length > 0
          ? `${s.secretKeys.join(", ")} API ${plural(s.secretKeys.length, "key")}`
          : "",
        s.hasMcpToken ? "the MCP token" : "",
      ]
        .filter(Boolean)
        .join(" and "),
    present: (s) => s.secretKeys.length > 0 || s.hasMcpToken,
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

/**
 * A failure, with the step that produced it. One `openError` rendered under a
 * single title made an import that failed on a read-only config directory read
 * as "That file could not be used" — blaming the bundle for something the
 * bundle had nothing to do with.
 */
interface Failure {
  stage: "choose" | "open" | "import";
  error: unknown;
}

const FAILURE_TITLES: Record<Failure["stage"], string> = {
  choose: "The file could not be selected",
  open: "That bundle could not be opened",
  import: "The setup could not be imported",
};

/** Every line of an import report that has something to say. */
export function reportLines(report: ImportReport): string[] {
  const lines: string[] = [];
  const say = (items: string[], text: (count: number, list: string) => string) => {
    if (items.length > 0) lines.push(text(items.length, items.join(", ")));
  };
  say(report.kubeconfigsAdded, (n, list) => `Added ${n} ${plural(n, "kubeconfig")}: ${list}.`);
  say(
    report.kubeconfigsAlreadyPresent,
    (n, list) =>
      `${n} ${plural(n, "kubeconfig")} ${n === 1 ? "was" : "were"} already here: ${list}.`,
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

export function BackupPane() {
  const [passphrase, setPassphrase] = useState("");
  const [repeated, setRepeated] = useState("");
  const [includeSecrets, setIncludeSecrets] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [exportError, setExportError] = useState<unknown>(null);

  const [path, setPath] = useState("");
  const [importPassphrase, setImportPassphrase] = useState("");
  const [opened, setOpened] = useState<Opened | null>(null);
  const [selected, setSelected] = useState<BundleGroup[]>([]);
  const [opening, setOpening] = useState(false);
  const [importing, setImporting] = useState(false);
  const [report, setReport] = useState<ImportReport | null>(null);
  const [failure, setFailure] = useState<Failure | null>(null);

  /**
   * Which selection the import panel is currently working on. Every async step
   * captures it and drops its own result if it has moved on — the reader can
   * pick a second file while the first is still being decrypted (argon2id is
   * deliberately slow), and without this the first file's manifest would land
   * under the second file's name, and `Import selected` would then send the
   * NEW path with the OLD file's groups.
   */
  const attempt = useRef(0);
  const begin = () => (attempt.current += 1);
  const current = (token: number) => attempt.current === token;

  const tooShort = passphrase.length > 0 && passphrase.length < BUNDLE_MIN_PASSPHRASE;
  const mismatch = repeated.length > 0 && repeated !== passphrase;
  const canExport =
    !exporting && passphrase.length >= BUNDLE_MIN_PASSPHRASE && repeated === passphrase;

  async function runExport() {
    setExporting(true);
    setExportError(null);
    try {
      const saved = await exportSetupBundle({ passphrase, includeSecrets });
      // `null` is a cancelled save dialog. Not a failure — and not a success
      // to announce either.
      if (saved === null) return;
      notify.success(`Setup exported to ${saved}`);
      setPassphrase("");
      setRepeated("");
    } catch (error) {
      setExportError(error);
    } finally {
      setExporting(false);
    }
  }

  async function choose() {
    const token = begin();
    try {
      const picked = await pickSetupBundle();
      if (!current(token)) return;
      if (picked === null) return;
      setPath(picked);
      setOpened(null);
      setReport(null);
      setFailure(null);
    } catch (error) {
      if (current(token)) setFailure({ stage: "choose", error });
    }
  }

  async function openBundle() {
    const token = begin();
    const selection = { path, passphrase: importPassphrase };
    setOpening(true);
    setFailure(null);
    try {
      const summary = await previewSetupBundle(selection.path, selection.passphrase);
      if (!current(token)) return;
      setOpened({ ...selection, summary });
      setSelected(GROUPS.filter((g) => g.present(summary)).map((g) => g.id));
    } catch (error) {
      if (!current(token)) return;
      setOpened(null);
      setFailure({ stage: "open", error });
    } finally {
      if (current(token)) setOpening(false);
    }
  }

  async function runImport() {
    if (opened === null) return;
    const token = begin();
    setImporting(true);
    setFailure(null);
    // The previous attempt's report goes with the error. Left standing, a
    // second import that fails would leave the first one's "Added 2
    // kubeconfigs" beside the failure alert, describing writes this attempt
    // did not make.
    setReport(null);
    try {
      // The selection that produced the manifest, not whatever is in the
      // fields now.
      const result = await importSetupBundle({
        path: opened.path,
        passphrase: opened.passphrase,
        groups: selected,
      });
      if (!current(token)) return;
      setReport(result);
      if (importWroteSomething(result)) {
        notify.success("Setup imported. Reload srelens to see the imported settings.");
      }
    } catch (error) {
      if (current(token)) setFailure({ stage: "import", error });
    } finally {
      if (current(token)) setImporting(false);
    }
  }

  const summary = opened?.summary ?? null;
  const offered = summary === null ? [] : GROUPS.filter((g) => g.present(summary));

  return (
    <div className="flex flex-col gap-4">
      <Panel title="Export">
        <p className="text-[0.75rem] leading-relaxed text-muted">
          Write this whole setup to one file: your clusters&apos; kubeconfigs, their names, colours
          and order, your preferences, assistant skills and MCP prompts. Kubeconfigs travel by
          contents, not by path, so they land wherever srelens looks on the other machine.
        </p>
        <p className="mt-2 text-[0.75rem] leading-relaxed text-muted">
          The file is always encrypted — it carries the credentials your clusters connect with — and
          what is readable on the outside is only the key-derivation parameters, so a bundle sitting
          in a downloads folder does not list your clusters. Choose a passphrase you can type on the
          other machine: <strong>there is no way to recover the file without it</strong>.
        </p>
        <div className="mt-3 max-w-sm">
          <Field label="Passphrase" error={tooShort ? `At least ${BUNDLE_MIN_PASSPHRASE} characters.` : undefined}>
            <TextInput type="password" value={passphrase} onValueChange={setPassphrase} />
          </Field>
          <Field
            label="Confirm passphrase"
            error={mismatch ? "The two passphrases don't match." : undefined}
          >
            <TextInput type="password" value={repeated} onValueChange={setRepeated} />
          </Field>
          <Checkbox
            checked={includeSecrets}
            onChange={setIncludeSecrets}
            label="Include the assistant's API keys and the MCP token"
            className="mt-2"
          />
          <div className="mt-3">
            <Button disabled={!canExport} onClick={() => void runExport()}>
              {exporting ? "Exporting…" : "Export setup…"}
            </Button>
          </div>
        </div>
        {exportError !== null && (
          <FailureAlert
            tone="sev"
            title="The setup could not be exported"
            error={exportError}
            className="mt-3"
          />
        )}
      </Panel>

      <Panel title="Import">
        <p className="text-[0.75rem] leading-relaxed text-muted">
          Importing only adds. A cluster whose kubeconfig you already have is not duplicated, a
          skill or prompt you have edited here is kept as it is, and an API key this machine already
          holds is not replaced. Nothing is deleted.
        </p>
        <div className="mt-3 flex max-w-sm flex-col gap-2">
          <div className="flex flex-wrap items-center gap-2">
            <Button variant="secondary" onClick={() => void choose()}>
              Choose file…
            </Button>
            {path !== "" && (
              <span className="min-w-0 truncate text-[0.75rem] text-muted" title={path}>
                {path}
              </span>
            )}
          </div>
          {path !== "" && opened === null && (
            <>
              <Field label="Bundle passphrase">
                <TextInput
                  type="password"
                  value={importPassphrase}
                  onValueChange={setImportPassphrase}
                  onEnter={() => void openBundle()}
                />
              </Field>
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

        {failure !== null && (
          // The detail is rendered verbatim — the backend already tells a wrong
          // passphrase from a damaged file from one that is not a bundle at
          // all, and the remedy differs for each. The TITLE names the step,
          // so an import that failed on a read-only config directory is not
          // announced as a problem with the file.
          <FailureAlert
            tone="sev"
            title={FAILURE_TITLES[failure.stage]}
            error={failure.error}
            className="mt-3"
          />
        )}

        {summary !== null && (
          <div className="mt-3 flex max-w-sm flex-col gap-2">
            <p className="text-[0.75rem] leading-relaxed text-muted">
              Exported {summary.created || "at an unknown time"}
              {summary.appVersion !== "" && ` by srelens ${summary.appVersion}`}. Choose what to
              bring over:
            </p>
            {offered.map((group) => (
              <Checkbox
                key={group.id}
                checked={selected.includes(group.id)}
                onChange={(on) =>
                  setSelected((current) =>
                    on ? [...current, group.id] : current.filter((entry) => entry !== group.id),
                  )
                }
                label={
                  <span>
                    <strong>{group.label}</strong>
                    <span className="text-muted"> — {group.describe(summary)}</span>
                  </span>
                }
              />
            ))}
            {summary.extensions.length > 0 && (
              <Alert tone="info" title="Apps are not imported">
                This setup had {summary.extensions.length} {plural(summary.extensions.length, "app")}{" "}
                installed ({summary.extensions.map((app) => app.name).join(", ")}). Apps are code
                with capability grants, so install them again from Settings → Apps and review their
                permissions on this machine.
              </Alert>
            )}
            <div>
              <Button disabled={importing || selected.length === 0} onClick={() => void runImport()}>
                {importing ? "Importing…" : "Import selected"}
              </Button>
            </div>
          </div>
        )}

        {report !== null && (
          <div className="mt-3 flex flex-col gap-1 text-[0.75rem] leading-relaxed" role="status">
            {/* The detail lines and the verdict are separate questions.
                `reportLines` includes the skips — "already here", "kept your
                own version" — so a report made up entirely of them is not
                empty, but nothing was written. Keying the reload advice off
                the line count told such a reader to restart srelens to pick up
                changes that were never made. */}
            {reportLines(report).map((line) => (
              <p key={line}>{line}</p>
            ))}
            {importWroteSomething(report) ? (
              <p className="text-muted">Reload srelens to pick up the imported settings.</p>
            ) : (
              <p>Nothing to import — this machine already has everything in that bundle.</p>
            )}
          </div>
        )}
      </Panel>
    </div>
  );
}
