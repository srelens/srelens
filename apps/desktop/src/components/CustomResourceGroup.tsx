import React, { useEffect, useMemo, useState } from "react";
import { Braces, ChevronRight } from "lucide-react";
import { listCrds, type CrdRef } from "@srelens/core";
import { cn } from "@/ui/utils";

/**
 * The sidebar's "Custom Resources" group for one cluster: lazily discovers CRDs
 * the first time it's opened, then lists them grouped by API group → kind.
 * `listCrdsFn` is injectable for testing.
 */
export function CustomResourceGroup({
  cluster,
  open,
  activeCrd,
  onSelectCrd,
  renderTrigger,
  listCrdsFn = listCrds,
}: {
  cluster: string;
  open: boolean;
  activeCrd?: CrdRef | null;
  onSelectCrd: (crd: CrdRef) => void;
  renderTrigger: (open: boolean, onToggle: () => void) => React.ReactNode;
  listCrdsFn?: typeof listCrds;
}) {
  const [crds, setCrds] = useState<CrdRef[] | null>(null);
  const [error, setError] = useState("");
  const [openApiGroups, setOpenApiGroups] = useState<Record<string, boolean>>({});
  const [localOpen, setLocalOpen] = useState(open);

  useEffect(() => setLocalOpen(open), [open]);

  useEffect(() => {
    if (!localOpen || crds !== null) return; // load once, when first opened
    let active = true;
    void listCrdsFn(cluster).then((out) => {
      if (!active) return;
      if (out.error) setError(out.error);
      else setCrds(out.crds ?? []);
    });
    return () => {
      active = false;
    };
  }, [localOpen, cluster, crds, listCrdsFn]);

  const byGroup = useMemo(() => {
    const m = new Map<string, CrdRef[]>();
    for (const c of crds ?? []) {
      const list = m.get(c.group) ?? [];
      list.push(c);
      m.set(c.group, list);
    }
    return [...m.entries()].sort((a, b) => a[0].localeCompare(b[0]));
  }, [crds]);

  return (
    <div>
      {renderTrigger(localOpen, () => setLocalOpen((o) => !o))}
      {localOpen && (
        <div className="flex flex-col">
          {crds === null && !error && (
            <span className="py-0.5 pl-7 text-xs text-muted-foreground">Loading…</span>
          )}
          {error && <span className="py-0.5 pl-7 text-xs text-muted-foreground">Unavailable</span>}
          {crds && crds.length === 0 && (
            <span className="py-0.5 pl-7 text-xs text-muted-foreground">None</span>
          )}
          {byGroup.map(([group, list]) => {
            const gopen = openApiGroups[group] ?? false;
            return (
              <div key={group}>
                <button
                  type="button"
                  onClick={() => setOpenApiGroups((s) => ({ ...s, [group]: !gopen }))}
                  className="flex w-full items-center gap-1.5 rounded-md py-0.5 pl-7 pr-2 text-left text-xs text-muted-foreground hover:bg-muted/50"
                >
                  <ChevronRight
                    className={cn("size-3.5 shrink-0 transition-transform", gopen && "rotate-90")}
                  />
                  <span className="truncate" title={group}>
                    {group}
                  </span>
                </button>
                {gopen &&
                  list.map((crd) => {
                    const active = activeCrd?.name === crd.name;
                    return (
                      <button
                        key={crd.name}
                        type="button"
                        aria-current={active ? "page" : undefined}
                        onClick={() => onSelectCrd(crd)}
                        className={cn(
                          "flex w-full items-center gap-2 rounded-md py-0.5 pr-2 pl-11 text-left text-xs transition-colors",
                          active
                            ? "bg-muted font-medium text-foreground"
                            : "text-muted-foreground hover:bg-muted/50 hover:text-foreground",
                        )}
                      >
                        <Braces className="fl-crd-icon" aria-hidden="true" />
                        <span className="truncate">{crd.kind}</span>
                      </button>
                    );
                  })}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
