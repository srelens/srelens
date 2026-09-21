import { invokeCommand } from "../transport/transport";

/** A group of a setup bundle, selectable on import. */
export type BundleGroup = "settings" | "kubeconfigs" | "skills" | "prompts" | "secrets";

/**
 * What a bundle holds, as reported by `bundle_preview`. Secrets are described
 * by provider slug only — the backend never puts a key's value in here.
 */
export interface BundleSummary {
  created: string;
  appVersion: string;
  settingsKeys: number;
  kubeconfigs: string[];
  skills: string[];
  prompts: string[];
  extensions: Array<{ id: string; name: string; version: string }>;
  secretKeys: string[];
  hasMcpToken: boolean;
}

/** Exactly what an import wrote, and what it left alone. */
export interface ImportReport {
  settingsWritten: string[];
  kubeconfigsAdded: string[];
  kubeconfigsAlreadyPresent: string[];
  /** Bundled files that do not parse as a kubeconfig, so were not written. */
  kubeconfigsRejected: string[];
  skillsAdded: string[];
  skillsKeptLocal: string[];
  promptsAdded: string[];
  promptsKeptLocal: string[];
  secretsWritten: string[];
}

/** The passphrase minimum the backend enforces; mirrored so the form can say so. */
export const BUNDLE_MIN_PASSPHRASE = 8;

/** `srelens-setup-YYYY-MM-DD.srelens`, the save dialog's suggested name. */
export function defaultBundleFilename(now: Date = new Date()): string {
  const part = (value: number) => String(value).padStart(2, "0");
  return `srelens-setup-${now.getFullYear()}-${part(now.getMonth() + 1)}-${part(now.getDate())}.srelens`;
}

/**
 * Write an encrypted setup bundle. Resolves to the saved path, or `null` when
 * the reader cancelled the save dialog — which is not a failure and must not
 * be reported as one.
 */
export async function exportSetupBundle(options: {
  passphrase: string;
  includeSecrets: boolean;
  filename?: string;
}): Promise<string | null> {
  return (
    (await invokeCommand<string | null>("bundle_export", {
      passphrase: options.passphrase,
      includeSecrets: options.includeSecrets,
      filename: options.filename ?? defaultBundleFilename(),
    })) ?? null
  );
}

/** Choose a bundle file. `null` when the reader cancelled. */
export async function pickSetupBundle(): Promise<string | null> {
  return (await invokeCommand<string | null>("bundle_pick_file")) ?? null;
}

/** Open a bundle and describe it, writing nothing. */
export async function previewSetupBundle(path: string, passphrase: string): Promise<BundleSummary> {
  return await invokeCommand<BundleSummary>("bundle_preview", { path, passphrase });
}

/** Apply the selected groups of a bundle to this machine. */
export async function importSetupBundle(options: {
  path: string;
  passphrase: string;
  groups: BundleGroup[];
}): Promise<ImportReport> {
  return await invokeCommand<ImportReport>("bundle_import", options);
}

/** Whether an import report describes anything at all having been written. */
export function importWroteSomething(report: ImportReport): boolean {
  return (
    report.settingsWritten.length > 0 ||
    report.kubeconfigsAdded.length > 0 ||
    report.skillsAdded.length > 0 ||
    report.promptsAdded.length > 0 ||
    report.secretsWritten.length > 0
  );
}
