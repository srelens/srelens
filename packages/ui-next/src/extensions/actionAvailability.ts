import type { ActionPredicate } from "@srelens/core";

/** Shared display predicates matching core GitOps guards; #551 moves them into manifests. */
const notSuspended: ActionPredicate = {jsonPath:".spec.suspend",notEquals:true,reason:"Resume this resource before requesting reconciliation"};
export const ACTION_AVAILABILITY: Record<string, ActionPredicate[]> = {
  suspend:[{jsonPath:".spec.suspend",notEquals:true,reason:"This resource is already suspended"}],
  resume:[{jsonPath:".spec.suspend",equals:true,reason:"This resource is not suspended"}],
  reconcile:[notSuspended],
  force:[notSuspended],
  reset:[notSuspended],
  refresh:[],
  "hard-refresh":[],
  sync:[{jsonPath:".operation",absent:true,reason:"An Argo CD operation is already in progress"}],
};
