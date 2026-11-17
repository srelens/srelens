import {
  Activity,
  ArchiveRestore,
  BadgeCheck,
  Blocks,
  BrickWall,
  Cable,
  Camera,
  ChartLine,
  CirclePlay,
  ClipboardCheck,
  Cloud,
  Cylinder,
  Database,
  DoorOpen,
  FileKey,
  Gavel,
  GitMerge,
  Globe,
  Hammer,
  Link,
  LockKeyhole,
  MessagesSquare,
  Monitor,
  Rocket,
  Scale,
  Scaling,
  ScanSearch,
  Share2,
  TrafficCone,
  Workflow,
  Zap,
} from "lucide-react";
import type { IconComponent } from "@srelens/ui-kit";
import { Icons } from "./icons";

/**
 * A custom resource's glyph, by API group: the operators seen on real
 * clusters, each drawn by what it does. Generic glyphs only, not the projects'
 * logos, so nothing here is anyone's trademark.
 *
 * A key covers its subgroups: `fluxcd.io` is every `*.toolkit.fluxcd.io`, and
 * a longer key wins over a shorter one (`karpenter.k8s.aws` over `k8s.aws`).
 * A Map, not an object literal: a group is cluster data, and a plain object
 * answers `constructor`.
 */
const BY_GROUP = new Map<string, IconComponent>([
  ["cert-manager.io", BadgeCheck],
  // Argo CD, Rollouts, Workflows and Events all share the one group.
  ["argoproj.io", Rocket],
  ["fluxcd.io", GitMerge],
  ["istio.io", Share2],
  ["linkerd.io", Link],
  ["cilium.io", Cable],
  ["projectcalico.org", BrickWall],
  ["tigera.io", BrickWall],
  ["monitoring.coreos.com", Activity],
  ["gateway.networking.k8s.io", DoorOpen],
  ["gateway.networking.x-k8s.io", DoorOpen],
  ["knative.dev", Zap],
  ["keda.sh", Scaling],
  ["velero.io", ArchiveRestore],
  ["crossplane.io", Blocks],
  ["upbound.io", Blocks],
  ["external-secrets.io", FileKey],
  // Sealed Secrets.
  ["bitnami.com", LockKeyhole],
  ["kyverno.io", ClipboardCheck],
  // Policy reports, which Kyverno writes.
  ["wgpolicyk8s.io", ClipboardCheck],
  ["gatekeeper.sh", Gavel],
  ["traefik.io", TrafficCone],
  ["traefik.containo.us", TrafficCone],
  ["nginx.org", Globe],
  ["karpenter.sh", Hammer],
  ["karpenter.k8s.aws", Hammer],
  ["karpenter.azure.com", Hammer],
  ["k8s.aws", Cloud],
  ["amazonaws.com", Cloud],
  ["cloud.google.com", Cloud],
  ["gke.io", Cloud],
  ["azure.com", Cloud],
  ["strimzi.io", MessagesSquare],
  ["k8s.elastic.co", ScanSearch],
  ["tekton.dev", Workflow],
  ["kubevirt.io", Monitor],
  ["longhorn.io", Cylinder],
  ["rook.io", Database],
  ["metallb.io", Scale],
  ["snapshot.storage.k8s.io", Camera],
  ["metrics.k8s.io", ChartLine],
  // Actions Runner Controller, and the community one before it.
  ["actions.github.com", CirclePlay],
  ["actions.summerwind.dev", CirclePlay],
]);

/**
 * The glyph for a custom resource in API group `group`.
 *
 * Tries the exact group, then drops one leading label at a time, down to the
 * last two labels (`acme.cert-manager.io`, then `cert-manager.io`), and gives
 * the generic CRD glyph when none of those is listed. It stops at two labels
 * so that a bare `io` or `com` can never be a key.
 */
export function crdIcon(group: string): IconComponent {
  for (let at = group; ; ) {
    const icon = BY_GROUP.get(at);
    if (icon) return icon;
    const dot = at.indexOf(".");
    if (dot < 0 || !at.includes(".", dot + 1)) return Icons.crds;
    at = at.slice(dot + 1);
  }
}
