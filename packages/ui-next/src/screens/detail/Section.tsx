import { createContext, useContext, type ReactNode } from "react";
import { Section as KitSection } from "@srelens/ui-kit";
import { setSectionOpen, useSectionOpen } from "../../lib/sectionFolds";

/**
 * The kind whose folds are being remembered, or `null` where nothing is.
 *
 * A context rather than a prop threaded through ten bodies: every block in a
 * detail belongs to the one subject the host is drawing, and a `kind` passed
 * down by hand is a `kind` some future body forgets to pass. It carries no
 * store and no state — just the key the memory is looked up under.
 */
const KindContext = createContext<string | null>(null);

/**
 * Everything inside this remembers which of its blocks the reader opened, for
 * this kind.
 *
 * Rendered ONCE by each screen, around its own pane, so the peek and the full
 * tab get the same memory without either of them knowing the other exists —
 * the key is the KIND, and the store is neither screen's. They are different
 * layouts of one subject; a block a reader cares about is the same block in
 * both.
 *
 * A provider renders no element, so the run of sections beneath it is still a
 * run of direct siblings and `.section + .section` still draws every hairline.
 */
export function SectionMemory({ kind, children }: { kind: string; children: ReactNode }) {
  return <KindContext.Provider value={kind}>{children}</KindContext.Provider>;
}

export interface DetailSectionProps {
  /**
   * The small bold line naming the block, and — unless {@link
   * DetailSectionProps.id} says otherwise — what the memory is keyed on.
   * Left off for the lead fact list, which the design heads with nothing.
   */
  title?: ReactNode;
  /**
   * What to remember this block as, for a heading that is not a fixed string.
   * `Data (3 keys)` becomes `Data (4 keys)` the moment someone edits the
   * ConfigMap, and a memory keyed on the heading would go with it.
   */
  id?: string;
  /**
   * Open this block on a first visit, rather than shut.
   *
   * FOR ONE SHAPE ONLY: a pane whose whole content is this single titled
   * block, which would otherwise open showing nothing at all — the peek's
   * Containers tab, a ConfigMap's Data. That is the same hostility argument
   * the unheaded lead fact list is exempted under, and the reader's "first
   * open should keep everything collapsed" still holds everywhere else.
   *
   * Declared by the body that draws the block, because a section cannot see
   * its siblings: a rule that counted them would be wrong the moment a Labels
   * block turned up beside it.
   *
   * It still folds and is still remembered. Shutting one is recorded as the
   * reader's own choice and survives the launch, exactly as opening a shut one
   * does — only the starting point differs.
   */
  defaultOpen?: boolean;
  children: ReactNode;
  className?: string;
}

/**
 * A block of a resource detail: the kit's `Section`, wired to the memory of
 * what this kind's reader has opened.
 *
 * EVERY BODY IN THIS DIRECTORY IMPORTS `Section` FROM HERE, never from the
 * kit, and `Section.test.tsx` sweeps the directory to keep it that way. A body
 * that reached past this would render a block that never folds and never
 * remembers, and would look exactly right on the day it was written.
 *
 * The split is the kit's usual one, the same `Sidebar`/`ResizeHandle` take for
 * the peek's width: the kit draws a disclosure and reports a toggle, this
 * module decides what open means and where it is written down. The kit holds
 * no app state and touches no storage.
 *
 * WHAT STARTS OPEN. Almost nothing: the reader asked for everything
 * collapsed. The exception is {@link DetailSectionProps.defaultOpen}, for a
 * block that is the only thing in its pane — see there.
 *
 * WHAT DOES NOT FOLD AT ALL, and both cases are deliberate:
 *
 * - An untitled block. The design heads the first fact list with nothing, so
 *   there is no line to hang a control on — and a pane that opens showing
 *   nothing at all is hostile. It stays open.
 * - A block drawn with no {@link SectionMemory} around it. There is no kind to
 *   key the memory on, so there is nothing to remember, and a block that
 *   folded with nowhere to record it would fold itself back on every render.
 *   That is what a body rendered on its own in a test is, and it is the shape
 *   every one of them had before this existed.
 *
 * WHAT THIS DOES NOT REACH. `AnnotationsSection` gates a Secret's annotations
 * behind a toggle of its own, because a `kubectl apply`-managed Secret carries
 * its whole base64 `data` map inside an annotation. This memory sits ABOVE
 * that gate and can only ever disclose the gate's own button: the toggle keeps
 * its own state, starts shut on every mount, and no value in this store is
 * read by it. A remembered "open" on `Secret`/`Annotations` therefore shows a
 * reader the words "Show 1 annotation" and nothing else. (#331)
 */
export function Section({ title, id, defaultOpen = false, children, className }: DetailSectionProps) {
  const kind = useContext(KindContext);
  // A heading that is not a plain string has no stable key to be remembered
  // under, and inventing one from a ReactNode is how a memory starts pointing
  // at the wrong block. Such a section simply does not fold.
  const sectionId = id ?? (typeof title === "string" ? title : null);
  const open = useSectionOpen(kind, sectionId, defaultOpen);

  if (kind === null || sectionId === null) {
    return (
      <KitSection title={title} className={className}>
        {children}
      </KitSection>
    );
  }

  return (
    <KitSection
      title={title}
      className={className}
      open={open}
      onToggle={(next) => setSectionOpen(kind, sectionId, next, { defaultOpen })}
    >
      {children}
    </KitSection>
  );
}
