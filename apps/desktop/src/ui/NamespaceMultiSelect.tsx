import React, { useState } from "react";
import { Check, ChevronsUpDown } from "lucide-react";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Command, CommandInput, CommandList, CommandEmpty, CommandGroup, CommandItem } from "@/components/ui/command";
import { cn } from "@/ui/utils";

export interface NamespaceMultiSelectProps {
  namespaces: string[];
  /** Selected namespaces; an empty array means "all namespaces". */
  selection: string[];
  onChange: (selection: string[]) => void;
  ariaLabel?: string;
  className?: string;
}

function summarize(selection: string[]): string {
  if (selection.length === 0) return "All namespaces";
  if (selection.length === 1) return selection[0];
  return `${selection.length} namespaces`;
}

/**
 * Multi-select namespace picker. An empty selection means all namespaces; the
 * popover stays open while toggling so several can be picked at once. Serialized
 * to/from a comma string by `lib/namespaces` for persistence.
 */
export function NamespaceMultiSelect({
  namespaces,
  selection,
  onChange,
  ariaLabel,
  className,
}: NamespaceMultiSelectProps) {
  const [open, setOpen] = useState(false);
  const selected = new Set(selection);

  const toggle = (ns: string) => {
    const next = new Set(selected);
    if (next.has(ns)) next.delete(ns);
    else next.add(ns);
    onChange([...next]);
  };

  // Selected namespaces first (in their given order), then the rest — so a
  // selection doesn't get lost among dozens of other namespaces.
  const orderedNamespaces =
    selected.size === 0
      ? namespaces
      : [...selection.filter((ns) => namespaces.includes(ns)), ...namespaces.filter((ns) => !selected.has(ns))];

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          role="combobox"
          aria-expanded={open}
          aria-label={ariaLabel}
          className={cn(
            "flex h-8 items-center justify-between gap-2 rounded-md border border-input bg-background px-3 text-sm shadow-xs outline-none hover:bg-accent/40 focus-visible:ring-2 focus-visible:ring-ring",
            className,
          )}
        >
          <span className="truncate">{summarize(selection)}</span>
          <ChevronsUpDown className="size-3.5 shrink-0 opacity-50" />
        </button>
      </PopoverTrigger>
      <PopoverContent className="w-64 p-0" align="start">
        <Command>
          <CommandInput placeholder="Search namespaces…" />
          <CommandList>
            <CommandEmpty>No results</CommandEmpty>
            <CommandGroup>
              <CommandItem value="All namespaces" onSelect={() => onChange([])}>
                <Check className={cn("size-3.5 shrink-0", selection.length === 0 ? "opacity-100" : "opacity-0")} />
                <span className="truncate">All namespaces</span>
              </CommandItem>
              {orderedNamespaces.map((ns) => (
                <CommandItem key={ns} value={ns} onSelect={() => toggle(ns)}>
                  <Check className={cn("size-3.5 shrink-0", selected.has(ns) ? "opacity-100" : "opacity-0")} />
                  <span className="truncate">{ns}</span>
                </CommandItem>
              ))}
            </CommandGroup>
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  );
}
