import React from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from "@/components/ui/dialog";
import { formatShortcut, groupedShortcuts, isApplePlatform } from "@srelens/core";

/**
 * The `?` overlay: every shortcut the app answers to, grouped by where it
 * applies (#160). Its content comes from the shortcut registry, so a binding
 * cannot be added or changed without this list following.
 */
export function ShortcutCheatSheet({
  open,
  onOpenChange,
  desktop,
  apple = isApplePlatform(),
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Desktop build: browser builds hide the keys the browser itself owns. */
  desktop: boolean;
  /** Label the modifier as `⌘` rather than `Ctrl`. Injectable for tests. */
  apple?: boolean;
}) {
  const groups = groupedShortcuts(desktop, apple);
  // An IDREF list is whitespace-separated, so "shortcuts-Command palette" reads
  // as two references, neither of which exists, and the section loses its name.
  const headingId = (group: string) => `shortcuts-${group.replace(/\s+/g, "-").toLowerCase()}`;
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[80vh] overflow-y-auto sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Keyboard shortcuts</DialogTitle>
          <DialogDescription>
            Press <kbd className="fl-kbd">?</kbd> any time to bring this back.
          </DialogDescription>
        </DialogHeader>
        <div className="flex flex-col gap-4">
          {groups.map(([group, shortcuts]) => (
            <section key={group} aria-labelledby={headingId(group)}>
              <h3
                id={headingId(group)}
                className="mb-1 text-xs font-medium uppercase tracking-wide text-muted-foreground"
              >
                {group}
              </h3>
              <dl className="flex flex-col gap-1">
                {shortcuts.map((shortcut) => (
                  <div key={shortcut.id} className="flex items-baseline gap-3 text-sm">
                    <dt className="min-w-24 shrink-0">
                      <kbd className="fl-kbd">{formatShortcut(shortcut, apple)}</kbd>
                    </dt>
                    <dd className="min-w-0 text-muted-foreground">{shortcut.description}</dd>
                  </div>
                ))}
              </dl>
            </section>
          ))}
        </div>
      </DialogContent>
    </Dialog>
  );
}
