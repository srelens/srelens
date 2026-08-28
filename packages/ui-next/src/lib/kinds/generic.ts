import { ageSortValue, type ResourceRow } from "@srelens/core";
import type { Column } from "@srelens/ui-kit";

/**
 * What a kind with no typed view shows. `listResource` returns exactly these
 * three fields for any kind the cluster has, so this is not a placeholder —
 * it is the whole of what the backend knows about a LeaseList.
 */
export const genericColumns: Column<ResourceRow>[] = [
  { key: "name", header: "Name", sortable: true },
  { key: "namespace", header: "Namespace", sortable: true },
  { key: "age", header: "Age", sortable: true, align: "end", getSortValue: ageSortValue },
];

/** The same, for a kind that has no namespace to show. */
export const genericClusterColumns: Column<ResourceRow>[] = genericColumns.filter(
  (c) => c.key !== "namespace",
);
