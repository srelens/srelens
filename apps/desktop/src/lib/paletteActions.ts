import { K8S_KIND, type ResourceKind } from "../components/ResourceBrowser";
import {
  deleteResource,
  rolloutRestart,
  cordonNode,
  drainNode,
  cronjobSetSuspend,
  cronjobTriggerNow,
  createNodeDebugPod,
} from "./actions";
import { deletePod, evictPod } from "./workloads";

/** Target resource a palette action runs against. */
export interface PaletteActionCtx {
  context: string;
  kind: ResourceKind;
  namespace: string | null;
  name: string;
}

/** One entry in the palette's action registry. */
export interface PaletteAction {
  /** Backend capability id this action invokes (see capability-catalog.json). */
  capabilityId: string;
  label: string;
  /** Resource kinds this action applies to, or "*" for every kind. */
  kinds: ResourceKind[] | "*";
  destructive?: boolean;
  /**
   * Marks this action as needing extra input the palette can't collect
   * inline (a replica count, a debug image). Rather than prompting via
   * `window.prompt` or a hardcoded value, the palette (wired up in Task 4)
   * opens the matching existing dialog — the scale dialog for "scale", the
   * debug-container dialog for "debug" — and lets that dialog's own submit
   * flow invoke the backend. Actions marked this way have no `run`.
   */
  opensDialog?: "scale" | "debug";
  run?: (c: PaletteActionCtx) => Promise<{ error?: string }> | { error?: string } | void;
}

/** Convert a palette `ResourceKind` to the API kind string `lib/actions.ts` expects. */
function kindToK8s(kind: ResourceKind): string {
  return K8S_KIND[kind];
}

// Same grouping as SCALABLE/RESTARTABLE in DetailActions.tsx, expressed as
// ResourceKind rather than API-kind strings so they can key the registry.
const SCALABLE_KINDS: ResourceKind[] = ["deployments", "statefulsets", "replicasets"];
const RESTARTABLE_KINDS: ResourceKind[] = ["deployments", "statefulsets", "daemonsets"];

/**
 * Registry of resource-targeted mutating actions, one entry per
 * (capability, verb) pair. Covers every non-read-only, single-resource
 * capability in the catalog except: manifest editing (`k8s.applyManifest`),
 * ConfigMap/Secret data edits (`k8s.updateConfigData`, drawer-only — needs a
 * key/value patch beyond a bare resource ref), context management
 * (`k8s.deleteContext`), and Helm/toolbox operations (their own release- or
 * plugin-scoped flows, not a single ResourceKind target). Task 3's audit
 * tracks those exclusions explicitly.
 */
export const PALETTE_ACTIONS: PaletteAction[] = [
  {
    capabilityId: "k8s.scale",
    label: "Scale…",
    kinds: SCALABLE_KINDS,
    opensDialog: "scale",
  },
  {
    capabilityId: "k8s.rolloutRestart",
    label: "Rollout restart",
    kinds: RESTARTABLE_KINDS,
    run: (c) => rolloutRestart(c.context, kindToK8s(c.kind), c.namespace ?? "", c.name),
  },
  {
    capabilityId: "k8s.deleteResource",
    label: "Delete",
    kinds: "*",
    destructive: true,
    run: (c) => deleteResource(c.context, kindToK8s(c.kind), c.namespace, c.name),
  },
  {
    capabilityId: "k8s.deletePod",
    label: "Force delete pod",
    kinds: ["pods"],
    destructive: true,
    run: (c) => deletePod(c.context, c.namespace ?? "", c.name),
  },
  {
    capabilityId: "k8s.evictPod",
    label: "Evict pod",
    kinds: ["pods"],
    destructive: true,
    run: (c) => evictPod(c.context, c.namespace ?? "", c.name),
  },
  {
    capabilityId: "k8s.debugPod",
    label: "Debug (ephemeral container)…",
    kinds: ["pods"],
    destructive: true,
    opensDialog: "debug",
  },
  {
    capabilityId: "k8s.cordonNode",
    label: "Cordon node",
    kinds: ["nodes"],
    run: (c) => cordonNode(c.context, c.name, true),
  },
  {
    capabilityId: "k8s.cordonNode",
    label: "Uncordon node",
    kinds: ["nodes"],
    run: (c) => cordonNode(c.context, c.name, false),
  },
  {
    capabilityId: "k8s.drainNode",
    label: "Drain node",
    kinds: ["nodes"],
    destructive: true,
    run: async (c) => {
      const out = await drainNode(c.context, c.name);
      return out.error ? { error: out.error } : {};
    },
  },
  {
    capabilityId: "k8s.createNodeDebugPod",
    label: "Debug node",
    kinds: ["nodes"],
    destructive: true,
    run: async (c) => {
      const out = await createNodeDebugPod(c.context, c.name);
      return out.error ? { error: out.error } : {};
    },
  },
  {
    capabilityId: "k8s.cronjobSetSuspend",
    label: "Suspend CronJob",
    kinds: ["cronjobs"],
    run: (c) => cronjobSetSuspend(c.context, c.namespace ?? "", c.name, true),
  },
  {
    capabilityId: "k8s.cronjobSetSuspend",
    label: "Resume CronJob",
    kinds: ["cronjobs"],
    run: (c) => cronjobSetSuspend(c.context, c.namespace ?? "", c.name, false),
  },
  {
    capabilityId: "k8s.cronjobTriggerNow",
    label: "Run now",
    kinds: ["cronjobs"],
    run: async (c) => {
      const out = await cronjobTriggerNow(c.context, c.namespace ?? "", c.name);
      return out.error ? { error: out.error } : {};
    },
  },
];

/** Actions applicable to a given resource kind, in registry order. */
export function actionsForKind(kind: ResourceKind): PaletteAction[] {
  // UI-only pseudo-kinds (settings, toolbox, overview, …) have no API kind
  // string and aren't a real resource a "*" action could target.
  if (kindToK8s(kind) === "") return [];
  return PALETTE_ACTIONS.filter((a) => a.kinds === "*" || a.kinds.includes(kind));
}

/** Distinct backend capability ids this registry covers. */
export function paletteActionCapabilityIds(): Set<string> {
  return new Set(PALETTE_ACTIONS.map((a) => a.capabilityId));
}
