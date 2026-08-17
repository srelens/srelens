import React, { useEffect, useMemo, useState } from "react";
import { RefreshCw } from "lucide-react";
import { listCustomResource, type CrdRef, type CustomRow } from "../lib/crds";
import { listNamespaces } from "../lib/workloads";
import { YamlView } from "./YamlView";
import { Table, filterTableData, Select, Button, ColumnPicker, useColumnVisibility, Spinner, Drawer, TextInput, type Column } from "../ui";
import { ageSortValue } from "../lib/age";
import type { TabViewState } from "../lib/tabView";

interface Selected {
  name: string;
  namespace: string;
}

/**
 * Lists instances of a custom resource (CRD-backed kind) for a cluster, with a
 * namespace filter, search, and a YAML detail drawer. Uses the dynamic
 * `k8s.listCustomResource` + CRD-aware `k8s.getManifest`.
 */
export function CustomResourceBrowser({
  context,
  crd,
  query = "",
  onQueryChange,
  detailDrawerWidth = 480,
  view,
  onViewChange,
}: {
  context: string;
  crd: CrdRef;
  query?: string;
  onQueryChange?: (q: string) => void;
  detailDrawerWidth?: number;
  /** Sort + filtered column owned by the tab, so they survive a switch (#254). */
  view?: TabViewState;
  onViewChange?: (patch: Partial<TabViewState>) => void;
}) {
  const [namespaces, setNamespaces] = useState<string[]>([]);
  const [namespace, setNamespace] = useState("");
  const [rows, setRows] = useState<CustomRow[] | null>(null);
  const [error, setError] = useState("");
  const [reloadKey, setReloadKey] = useState(0);
  const [selected, setSelected] = useState<Selected | null>(null);
  // Tab-owned when a change handler is supplied; local otherwise.
  const [localFilterColumn, setLocalFilterColumn] = useState<string | null>(null);
  const filterColumn = onViewChange ? (view?.filterColumn ?? null) : localFilterColumn;
  const setFilterColumn = (next: string | null) => {
    if (onViewChange) onViewChange({ filterColumn: next });
    else setLocalFilterColumn(next);
  };

  useEffect(() => {
    if (!crd.namespaced) return;
    void listNamespaces(context).then((o) => setNamespaces(o.namespaces ?? []));
  }, [context, crd.namespaced]);

  useEffect(() => {
    let active = true;
    setRows(null);
    setError("");
    setSelected(null);
    void listCustomResource(context, crd, crd.namespaced ? namespace : null).then((o) => {
      if (!active) return;
      if (o.error) setError(o.error);
      else setRows(o.items ?? []);
    });
    return () => {
      active = false;
    };
  }, [context, crd, namespace, reloadKey]);

  const columns: Column<CustomRow>[] = [
    { key: "name", header: crd.kind, render: (r) => <strong>{r.name}</strong> },
    ...(crd.namespaced
      ? [
          {
            key: "namespace",
            header: "Namespace",
            render: (r: CustomRow) => <span className="fl-link">{r.namespace}</span>,
          },
        ]
      : []),
    { key: "age", header: "Age", getSortValue: ageSortValue, render: (r) => <span className="text-muted-foreground">{r.age}</span> },
  ];

  // Show/hide columns, persisted per CRD.
  const { visibleColumns, columnOptions, hidden, toggle, pinnedKey } = useColumnVisibility(
    `crd:${crd.group}/${crd.kind}`,
    columns,
  );
  useEffect(() => {
    if (filterColumn && !visibleColumns.some((column) => column.key === filterColumn)) {
      setFilterColumn(null);
    }
  }, [visibleColumns, filterColumn]);

  const filtered = useMemo(
    () => filterTableData(rows ?? [], visibleColumns, query, filterColumn),
    [visibleColumns, filterColumn, query, rows],
  );
  const filterLabel = filterColumn
    ? visibleColumns.find((column) => column.key === filterColumn)?.header
    : null;

  return (
    <div className="flex min-h-0 flex-1">
      <div className="flex min-w-0 flex-1 flex-col overflow-hidden">
        <div className="flex shrink-0 flex-wrap items-center gap-3 border-b border-border px-3 py-2">
          {crd.namespaced && (
            <div className="flex items-center gap-2">
              <span className="text-sm text-muted-foreground">Namespace</span>
              <Select
                value={namespace}
                onValueChange={setNamespace}
                options={[{ value: "", label: "All namespaces" }, ...namespaces.map((n) => ({ value: n }))]}
                aria-label="Namespace"
                className="min-w-44"
              />
            </div>
          )}
          <Button variant="ghost" size="sm" onClick={() => setReloadKey((k) => k + 1)} disabled={rows === null}>
            <RefreshCw data-icon="inline-start" />
            Refresh
          </Button>
          {rows === null && <Spinner label="Loading resources" />}
          <div className="ml-auto">
            <ColumnPicker columns={columnOptions} hidden={hidden} onToggle={toggle} pinnedKey={pinnedKey} />
          </div>
          <div className="w-56">
            <TextInput
              value={query}
              onValueChange={(q) => onQueryChange?.(q)}
              type="search"
              placeholder={typeof filterLabel === "string" ? `Search ${filterLabel}…` : "Search all columns…"}
              aria-label="Search resources"
            />
          </div>
          {!error && (
            <span className="text-sm text-muted-foreground tabular-nums">
              {filtered.length} {filtered.length === 1 ? "item" : "items"}
            </span>
          )}
        </div>

        <div className="min-h-0 flex-1 overflow-auto">
          {error && <p className="px-3 py-2 text-sm text-destructive">Error: {error}</p>}
          {!error && (
            <Table
              columns={visibleColumns}
              data={filtered}
              getRowKey={(r) => r.name}
              selectedKey={selected?.name}
              onRowClick={(r) => setSelected({ name: r.name, namespace: r.namespace })}
              activeFilterKey={filterColumn}
              onActiveFilterKeyChange={setFilterColumn}
              sort={view?.sort ?? null}
              onSortChange={onViewChange ? (sort) => onViewChange({ sort }) : undefined}
              emptyText={query ? "No matches" : `No ${crd.kind} resources`}
            />
          )}
        </div>
      </div>

      <Drawer
        open={!!selected}
        defaultWidth={detailDrawerWidth}
        title={selected ? <>{crd.kind}: <code>{selected.name}</code></> : null}
        onClose={() => setSelected(null)}
      >
        {selected && (
          <YamlView
            context={context}
            kind={crd.kind}
            namespace={crd.namespaced ? selected.namespace : null}
            name={selected.name}
            crd={{ group: crd.group, version: crd.version, plural: crd.plural }}
          />
        )}
      </Drawer>
    </div>
  );
}
