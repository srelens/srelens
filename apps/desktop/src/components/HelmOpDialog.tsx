import { lazy, Suspense, useEffect, useMemo, useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog";
import { Button, TextInput, Field, Select, Spinner } from "../ui";
import { DiffView } from "./DiffView";
import { helmTemplate, helmSearchRepo, diffTextLines, type HelmChartRef } from "@srelens/core";
import type { DiffDoc } from "@srelens/core";

const CodeEditor = lazy(() => import("../ui/CodeEditor").then((m) => ({ default: m.CodeEditor })));

export interface HelmOpRelease {
  name: string;
  namespace: string;
  chart: string;
  chartVersion: string;
  valuesYaml: string;
  manifest: string;
}

function buildArgs(
  mode: "install" | "upgrade",
  name: string,
  chart: string,
  namespace: string,
  version: string,
): string[] {
  const args = [mode === "install" ? "install" : "upgrade", name, chart];
  if (namespace) {
    args.push("--namespace", namespace);
    if (mode === "install") args.push("--create-namespace");
  }
  if (version) args.push("--version", version);
  return args;
}

/** Distinct `entry.name` values, in first-seen order. */
function distinctRefs(entries: HelmChartRef[]): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const e of entries) {
    if (!seen.has(e.name)) {
      seen.add(e.name);
      out.push(e.name);
    }
  }
  return out;
}

export function HelmOpDialog({
  context,
  mode,
  release,
  onRun,
  onClose,
}: {
  context: string;
  mode: "install" | "upgrade";
  release?: HelmOpRelease;
  onRun: (r: { name: string; chart: string; namespace: string; values: string; helmArgs: string[] }) => void;
  onClose: () => void;
}) {
  const [name, setName] = useState(release?.name ?? "");
  const [chart, setChart] = useState(release?.chart ?? "");
  const [namespace, setNamespace] = useState(release?.namespace ?? "");
  const [values, setValues] = useState(release?.valuesYaml ?? "");
  const [diff, setDiff] = useState<DiffDoc | null>(null);
  const [previewError, setPreviewError] = useState("");
  const [previewing, setPreviewing] = useState(false);

  // Helm's release metadata carries only the chart's bare name (e.g.
  // "cert-manager"), never its source ref — so for upgrades, resolve the ref
  // (and its available versions) from the user's configured repos.
  const lookupName = mode === "upgrade" ? release?.chart : undefined;
  const [entries, setEntries] = useState<HelmChartRef[]>([]);
  const [searchError, setSearchError] = useState("");

  useEffect(() => {
    if (!lookupName) {
      setEntries([]);
      setSearchError("");
      return;
    }
    let cancelled = false;
    void helmSearchRepo(context, lookupName).then((res) => {
      if (cancelled) return;
      if (res.error) {
        setSearchError(res.error);
        setEntries([]);
        return;
      }
      setSearchError("");
      setEntries(res.entries ?? []);
    });
    return () => {
      cancelled = true;
    };
  }, [context, lookupName]);

  const refs = useMemo(() => distinctRefs(entries), [entries]);
  function versionsFor(ref: string): HelmChartRef[] {
    return entries.filter((e) => e.name === ref);
  }

  const [selectedRef, setSelectedRef] = useState("");
  const [selectedVersion, setSelectedVersion] = useState("");

  // Re-derive the default ref/version whenever the resolved entries change —
  // default to the single ref (or the first, when several repos match), and
  // to the release's currently-installed version when it's still available.
  useEffect(() => {
    if (refs.length === 0) {
      setSelectedRef("");
      setSelectedVersion("");
      return;
    }
    const ref = refs[0];
    setSelectedRef(ref);
    const versions = entries.filter((e) => e.name === ref).map((e) => e.version);
    const current = release?.chartVersion;
    setSelectedVersion(current && versions.includes(current) ? current : (versions[0] ?? ""));
  }, [refs, entries, release?.chartVersion]);

  // The chart ref actually used for preview/run: the resolved ref once repos
  // yield a match, otherwise the free-text fallback the user typed.
  const resolvedChart = refs.length > 0 ? selectedRef : chart;

  const title = mode === "install" ? "Install chart" : `Upgrade ${release?.name ?? ""}`;
  const confirmLabel = mode === "install" ? "Install" : "Upgrade";

  async function preview() {
    setPreviewing(true);
    setPreviewError("");
    const res = await helmTemplate(context, { name, chart: resolvedChart, namespace, values, version: selectedVersion || undefined });
    setPreviewing(false);
    if (res.error) {
      setPreviewError(res.error);
      return;
    }
    const rows = diffTextLines(release?.manifest ?? "", res.output ?? "");
    setDiff({
      kind: "Helm",
      name,
      namespace: namespace || null,
      exists: Boolean(release),
      changed: rows.some((r) => r.tag !== "same"),
      rows,
      currentResourceVersion: null,
    });
  }

  function run() {
    if (!name.trim() || !resolvedChart.trim()) {
      setPreviewError("Release name and chart reference are required.");
      return;
    }
    onRun({
      name,
      chart: resolvedChart,
      namespace,
      values,
      helmArgs: buildArgs(mode, name, resolvedChart, namespace, selectedVersion),
    });
  }

  function invalidatePreview() {
    setDiff(null);
    setPreviewError("");
  }

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <DialogContent className="sm:max-w-3xl">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription className="sr-only">
            {mode === "install" ? "Install a Helm chart." : "Upgrade this Helm release."}
          </DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-4">
          <div className="grid grid-cols-2 gap-3">
            <Field label="Release name">
              <TextInput
                aria-label="Release name"
                value={name}
                onValueChange={(v) => {
                  setName(v);
                  invalidatePreview();
                }}
                disabled={mode === "upgrade"}
                placeholder="release name"
              />
            </Field>
            <Field label="Namespace">
              <TextInput
                aria-label="Namespace"
                value={namespace}
                onValueChange={(v) => {
                  setNamespace(v);
                  invalidatePreview();
                }}
                placeholder="namespace"
              />
            </Field>
          </div>

          <div className="grid grid-cols-2 gap-3">
            <Field
              label="Chart reference"
              hint={
                refs.length === 0
                  ? mode === "upgrade"
                    ? searchError
                      ? "Couldn't search configured repos; enter a full ref (e.g. bitnami/nginx)."
                      : "Helm doesn't record the chart's source. If this release came from a repo, enter a full ref (e.g. bitnami/nginx)."
                    : undefined
                  : undefined
              }
            >
              {refs.length > 0 ? (
                <Select
                  aria-label="Chart reference"
                  value={selectedRef}
                  onValueChange={(v) => {
                    setSelectedRef(v);
                    const versions = versionsFor(v).map((e) => e.version);
                    const current = release?.chartVersion;
                    setSelectedVersion(current && versions.includes(current) ? current : (versions[0] ?? ""));
                    invalidatePreview();
                  }}
                  options={refs.map((r) => ({ value: r }))}
                />
              ) : (
                <TextInput
                  aria-label="Chart reference"
                  value={chart}
                  onValueChange={(v) => {
                    setChart(v);
                    invalidatePreview();
                  }}
                  placeholder="repo/chart, oci://…, or ./path"
                />
              )}
            </Field>

            {selectedVersion && (
              <Field label="Version">
                <Select
                  aria-label="Version"
                  value={selectedVersion}
                  onValueChange={(v) => {
                    setSelectedVersion(v);
                    invalidatePreview();
                  }}
                  options={versionsFor(selectedRef).map((e) => ({
                    value: e.version,
                    label: e.version === release?.chartVersion ? `${e.version} (current)` : e.version,
                  }))}
                />
              </Field>
            )}
          </div>

          {mode === "upgrade" && release?.chartVersion && (
            <p className="text-xs text-muted-foreground">Currently installed: {release.chartVersion}</p>
          )}

          <Field
            label="Values (YAML)"
            action={
              <Button
                variant="secondary"
                onClick={() => void preview()}
                disabled={previewing || !resolvedChart || !name}
              >
                {previewing ? "Rendering…" : "Preview"}
              </Button>
            }
          >
            <div className="relative h-64 overflow-hidden rounded-md border border-border">
              <Suspense fallback={<Spinner label="Loading editor" />}>
                <div className="absolute inset-0">
                  <CodeEditor
                    value={values}
                    onChange={(v) => {
                      setValues(v);
                      invalidatePreview();
                    }}
                    fill
                    ariaLabel="Values YAML"
                  />
                </div>
              </Suspense>
            </div>
          </Field>

          {previewError && (
            <div className="text-destructive text-sm" role="alert">
              {previewError}
            </div>
          )}
          {diff && (
            <div className="max-h-64 overflow-auto rounded-md border border-border">
              <DiffView doc={diff} />
            </div>
          )}
        </div>

        <DialogFooter>
          <Button variant="secondary" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" onClick={run}>
            {confirmLabel}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
