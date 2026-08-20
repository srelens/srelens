import React, { useEffect, useRef, useState } from "react";
import { ChevronRight } from "lucide-react";
import { RESOURCE_LABELS, type ResourceKind } from "../lib/kinds";
import { CustomResourceGroup } from "./CustomResourceGroup";
import { iconForResourceKind, NavIcon } from "../ui/NavIcon";
import { cn } from "@/lib/utils";
import type { CrdRef } from "../lib/crds";
import { contextDisplayName, type ContextProfiles } from "../lib/settings";

const NAV_SECTIONS: Array<{ heading: string; kinds: ResourceKind[] }> = [
  { heading: "Cluster", kinds: ["overview", "nodes", "namespaces", "events"] },
  {
    heading: "Workloads",
    kinds: ["pods", "deployments", "statefulsets", "daemonsets", "replicasets", "jobs", "cronjobs"],
  },
  {
    heading: "Config",
    kinds: [
      "configmaps",
      "secrets",
      "resourcequotas",
      "limitranges",
      "horizontalpodautoscalers",
      "poddisruptionbudgets",
      "priorityclasses",
      "runtimeclasses",
      "leases",
      "mutatingwebhookconfigurations",
      "validatingwebhookconfigurations",
    ],
  },
  {
    heading: "Network",
    kinds: ["services", "endpointslices", "endpoints", "ingresses", "ingressclasses", "networkpolicies", "portforwards"],
  },
  { heading: "Storage", kinds: ["persistentvolumeclaims", "persistentvolumes", "storageclasses"] },
  {
    heading: "Access Control",
    kinds: ["serviceaccounts", "clusterroles", "roles", "clusterrolebindings", "rolebindings"],
  },
  { heading: "Helm", kinds: ["helmreleases"] },
];

/** A rotating chevron disclosure indicator. */
function Caret({ open }: { open: boolean }) {
  return (
    <ChevronRight
      aria-hidden="true"
      className={cn("size-3.5 shrink-0 text-muted-foreground transition-transform", open && "rotate-90")}
    />
  );
}

/** A collapsible row used at every level of the tree. */
function TreeRow({
  open,
  onToggle,
  children,
  className,
}: {
  open: boolean;
  onToggle: () => void;
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <button
      type="button"
      onClick={onToggle}
      // The caret is the only signal that these rows expand, and it is drawn
      // with an aria-hidden icon — so the state has to be said out loud.
      aria-expanded={open}
      className={cn(
        "fl-tree-row flex w-full items-center gap-1.5 rounded-md py-0.5 text-left hover:bg-muted/50",
        className,
      )}
    >
      <Caret open={open} />
      {children}
    </button>
  );
}

/**
 * Resource navigation: one collapsible tree per opened cluster
 * (Cluster → Group → Resource), plus a dynamic Custom Resources group. Every
 * level expands/collapses; a drag handle resizes the panel.
 */
export function Sidebar({
  clusters = [],
  activeCluster,
  activeKind,
  activeCrd,
  onSelect,
  onSelectCrd,
  width = 200,
  onResize,
  contextProfiles = {},
}: {
  clusters: string[];
  activeCluster?: string | null;
  activeKind: ResourceKind;
  activeCrd?: CrdRef | null;
  onSelect: (cluster: string, kind: ResourceKind) => void;
  onSelectCrd: (cluster: string, crd: CrdRef) => void;
  width?: number;
  onResize?: (width: number) => void;
  contextProfiles?: ContextProfiles;
}) {
  const handleRef = useRef<HTMLDivElement>(null);
  const startX = useRef(0);
  const startW = useRef(0);
  const widthRef = useRef(width);
  widthRef.current = width;

  // Open/closed state keyed so each cluster has independent disclosure.
  const [openClusters, setOpenClusters] = useState<Record<string, boolean>>({});
  const [openGroups, setOpenGroups] = useState<Record<string, boolean>>({});
  const clusterOpen = (c: string) => openClusters[c] ?? c === activeCluster;
  const groupOpen = (key: string, openByDefault: boolean) => openGroups[key] ?? openByDefault;
  const toggleCluster = (c: string) =>
    setOpenClusters((s) => ({ ...s, [c]: !clusterOpen(c) }));
  const toggleGroup = (key: string, openByDefault: boolean) =>
    setOpenGroups((s) => ({ ...s, [key]: !groupOpen(key, openByDefault) }));

  useEffect(() => {
    if (!onResize) return;
    const handle = handleRef.current;
    if (!handle) return;
    function move(e: MouseEvent) {
      onResize!(Math.max(168, Math.min(480, startW.current + (e.clientX - startX.current))));
    }
    function up() {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
      document.body.style.userSelect = "";
    }
    function down(e: MouseEvent) {
      e.preventDefault();
      startX.current = e.clientX;
      startW.current = widthRef.current;
      document.body.style.userSelect = "none";
      window.addEventListener("mousemove", move);
      window.addEventListener("mouseup", up);
    }
    handle.addEventListener("mousedown", down);
    return () => {
      handle.removeEventListener("mousedown", down);
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
      document.body.style.userSelect = "";
    };
  }, [onResize]);

  return (
    // Named, because a screen reader announcing "complementary" tells the user
    // nothing about which of the app's two navigation regions they are in.
    <aside className="fl-sidebar" aria-label="Cluster resources">
      <div className="flex flex-col p-1 text-sm">
        {clusters.map((cluster) => (
          <React.Fragment key={cluster}>
            {/* Cluster (level 0) */}
            <TreeRow
              open={clusterOpen(cluster)}
              onToggle={() => toggleCluster(cluster)}
              className="fl-sidebar__cluster-row pl-1.5 font-medium"
            >
              <span className="size-2 shrink-0 rounded-full bg-emerald-500" />
              <span className="truncate" title={cluster}>
                {contextDisplayName(cluster, contextProfiles[cluster])}
              </span>
            </TreeRow>

            {clusterOpen(cluster) &&
              NAV_SECTIONS.map((section) => {
                const gkey = `${cluster}|${section.heading}`;
                const openByDefault =
                  cluster === activeCluster && section.kinds.includes(activeKind);
                const open = groupOpen(gkey, openByDefault);
                return (
                  <React.Fragment key={section.heading}>
                    {/* Group (level 1) */}
                    <TreeRow
                      open={open}
                      onToggle={() => toggleGroup(gkey, openByDefault)}
                      className="fl-sidebar__group-row pl-3 text-muted-foreground"
                    >
                      <span className="truncate">{section.heading}</span>
                    </TreeRow>
                    {open &&
                      section.kinds.map((kind) => {
                        const active = cluster === activeCluster && kind === activeKind;
                        return (
                          <button
                            key={kind}
                            type="button"
                            aria-current={active ? "page" : undefined}
                            onClick={() => onSelect(cluster, kind)}
                            className={cn(
                              "fl-sidebar__resource-row flex w-full items-center gap-2 rounded-md py-0.5 pr-2 pl-7 text-left transition-colors",
                              active
                                ? "fl-sidebar__resource-row--active bg-muted font-medium text-foreground"
                                : "text-muted-foreground hover:bg-muted/50 hover:text-foreground",
                            )}
                          >
                            <NavIcon icon={iconForResourceKind(kind)} />
                            <span className="truncate" title={RESOURCE_LABELS[kind]}>
                              {RESOURCE_LABELS[kind]}
                            </span>
                          </button>
                        );
                      })}
                  </React.Fragment>
                );
              })}

            {/* Custom Resources (level 1, lazy-loaded per cluster, default collapsed) */}
            {clusterOpen(cluster) && (
              <CustomResourceGroup
                cluster={cluster}
                open={false}
                activeCrd={cluster === activeCluster ? activeCrd : null}
                onSelectCrd={(crd) => onSelectCrd(cluster, crd)}
                renderTrigger={(open, onToggle) => (
                  <TreeRow open={open} onToggle={onToggle} className="fl-sidebar__group-row pl-3 text-muted-foreground">
                    <span className="truncate">Custom Resources</span>
                  </TreeRow>
                )}
              />
            )}
          </React.Fragment>
        ))}
      </div>
      <div className="fl-sidebar__resize" ref={handleRef} aria-hidden="true" />
    </aside>
  );
}
