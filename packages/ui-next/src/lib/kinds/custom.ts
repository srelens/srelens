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

/**
 * A custom resource's table, built from the printer columns the API server
 * declares for it. Keys come from `printerColumnKeys` rather than the
 * column's index: hidden columns and the tab's sort key persist under them,
 * and a positional key would move a user's choices the day an operator
 * upgrade inserts or reorders `additionalPrinterColumns`.
 */
export function customColumns(crd: CrdRef): Column<CustomRow>[] {
  const printers = crd.printerColumns ?? [];
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
  columns.push({ key: "age", header: "Age", sortable: true, align: "end", getSortValue: ageSortValue });
  return columns;
}

/**
 * What a custom resource offers, which is everything a kind offers by default
 * minus Delete.
 *
 * The backend resolves kind→GVR through a closed match with no CRD path, so
 * Delete on a custom resource always fails — see `KindActions.delete`. Named
 * rather than written inline because a second surface has to reach the same
 * verdict: the resource detail pane's footer builds its actions from the
 * descriptor too, and a kind outside `K8S_KIND` has no descriptor to read
 * (`descriptorFor` answers `undefined` for exactly the custom-resource case).
 * Re-deriving "and custom resources cannot be deleted" there would be a second
 * place to remember the day the backend grows a CRD path.
 */
export const CUSTOM_RESOURCE_ACTIONS: KindActions = { delete: false };

/** The descriptor for one discovered CRD. */
export function customDescriptor(crd: CrdRef): KindDescriptor<CustomRow> {
  return {
    k8sKind: crd.kind,
    columns: customColumns(crd),
    source: "poll",
    scope: crd.namespaced ? "namespaced" : "cluster",
    load: (context, namespace) =>
      listCustomResource(context, crd, namespace || null).then((o) => ({ rows: o.items, error: o.error })),
    actions: CUSTOM_RESOURCE_ACTIONS,
  };
}
