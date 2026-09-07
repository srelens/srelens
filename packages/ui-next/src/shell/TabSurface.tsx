import type { ReactNode } from "react";
import { PortalScopeProvider, usePortalHost } from "@srelens/ui-kit";

/**
 * One tab's view, kept mounted whether or not it is the one on screen.
 *
 * Switching tabs hides and shows rather than unmounting and remounting, the
 * way a desktop app behaves: coming back is instant, scroll position is where
 * it was, and a load that was in flight finishes instead of starting over. The
 * parent spec calls this out for Logs, where a stream that restarted on every
 * switch would be unusable; it is cheaper to have from the first tab than to
 * retrofit.
 *
 * `hidden` rather than a class: it removes the subtree from the accessibility
 * tree and the tab order as well as from view, which `display: none` also
 * does, but the attribute says what is meant. Every surface is absolutely
 * positioned over the same box, so the visible one is the only one laid out.
 *
 * It is also a portal scope, which is what keeps a dialog opened in this tab
 * inside this tab. Two halves of one bug (#357). A dialog portalled to
 * `document.body` covered the tab strip, the cluster rail and the status bar,
 * and Radix's focus trap and `aria-hidden` isolation made every other tab
 * unreachable until it was dismissed — and a portal escapes `hidden`, so
 * lifting that blocking on its own would have left the first tab's dialog
 * sitting on top of whatever tab the reader moved to. Mounting the dialog in
 * this subtree answers both: it is hidden with the tab, and it covers only the
 * tab, so the strip and the rail stay live behind it.
 *
 * Hence the two children rather than one. The content goes in its own wrapper
 * so it can be marked `inert` while a dialog covers it — the overlay stops the
 * pointer and nothing else, and a non-modal Radix dialog applies `aria-hidden`
 * to nothing, so without this the covered screen is still a tab stop and still
 * on a screen reader's cursor. The portal target has to be that wrapper's
 * *sibling*: an inert subtree takes its own descendants with it, so a dialog
 * mounted inside the content it is meant to be covering would be inert too.
 *
 * The scope is told whether this tab is the one on screen, because a hidden
 * tab keeps its dialogs mounted: without it, a dialog left open on a tab the
 * reader has switched away from would still answer their Escape. (#357)
 *
 * The target carries no layout of its own. This surface is `absolute`, so it
 * is already the containing block a dialog's `absolute inset-0` overlay
 * resolves against; an empty static div is a zero-sized flex item and changes
 * nothing about how the tab is laid out.
 */
export function TabSurface({ visible, children }: { visible: boolean; children: ReactNode }) {
  const { ref, layered, scope } = usePortalHost(visible);

  return (
    <div data-slot="tab-surface" hidden={!visible} className="absolute inset-0 flex min-h-0 flex-col">
      <PortalScopeProvider scope={scope}>
        {/* The column the screens are written against, unchanged: it was this
            element's parent that held it before the wrapper existed. */}
        <div data-slot="tab-content" inert={layered} className="flex min-h-0 flex-1 flex-col">
          {children}
        </div>
        <div data-slot="tab-layers" ref={ref} />
      </PortalScopeProvider>
    </div>
  );
}
