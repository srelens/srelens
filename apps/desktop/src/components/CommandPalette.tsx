import React, { useEffect, useMemo, useState } from "react";
import { Braces, type LucideIcon } from "lucide-react";
import {
  Command,
  CommandDialog,
  CommandInput,
  CommandList,
  CommandEmpty,
  CommandGroup,
  CommandItem,
} from "./ui/command";
import { RESOURCE_LABELS, K8S_KIND, type ResourceKind } from "@srelens/core";
import { listResource } from "@srelens/core";
import { listCrds, type CrdRef } from "@srelens/core";
import { getRecents, pushRecent, recentId, type RecentItem } from "@srelens/core";
import { iconForResourceKind } from "../ui/NavIcon";
import { actionsForKind, type PaletteAction, type PaletteActionCtx } from "@srelens/core";
import { ConfirmDialog } from "../ui";
import { notify } from "@srelens/core";

/** Kinds indexed for name search when the palette opens. */
const SEARCH_KINDS: ResourceKind[] = [
  "pods",
  "deployments",
  "statefulsets",
  "daemonsets",
  "replicasets",
  "jobs",
  "cronjobs",
  "services",
  "ingresses",
  "configmaps",
  "secrets",
  "persistentvolumeclaims",
  "serviceaccounts",
  "nodes",
];

// Views you can jump to. "portforwards" is a virtual view, still navigable.
const NAV_KINDS = (Object.keys(RESOURCE_LABELS) as ResourceKind[]).filter((k) => k !== "overview");

/**
 * Maps a leading query token to a kind, so "pod nginx" or "svc web" narrows
 * the resource search to that kind before name-filtering. Keys include each
 * searchable ResourceKind itself, its singular K8S_KIND label, and common
 * kubectl-style short aliases.
 */
const KIND_TOKENS: Partial<Record<string, ResourceKind>> = (() => {
  const map: Partial<Record<string, ResourceKind>> = {};
  for (const k of SEARCH_KINDS) {
    map[k] = k;
    const label = K8S_KIND[k];
    if (label) map[label.toLowerCase()] = k;
  }
  const aliases: Record<string, ResourceKind> = {
    po: "pods",
    deploy: "deployments",
    deploys: "deployments",
    sts: "statefulsets",
    ds: "daemonsets",
    rs: "replicasets",
    cj: "cronjobs",
    svc: "services",
    ing: "ingresses",
    cm: "configmaps",
    pvc: "persistentvolumeclaims",
    sa: "serviceaccounts",
    no: "nodes",
  };
  Object.assign(map, aliases);
  return map;
})();

interface ResItem {
  kind: ResourceKind;
  namespace: string;
  name: string;
}

/** The resource the palette has drilled into — showing its available actions. */
interface ActionTarget {
  kind: ResourceKind;
  namespace: string | null;
  name: string;
}

function iconForRecent(item: RecentItem): LucideIcon {
  return item.type === "crd" ? Braces : iconForResourceKind(item.kind as ResourceKind);
}

function PaletteIcon({ icon: Icon }: { icon: LucideIcon }) {
  return <Icon aria-hidden="true" />;
}

/** Rank startsWith matches before substring matches, then alphabetically. */
function rankBy<T>(q: string, keyOf: (t: T) => string) {
  return (a: T, b: T) => {
    const A = keyOf(a).toLowerCase();
    const B = keyOf(b).toLowerCase();
    const as = A.startsWith(q) ? 0 : 1;
    const bs = B.startsWith(q) ? 0 : 1;
    return as !== bs ? as - bs : A.localeCompare(B);
  };
}

/**
 * Global command palette (Cmd/Ctrl-K): jump to any resource view, or fuzzy-find
 * a resource by name across kinds and open its detail. Resources are indexed
 * once when the palette opens; filtering is client-side for instant feedback.
 */
export function CommandPalette({
  open,
  onOpenChange,
  context,
  onOpenView,
  onOpenResource,
  onOpenCrd,
  currentViewKind,
  onAfterAction,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  context: string | null;
  onOpenView: (kind: ResourceKind) => void;
  onOpenResource: (kind: ResourceKind, namespace: string | null, name: string) => void;
  onOpenCrd: (crd: CrdRef) => void;
  /** The resource kind of the view the user currently has open, for ranking. */
  currentViewKind?: ResourceKind;
  /** Fired after a dispatched action completes, so the caller can refresh. */
  onAfterAction?: () => void;
}) {
  const [query, setQuery] = useState("");
  const [items, setItems] = useState<ResItem[]>([]);
  const [crds, setCrds] = useState<CrdRef[]>([]);
  const [recents, setRecents] = useState<RecentItem[]>([]);
  const [loading, setLoading] = useState(false);
  // Set once the user picks a resource that has applicable actions — the
  // palette then shows "action mode" for it instead of opening its detail.
  const [target, setTarget] = useState<ActionTarget | null>(null);
  // A destructive action awaiting confirmation.
  const [pendingAction, setPendingAction] = useState<PaletteAction | null>(null);
  const [busy, setBusy] = useState(false);

  // Index resources + CRDs, and read recents, each time the palette opens.
  useEffect(() => {
    if (!open || !context) return;
    let active = true;
    setLoading(true);
    setRecents(getRecents());
    void listCrds(context).then((o) => active && setCrds(o.crds ?? []));
    void Promise.all(
      SEARCH_KINDS.map((k) =>
        listResource(context, K8S_KIND[k], "").then((o) =>
          (o.items ?? []).map((r) => ({ kind: k, namespace: r.namespace, name: r.name })),
        ),
      ),
    ).then((lists) => {
      if (!active) return;
      setItems(lists.flat());
      setLoading(false);
    });
    return () => {
      active = false;
    };
  }, [open, context]);

  useEffect(() => {
    if (open) {
      setQuery("");
      setTarget(null);
      setPendingAction(null);
    }
  }, [open]);

  const q = query.trim().toLowerCase();

  // "Go to": resource kinds plus discovered CRDs (opened as custom-resource views).
  const navMatches = useMemo(() => {
    const kinds = NAV_KINDS.map((k) => ({
      id: `k:${k}`,
      label: RESOURCE_LABELS[k],
      recent: { type: "view", kind: k, label: RESOURCE_LABELS[k] } as RecentItem,
    }));
    const crdNav = crds.map((c) => ({
      id: `crd:${c.name}`,
      label: `${c.kind} (CRD)`,
      recent: { type: "crd", crd: c, label: `${c.kind} (CRD)` } as RecentItem,
    }));
    const all = [...kinds, ...crdNav];
    if (!q) return kinds.slice(0, 6);
    return all
      .filter((n) => n.label.toLowerCase().includes(q))
      .sort(rankBy(q, (n) => n.label))
      .slice(0, 8);
  }, [q, crds]);

  // Dispatch a chosen item. A resource with applicable actions drills into
  // "action mode" instead of opening its detail directly; everything else
  // (views, CRDs, action-less resources) records it in recents and closes.
  function pick(item: RecentItem) {
    if (item.type === "resource") {
      const kind = item.kind as ResourceKind;
      if (actionsForKind(kind).length > 0) {
        pushRecent(item);
        setTarget({ kind, namespace: item.namespace, name: item.name });
        setQuery("");
        return;
      }
    }
    pushRecent(item);
    if (item.type === "view") onOpenView(item.kind as ResourceKind);
    else if (item.type === "resource") onOpenResource(item.kind as ResourceKind, item.namespace, item.name);
    else onOpenCrd(item.crd);
    onOpenChange(false);
  }

  /** Back out of action mode to normal browsing. */
  function goBack() {
    setTarget(null);
    setQuery("");
  }

  function openTargetDetails() {
    if (!target) return;
    onOpenResource(target.kind, target.namespace, target.name);
    onOpenChange(false);
  }

  // Dispatch a chosen action for the current target.
  function selectAction(action: PaletteAction) {
    if (!target) return;
    if (action.opensDialog) {
      // Input-needing actions (scale/debug) are a one-hop to the resource's
      // detail drawer, where their dialogs already live — the palette never
      // renders them itself.
      onOpenResource(target.kind, target.namespace, target.name);
      onOpenChange(false);
      return;
    }
    if (action.destructive) {
      setPendingAction(action);
      return;
    }
    void runAction(action);
  }

  async function runAction(action: PaletteAction) {
    if (!target || !context) return;
    setBusy(true);
    const ctx: PaletteActionCtx = { context, kind: target.kind, namespace: target.namespace, name: target.name };
    const result = await action.run?.(ctx);
    setBusy(false);
    setPendingAction(null);
    if (result && "error" in result && result.error) {
      notify.error(`Failed: ${action.label}`, result.error);
    }
    onOpenChange(false);
    onAfterAction?.();
  }

  const resMatches = useMemo(() => {
    if (!q) return [];
    // A leading kind token ("pod nginx", "svc web") narrows the search to
    // that kind before filtering by the rest of the query as the name.
    const [first, ...rest] = q.split(/\s+/);
    const kind = KIND_TOKENS[first];
    const nameQuery = kind ? rest.join(" ") : q;
    const scoped = kind ? items.filter((r) => r.kind === kind) : items;
    return scoped
      .filter((r) => r.name.toLowerCase().includes(nameQuery))
      .sort(rankBy(nameQuery, (r) => r.name))
      .slice(0, 50);
  }, [items, q]);

  // Actions available for the drilled-into resource, optionally text-filtered.
  const actionMatches = useMemo(() => {
    if (!target) return [];
    const acts = actionsForKind(target.kind);
    if (!q) return acts;
    return acts.filter((a) => a.label.toLowerCase().includes(q));
  }, [target, q]);

  // Context-aware ranking: when browsing with no query, surface recents for
  // the resource kind currently in view ahead of the rest of Recent.
  const quickRecents = useMemo(() => {
    if (q || !currentViewKind) return [];
    return recents.filter((r) => r.type === "resource" && r.kind === currentViewKind);
  }, [q, currentViewKind, recents]);

  const otherRecents = useMemo(() => {
    if (!quickRecents.length) return recents;
    const quickIds = new Set(quickRecents.map(recentId));
    return recents.filter((r) => !quickIds.has(recentId(r)));
  }, [recents, quickRecents]);

  return (
    <CommandDialog open={open} onOpenChange={onOpenChange} title="Search" description="Jump to a view or resource">
      <Command shouldFilter={false}>
        <CommandInput
          value={query}
          onValueChange={setQuery}
          placeholder={target ? `Actions for ${target.name}…` : "Search resources and views…"}
          onKeyDown={(e) => {
            if (e.key === "Backspace" && query === "" && target) {
              e.preventDefault();
              goBack();
            }
          }}
        />
        <CommandList>
          <CommandEmpty>{loading ? "Indexing…" : "No results"}</CommandEmpty>

          {target ? (
            <CommandGroup heading={`Actions for ${target.name}`}>
              {actionMatches.map((a) => (
                <CommandItem key={a.capabilityId + a.label} value={`action:${a.label}`} onSelect={() => selectAction(a)}>
                  {a.label}
                </CommandItem>
              ))}
              <CommandItem value="palette:open-details" onSelect={openTargetDetails}>
                Open details
              </CommandItem>
              <CommandItem value="palette:back" onSelect={goBack}>
                ← Back
              </CommandItem>
            </CommandGroup>
          ) : (
            <>
              {!q && quickRecents.length > 0 && currentViewKind && (
                <CommandGroup heading={`Quick: ${RESOURCE_LABELS[currentViewKind]}`}>
                  {quickRecents.map((r) => (
                    <CommandItem key={`quick:${r.type}:${r.label}`} value={`quick:${r.label}`} onSelect={() => pick(r)}>
                      <PaletteIcon icon={iconForRecent(r)} />
                      <span className="truncate">{r.label}</span>
                    </CommandItem>
                  ))}
                </CommandGroup>
              )}

              {!q && otherRecents.length > 0 && (
                <CommandGroup heading="Recent">
                  {otherRecents.map((r) => (
                    <CommandItem key={`recent:${r.type}:${r.label}`} value={`recent:${r.label}`} onSelect={() => pick(r)}>
                      <PaletteIcon icon={iconForRecent(r)} />
                      <span className="truncate">{r.label}</span>
                      <span className="ml-2 rounded bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">
                        {r.type === "resource" ? K8S_KIND[r.kind as ResourceKind] : r.type === "crd" ? "CRD" : "view"}
                      </span>
                    </CommandItem>
                  ))}
                </CommandGroup>
              )}

              {navMatches.length > 0 && (
                <CommandGroup heading="Go to">
                  {navMatches.map((n) => (
                    <CommandItem key={n.id} value={`nav:${n.id}`} onSelect={() => pick(n.recent)}>
                      <PaletteIcon icon={iconForRecent(n.recent)} />
                      {n.label}
                    </CommandItem>
                  ))}
                </CommandGroup>
              )}

              {resMatches.length > 0 && (
                <CommandGroup heading={loading ? "Resources (indexing…)" : "Resources"}>
                  {resMatches.map((r, i) => (
                    <CommandItem
                      key={`${r.kind}/${r.namespace}/${r.name}/${i}`}
                      value={`res:${i}`}
                      onSelect={() =>
                        pick({
                          type: "resource",
                          kind: r.kind,
                          namespace: r.namespace || null,
                          name: r.name,
                          label: r.name,
                        })
                      }
                    >
                      <PaletteIcon icon={iconForResourceKind(r.kind)} />
                      <span className="truncate">{r.name}</span>
                      <span className="ml-2 rounded bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">
                        {K8S_KIND[r.kind]}
                      </span>
                      {r.namespace && (
                        <span className="ml-auto truncate pl-2 text-xs text-muted-foreground">{r.namespace}</span>
                      )}
                    </CommandItem>
                  ))}
                </CommandGroup>
              )}
            </>
          )}
        </CommandList>
      </Command>

      {pendingAction && target && (
        <ConfirmDialog
          title={`${pendingAction.label}?`}
          message={
            <p style={{ marginTop: 0 }}>
              {pendingAction.label} <code>{target.name}</code>
              {target.namespace ? (
                <>
                  {" "}
                  in <code>{target.namespace}</code>
                </>
              ) : null}
              ? This cannot be undone.
            </p>
          }
          confirmLabel={pendingAction.label}
          danger
          busy={busy}
          onConfirm={() => void runAction(pendingAction)}
          onCancel={() => setPendingAction(null)}
        />
      )}
    </CommandDialog>
  );
}
