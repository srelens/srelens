import {
  ageSortValue,
  listCustomResource,
  printerColumnKeys,
  printerSortValue,
  type CrdRef,
  type CustomRow,
} from "@srelens/core";
import type { Column } from "@srelens/ui-kit";
import type { KindActions, KindDescriptor } from "./types";
import { AgeCell } from "../ageCell";

/**
 * Matches `MAX_COLUMNS` in `crates/kube/src/crds.rs` / `MAX_PRINTER_COLUMNS`
 * in plugin-host. `listCustomResource` evaluates at most this many columns per
 * row; the table must not paint headers past that index (#609).
 */
const MAX_PRINTER_COLUMNS = 32;

/**
 * A custom resource's table, built from the printer columns the API server
 * declares for it. Keys come from `printerColumnKeys` rather than the
 * column's index: hidden columns and the tab's sort key persist under them,
 * and a positional key would move a user's choices the day an operator
 * upgrade inserts or reorders `additionalPrinterColumns`.
 */
export function customColumns(crd: CrdRef): Column<CustomRow>[] {
  const printers = (crd.printerColumns ?? []).slice(0, MAX_PRINTER_COLUMNS);
  const keys = printerColumnKeys(printers);
  const columns: Column<CustomRow>[] = [
    // The mock titles every list's identifier column "Name", never the kind
    // — the same rule every typed set and the generic three follow.
    { key: "name", header: "Name", sortable: true },
  ];
  if (crd.namespaced) {
    columns.push({ key: "namespace", header: "Namespace", sortable: true });
  }
  printers.forEach((printer, index) => {
    columns.push({
      key: keys[index],
      header: printer.name,
      sortable: true,
      render: (row) => row.columns?.[index] ?? "—",
      getSortValue: (row) => printerSortValue(printer.type, row.columns?.[index] ?? "", row.sortKeys?.[index]),
    });
  });
  // #405: live age for a custom resource too.
  columns.push({ key: "age", header: "Age", sortable: true, align: "end", render: (r) => <AgeCell created={r.created} age={r.age} />, getSortValue: ageSortValue });
  return columns;
}

/**
 * What a custom resource offers: what every kind offers by default — Edit,
 * Copy as kubectl and Delete — and none of the kind-specific actions.
 *
 * Named because the resource detail pane reaches it too: a kind outside
 * `K8S_KIND` has no descriptor to read (`descriptorFor` answers `undefined`
 * for exactly the custom-resource case).
 */
export const CUSTOM_RESOURCE_ACTIONS: KindActions = {};

/** The descriptor for one discovered CRD. */
export function customDescriptor(crd: CrdRef): KindDescriptor<CustomRow> {
  return {
    k8sKind: crd.kind,
    group: crd.group,
    columns: customColumns(crd),
    source: "poll",
    scope: crd.namespaced ? "namespaced" : "cluster",
    load: (context, namespace) =>
      listCustomResource(context, crd, namespace || null).then((o) => ({
        rows: o.items,
        truncated: o.truncated,
        error: o.error,
      })),
    actions: CUSTOM_RESOURCE_ACTIONS,
  };
}
