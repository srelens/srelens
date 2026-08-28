import type { CrdRef } from "./crds";
import type { ResourceKind } from "./kinds";
import type { TabViewState } from "./tabView";

/**
 * One open tab in the workspace. It lives here rather than beside the
 * component that renders it because `openTabs` persists it, and a persistence
 * module should not depend on the app root to describe its own payload.
 */
export interface ViewTab {
  id: number;
  cluster: string | null;
  kind: ResourceKind;
  /** Present when the tab is a custom-resource (CRD) view. */
  crd?: CrdRef;
  /** Deep-link target from global search (opens the resource's detail). */
  focus?: { name: string; namespace: string | null; nonce: number };
  /** For a "new resource" tab: the template kind to start from. */
  create?: { initialKind?: string };
  /** For an "edit resource" tab: the resource to preload and apply back. */
  edit?: { kind: string; namespace: string | null; name: string };
  /** Identity of `cluster` (#265). The display name changes when another
   *  kubeconfig declares the same context name; this does not, so a rename
   *  never reads as a deleted context and never closes the tab. */
  clusterId?: string;
  /** Selected namespace filter (empty = all), preserved per tab. */
  namespace?: string;
  /** Sort, search text and filtered column for this tab's list (#254). */
  view?: TabViewState;
}
