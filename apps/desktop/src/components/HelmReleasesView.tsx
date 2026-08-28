import { Suspense, lazy, useEffect, useState } from "react";
import { ArrowUp, Plus, RefreshCw, RotateCcw, Trash2 } from "lucide-react";
import {
  listHelmReleases,
  getHelmRelease,
  helmVersion,
  helmRepoUpdate,
  helmRepoAdd,
  type HelmReleaseSummary,
  type HelmReleaseDetail,
} from "@srelens/core";
import { ageFromTimestamp, absoluteTimestamp, Section, KV } from "./ResourceOverview";
import type { TabViewState } from "@srelens/core";
import {
  Table,
  filterTableData,
  Spinner,
  Badge,
  ColumnPicker,
  useColumnVisibility,
  StatusPill,
  Drawer,
  Tabs,
  Toolbar,
  Button,
  IconButton,
  TextInput,
  ConfirmDialog,
  Field,
  Select,
  type Column,
  type StatusKind,
} from "../ui";
import { NamespaceMultiSelect } from "../ui/NamespaceMultiSelect";
import {
  parseNamespaceSelection,
  serializeNamespaceSelection,
  watchNamespaceForSelection,
  rowInSelection,
} from "@srelens/core";
import { useNamespaceOptions } from "@srelens/core/react";
import { describeError } from "@srelens/core";
import { HelmOpDialog, type HelmOpRelease } from "./HelmOpDialog";

const CodeEditor = lazy(() => import("../ui/CodeEditor").then((m) => ({ default: m.CodeEditor })));

/** Session descriptor for a streamed helm operation, opened in the bottom dock. */
export type OpenHelmDock = (session: {
  context: string;
  namespace: string;
  helm: { args: string[]; title: string; values?: string; onComplete?: () => void };
}) => void;

/** Map a Helm status to a status-pill colour. */
function statusKind(status: string): StatusKind {
  if (status === "deployed" || status === "superseded") return "success";
  if (status === "failed" || status === "unknown") return "danger";
  if (status.startsWith("pending") || status === "uninstalling") return "warning";
  return "neutral";
}

/** Overview of Helm releases across the cluster, with a values/manifest/history drawer. */
export function HelmReleasesView({
  context,
  detailDrawerWidth = 480,
  openHelmDock,
  initialNamespace,
  onNamespaceChange,
  kubeconfigFiles = [],
  view,
  onViewChange,
}: {
  context: string;
  detailDrawerWidth?: number;
  openHelmDock?: OpenHelmDock;
  /** Namespace to start on (empty = all); persisted per tab/cluster by the parent. */
  initialNamespace?: string;
  /** Notified when the namespace filter changes, so the parent can preserve it. */
  onNamespaceChange?: (namespace: string) => void;
  /** Sort + search owned by the tab, so they survive a switch (#254). */
  view?: TabViewState;
  onViewChange?: (patch: Partial<TabViewState>) => void;
  /**
   * All configured kubeconfig files (default path + pasted/additional). Used to
   * register their paths in the backend client cache before we build a client
   * for this context — mirrors ResourceBrowser.
   */
  kubeconfigFiles?: string[];
}) {
  const [releases, setReleases] = useState<HelmReleaseSummary[]>([]);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(true);
  const [selected, setSelected] = useState<HelmReleaseSummary | null>(null);
  const [helmMissing, setHelmMissing] = useState(false);
  const [installing, setInstalling] = useState(false);
  const [addingRepo, setAddingRepo] = useState(false);
  const [repoName, setRepoName] = useState("");
  const [repoUrl, setRepoUrl] = useState("");
  const [repoError, setRepoError] = useState("");
  const [repoBusy, setRepoBusy] = useState(false);
  const [reloadKey, setReloadKey] = useState(0);
  // Namespace selection is a set (empty = all namespaces), serialized to/from
  // the persisted comma string. One selected namespace scopes the release
  // fetch server-side (efficient); none or many fetch all, filtered client-side.
  const [selection, setSelection] = useState(() => parseNamespaceSelection(initialNamespace ?? ""));
  const changeNamespaces = (next: string[]) => {
    setSelection(next);
    onNamespaceChange?.(serializeNamespaceSelection(next));
  };
  const { namespaces, scope: nsScope, error: nsError } = useNamespaceOptions(context, kubeconfigFiles);
  // Restricted credentials scope the view to a single namespace — force the
  // selection to match rather than leaving it at whatever was selected before.
  useEffect(() => {
    if (nsScope) setSelection([nsScope]);
  }, [nsScope]);
  const scopeNs = watchNamespaceForSelection(selection);
  // Tab-owned when a change handler is supplied (#254), so sorting or
  // searching releases survives a tab switch like every other list.
  const [localQuery, setLocalQuery] = useState("");
  const query = onViewChange ? (view?.query ?? "") : localQuery;
  const setQuery = (next: string) => {
    if (onViewChange) onViewChange({ query: next });
    else setLocalQuery(next);
  };
  const [detail, setDetail] = useState<HelmReleaseDetail | null>(null);
  const [detailError, setDetailError] = useState("");

  useEffect(() => {
    let active = true;
    setLoading(true);
    void listHelmReleases(context, scopeNs || null).then((o) => {
      if (!active) return;
      const next = o.releases ?? [];
      setReleases(next);
      setError(o.error ?? "");
      setLoading(false);
      setSelected((sel) =>
        sel && !next.some((r) => r.namespace === sel.namespace && r.name === sel.name) ? null : sel,
      );
    });
    return () => {
      active = false;
    };
  }, [context, reloadKey, scopeNs]);

  useEffect(() => {
    let active = true;
    void helmVersion(context).then((r) => {
      if (active) setHelmMissing(Boolean(r.error));
    });
    return () => {
      active = false;
    };
  }, [context]);

  // Fetch the selected release's detail here (rather than in the drawer panel)
  // so the header actions — which need `detail.history` — can be rendered by
  // the Drawer's `headerActions` slot alongside the rest of the app's detail
  // drawers. `reloadKey` stays in the deps so the detail refreshes after a
  // streamed helm op completes.
  useEffect(() => {
    if (!selected) {
      setDetail(null);
      setDetailError("");
      return;
    }
    let active = true;
    setDetail(null);
    setDetailError("");
    void getHelmRelease(context, selected.namespace, selected.name).then((o) => {
      if (!active) return;
      if (o.error) setDetailError(o.error);
      else setDetail(o.release ?? null);
    });
    return () => {
      active = false;
    };
  }, [context, selected?.namespace, selected?.name, reloadKey]);

  const now = Date.now();
  const columns: Column<HelmReleaseSummary>[] = [
    { key: "name", header: "Name", render: (r) => <span className="fl-mono">{r.name}</span> },
    { key: "namespace", header: "Namespace", render: (r) => r.namespace },
    { key: "chart", header: "Chart", render: (r) => `${r.chart}-${r.chartVersion}` },
    { key: "app", header: "App", render: (r) => r.appVersion || "—" },
    { key: "rev", header: "Rev", render: (r) => <span className="tabular-nums">{r.revision}</span> },
    { key: "status", header: "Status", render: (r) => <StatusPill status={r.status} kind={statusKind(r.status)} /> },
    { key: "updated", header: "Updated", render: (r) => ageFromTimestamp(r.updated, now) },
  ];

  const { visibleColumns, columnOptions, hidden, toggle, pinnedKey } = useColumnVisibility("helmreleases", columns);

  // The backend has already scoped `releases` to the selected namespace when a
  // single namespace is selected; the client-side filter only matters for the
  // multi-select case (several namespaces selected, all fetched, filtered here).
  const namespaceFiltered = releases.filter((r) => rowInSelection(r.namespace, selection));
  const filtered = filterTableData(namespaceFiltered, visibleColumns, query, null);

  return (
    <div className="flex min-h-0 flex-1">
      <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
        {helmMissing && (
          <div className="fl-notice fl-notice--warn" role="status">
            Helm CLI not found on PATH — install Helm to manage releases.
          </div>
        )}
        {repoError && (
          <div className="text-destructive text-sm" role="alert">
            {repoError}
          </div>
        )}
        {nsScope && (
          <p className="px-3 py-1 text-xs text-muted-foreground" role="status">
            Scoped to namespace {nsScope} — you don’t have permission to list all namespaces.
          </p>
        )}
        {nsError && (
          <p className="px-3 py-1 text-xs text-muted-foreground" role="status">
            {describeError(nsError).title} — can’t list namespaces; showing all.
          </p>
        )}
        <Toolbar className="fl-resource-toolbar shrink-0 flex-wrap">
          {namespaces && namespaces.length >= 1 && (
            <div className="fl-resource-toolbar__namespace flex items-center gap-2">
              <span>Namespace</span>
              <NamespaceMultiSelect
                namespaces={namespaces}
                selection={selection}
                onChange={changeNamespaces}
                ariaLabel="Namespace"
                className="min-w-44"
              />
            </div>
          )}
          <Button variant="ghost" size="sm" onClick={() => setReloadKey((k) => k + 1)} disabled={loading}>
            <RefreshCw data-icon="inline-start" />
            Refresh
          </Button>
          <Button variant="ghost" size="sm" disabled={helmMissing} onClick={() => setInstalling(true)}>
            <Plus data-icon="inline-start" />
            Install
          </Button>
          <Button variant="ghost" size="sm" disabled={helmMissing} onClick={() => setAddingRepo(true)}>
            Add repo
          </Button>
          <Button
            variant="ghost"
            size="sm"
            disabled={helmMissing || repoBusy}
            onClick={async () => {
              setRepoBusy(true);
              const r = await helmRepoUpdate(context);
              setRepoBusy(false);
              setRepoError(r.error ?? "");
            }}
          >
            Update repos
          </Button>
          {loading && <Spinner label="Loading releases" />}
          <div className="ml-auto">
            <ColumnPicker columns={columnOptions} hidden={hidden} onToggle={toggle} pinnedKey={pinnedKey} />
          </div>
          <div className="fl-resource-toolbar__search w-56">
            <TextInput
              value={query}
              onValueChange={setQuery}
              type="search"
              aria-label="Search resources"
              placeholder="Search all columns…"
            />
          </div>
          {!error && (
            <span className="fl-resource-toolbar__count tabular-nums">
              {filtered.length} {filtered.length === 1 ? "item" : "items"}
            </span>
          )}
        </Toolbar>
        <div className="min-h-0 flex-1 overflow-auto">
          {error ? (
            <div className="p-3 text-destructive">Error: {error}</div>
          ) : releases.length === 0 && !loading ? (
            <div className="p-8 text-center text-sm text-muted-foreground">
              No Helm releases found in this cluster.
            </div>
          ) : (
            <Table
              columns={visibleColumns}
              data={filtered}
              getRowKey={(r) => `${r.namespace}/${r.name}`}
              onRowClick={setSelected}
              selectedKey={selected ? `${selected.namespace}/${selected.name}` : undefined}
              sort={view?.sort ?? null}
              onSortChange={onViewChange ? (sort) => onViewChange({ sort }) : undefined}
            />
          )}
        </div>
      </div>

      <Drawer
        open={!!selected}
        defaultWidth={detailDrawerWidth}
        title={selected ? <>Release: <code>{selected.name}</code></> : null}
        headerActions={
          detail ? (
            <HelmReleaseActions
              context={context}
              detail={detail}
              openHelmDock={openHelmDock}
              helmMissing={helmMissing}
              onReload={() => setReloadKey((k) => k + 1)}
            />
          ) : undefined
        }
        onClose={() => setSelected(null)}
      >
        {selected && <HelmReleaseDetailPanel detail={detail} error={detailError} />}
      </Drawer>

      {installing && (
        <HelmOpDialog
          context={context}
          mode="install"
          onRun={(r) => {
            setInstalling(false);
            openHelmDock?.({
              context,
              namespace: r.namespace,
              helm: {
                args: r.helmArgs,
                title: `Install ${r.name}`,
                values: r.values,
                onComplete: () => setReloadKey((k) => k + 1),
              },
            });
          }}
          onClose={() => setInstalling(false)}
        />
      )}

      {addingRepo && (
        <ConfirmDialog
          title="Add chart repository"
          confirmLabel="Add"
          busy={repoBusy}
          onConfirm={async () => {
            setRepoBusy(true);
            const r = await helmRepoAdd(context, { name: repoName, url: repoUrl });
            setRepoBusy(false);
            if (r.error) {
              setRepoError(r.error);
              return;
            }
            setRepoError("");
            setAddingRepo(false);
            setRepoName("");
            setRepoUrl("");
          }}
          onCancel={() => {
            setAddingRepo(false);
            setRepoError("");
          }}
          message={
            <div className="flex flex-col gap-3">
              <Field label="Repository name">
                <TextInput aria-label="Repo name" value={repoName} onValueChange={setRepoName} placeholder="bitnami" />
              </Field>
              <Field label="Repository URL">
                <TextInput
                  aria-label="Repo URL"
                  value={repoUrl}
                  onValueChange={setRepoUrl}
                  placeholder="https://charts.bitnami.com/bitnami"
                />
              </Field>
              {repoError && (
                <div className="text-destructive text-sm" role="alert">
                  {repoError}
                </div>
              )}
            </div>
          }
        />
      )}
    </div>
  );
}

/** Drawer header actions for a Helm release: Upgrade, Rollback, Uninstall. */
function HelmReleaseActions({
  context,
  detail,
  openHelmDock,
  helmMissing,
  onReload,
}: {
  context: string;
  detail: HelmReleaseDetail;
  openHelmDock?: OpenHelmDock;
  helmMissing?: boolean;
  onReload: () => void;
}) {
  const [upgrading, setUpgrading] = useState(false);
  const [rollingBack, setRollingBack] = useState(false);
  const [rollbackRevision, setRollbackRevision] = useState<number | null>(null);
  const [uninstalling, setUninstalling] = useState(false);

  const upgradeRelease: HelmOpRelease = {
    name: detail.name,
    namespace: detail.namespace,
    chart: detail.chart,
    chartVersion: detail.chartVersion,
    valuesYaml: detail.valuesYaml,
    manifest: detail.manifest,
  };

  const canRollback = detail.history.length >= 2;
  const helmMissingTitle = "Helm CLI not found on PATH";

  return (
    <>
      <IconButton
        icon={ArrowUp}
        label="Upgrade"
        disabled={helmMissing}
        title={helmMissing ? helmMissingTitle : undefined}
        onClick={() => setUpgrading(true)}
      />
      <IconButton
        icon={RotateCcw}
        label="Rollback"
        disabled={helmMissing || !canRollback}
        title={helmMissing ? helmMissingTitle : !canRollback ? "No earlier revision to roll back to" : undefined}
        onClick={() => {
          setRollbackRevision(detail.history[1]?.revision ?? null);
          setRollingBack(true);
        }}
      />
      <IconButton
        icon={Trash2}
        label="Uninstall"
        danger
        disabled={helmMissing}
        title={helmMissing ? helmMissingTitle : undefined}
        onClick={() => setUninstalling(true)}
      />

      {upgrading && (
        <HelmOpDialog
          context={context}
          mode="upgrade"
          release={upgradeRelease}
          onRun={(r) => {
            setUpgrading(false);
            openHelmDock?.({
              context,
              namespace: detail.namespace,
              helm: {
                args: r.helmArgs,
                title: `Upgrade ${detail.name}`,
                values: r.values,
                onComplete: onReload,
              },
            });
          }}
          onClose={() => setUpgrading(false)}
        />
      )}

      {rollingBack && (
        <ConfirmDialog
          title="Rollback release"
          confirmLabel="Rollback"
          onConfirm={() => {
            const rev = rollbackRevision;
            setRollingBack(false);
            if (rev == null) return;
            openHelmDock?.({
              context,
              namespace: detail.namespace,
              helm: {
                args: ["rollback", detail.name, String(rev), "--namespace", detail.namespace],
                title: `Rollback ${detail.name} → ${rev}`,
                onComplete: onReload,
              },
            });
          }}
          onCancel={() => setRollingBack(false)}
          message={
            <Field label="Revision">
              <Select
                aria-label="Revision"
                value={String(rollbackRevision ?? "")}
                onValueChange={(v) => setRollbackRevision(Number(v))}
                options={detail.history.slice(1).map((h) => ({
                  value: String(h.revision),
                  label: `${h.revision} — ${h.description || h.status}`,
                }))}
              />
            </Field>
          }
        />
      )}

      {uninstalling && (
        <ConfirmDialog
          title="Uninstall release"
          confirmLabel="Uninstall"
          danger
          onConfirm={() => {
            setUninstalling(false);
            openHelmDock?.({
              context,
              namespace: detail.namespace,
              helm: {
                args: ["uninstall", detail.name, "--namespace", detail.namespace],
                title: `Uninstall ${detail.name}`,
                onComplete: onReload,
              },
            });
          }}
          onCancel={() => setUninstalling(false)}
          message={`Uninstall "${detail.name}" from namespace ${detail.namespace}? This removes all its resources.`}
        />
      )}
    </>
  );
}

/** Detail drawer body: release info + values/manifest/history/notes tabs. Actions live in the drawer header. */
function HelmReleaseDetailPanel({ detail, error }: { detail: HelmReleaseDetail | null; error: string }) {
  const [tab, setTab] = useState("values");

  if (error) return <div className="text-destructive">Error: {error}</div>;
  if (!detail) return <Spinner label="Loading release" />;

  // Same "<age> ago (<absolute>)" format the resource detail views use for Created.
  const updated = detail.updated
    ? `${ageFromTimestamp(detail.updated)} ago (${absoluteTimestamp(detail.updated)})`
    : "";

  return (
    <div className="flex h-full min-h-0 flex-col gap-3">
      {/* Same section/KV language as the resource detail overviews (Deployment
          etc.): an accent-barred card with label/value rows. */}
      <div className="fl-detail shrink-0">
        <Section title="Properties">
          <KV
            pairs={[
              ["Status", <StatusPill key="s" status={detail.status} kind={statusKind(detail.status)} />],
              ["Name", <span className="fl-mono">{detail.name}</span>],
              ["Namespace", <span className="fl-mono">{detail.namespace}</span>],
              ["Chart", <span className="fl-mono">{`${detail.chart}-${detail.chartVersion}`}</span>],
              ["App version", detail.appVersion],
              ["Revision", String(detail.revision)],
              ["Updated", updated],
            ]}
          />
        </Section>
      </div>

      <Tabs
        tabs={[
          { id: "values", label: "Values" },
          { id: "manifest", label: "Manifest" },
          { id: "history", label: `History (${detail.history.length})` },
          ...(detail.notes ? [{ id: "notes", label: "Notes" }] : []),
        ]}
        active={tab}
        onChange={setTab}
      />

      {/* The tab body fills the rest of the drawer: the editor scrolls inside a
          definite-height box (absolute-inset) rather than sitting in a short
          fixed box with dead space beneath it. */}
      <div className="relative min-h-0 flex-1 overflow-hidden">
        {tab === "values" && (
          <Suspense fallback={<Spinner label="Loading editor" />}>
            <div className="absolute inset-0">
              <CodeEditor
                value={detail.valuesYaml || "# no user-supplied values\n"}
                readOnly
                fill
                ariaLabel="Release values"
              />
            </div>
          </Suspense>
        )}
        {tab === "manifest" && (
          <Suspense fallback={<Spinner label="Loading editor" />}>
            <div className="absolute inset-0">
              <CodeEditor
                value={detail.manifest || "# empty manifest\n"}
                readOnly
                fill
                ariaLabel="Release manifest"
              />
            </div>
          </Suspense>
        )}
        {tab === "history" && (
          <div className="absolute inset-0 overflow-auto">
            <Table
              columns={[
                {
                  key: "revision",
                  header: "Rev",
                  render: (h) => (
                    <span className="tabular-nums">
                      {h.revision}
                      {h.revision === detail.revision && (
                        <span className="ml-2">
                          <Badge variant="info">current</Badge>
                        </span>
                      )}
                    </span>
                  ),
                },
                {
                  key: "status",
                  header: "Status",
                  render: (h) => <StatusPill status={h.status} kind={statusKind(h.status)} />,
                },
                { key: "chart", header: "Chart ver", render: (h) => h.chartVersion },
                {
                  key: "updated",
                  header: "Updated",
                  render: (h) => <span title={h.updated || undefined}>{ageFromTimestamp(h.updated)}</span>,
                },
                { key: "description", header: "Description", render: (h) => h.description },
              ]}
              data={detail.history}
              getRowKey={(h) => String(h.revision)}
            />
          </div>
        )}
        {tab === "notes" && (
          <pre className="absolute inset-0 overflow-auto whitespace-pre-wrap rounded border border-border bg-muted/40 p-2 text-xs">
            {detail.notes}
          </pre>
        )}
      </div>
    </div>
  );
}
