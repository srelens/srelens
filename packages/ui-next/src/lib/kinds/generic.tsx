import { ageSortValue, type ResourceRow } from "@srelens/core";
import { AgeCell } from "../ageCell";
import type { Column } from "@srelens/ui-kit";

/**
 * What a kind with no typed view shows. `listResource` returns exactly these
 * three fields for any kind the cluster has, so this is not a placeholder —
 * it is the whole of what the backend knows about a LeaseList.
 */
export const genericColumns: Column<ResourceRow>[] = [
  { key: "name", header: "Name", sortable: true },
  { key: "namespace", header: "Namespace", sortable: true },
  // #405: derived against a ticking clock from `created`, so every kind with
  // no typed view — which is most of them — gets a live age too.
  { key: "age", header: "Age", sortable: true, align: "end", render: (r) => <AgeCell created={r.created} age={r.age} />, getSortValue: ageSortValue },
];

/** The same, for a kind that has no namespace to show. */
export const genericClusterColumns: Column<ResourceRow>[] = genericColumns.filter(
  (c) => c.key !== "namespace",
);
