import { useId, type ReactNode } from "react";
import { cx } from "./cx";
import { filled } from "./slot";

export interface SideRailProps {
  /** The main region. Gets `min-w-0` so a wide table scrolls inside itself. */
  children: ReactNode;
  /** The rail's own heading, small caps. */
  head: ReactNode;
  /**
   * The MAIN region's heading, in the same small-caps strip — the cluster
   * overview's `prod-eu · v1.31.4`, which the design draws level with the
   * rail's own head.
   *
   * Optional because most screens put a filter bar or a table straight under
   * the toolbar and have nothing to say twice. Not a landmark label the way
   * {@link SideRailProps.head} is: the main region is where the reader already
   * is, and a second `complementary` around it would be noise.
   */
  mainHead?: ReactNode;
  /** What the rail holds — a run of `Section`s. */
  rail: ReactNode;
  /** Per screen. Events 250, custom resources 264, overview 286, logs 272. */
  width: number;
  className?: string;
}

/**
 * A main region beside a fixed-width rail — the shape almost every screen in
 * the design has.
 *
 * The resource list hand-rolled this once, with a comment saying the kit had
 * no split layout and one call site did not justify inventing one. Four
 * screens want it now, at four different widths, so here it is.
 *
 * **THIS DOES NOT RESIZE, AND THAT IS THE COMPONENT.** The design makes
 * exactly two things draggable: the detail inspector and the left sidebar.
 * Everything else is fixed. The inspector's grip carries a `ResizeObserver`, a
 * clamp measured against the row it shares rather than the window, a persisted
 * width and arrow keys that invert on a left edge — four mechanisms, each of
 * them arrived at through a defect. Copying them here "for consistency" would
 * move four solved problems somewhere none of them exists. The suite asserts
 * this component offers no `separator`, which is what a grip announces itself
 * as, so adding one turns a design decision into a failing test rather than a
 * silent drift.
 *
 * The width is a number, not a class: four one-off widths are four values, not
 * a scale, and `.side-rail` deliberately names no width of its own so the prop
 * is the only answer.
 *
 * **The one property carried over from the hand-rolled version is `min-w-0` on
 * the main region.** A flex item will not shrink below its content, so without
 * it a table wide enough to scroll pushes the rail off the window and the
 * whole screen scrolls sideways instead. It is exactly the property a fresh
 * component omits and nobody notices until a kind has fifteen columns.
 *
 * The rail is a `complementary` landmark named by its own head, so it is
 * reachable as a region — this is the "About this kind" / "By reason" material
 * a reader may want to jump to and may equally want to skip. `Inspector` is
 * pointedly NOT one of these: it is a `region` because it nests inside
 * whatever container the caller picks, and a second complementary around it
 * would be noise. The distinction is who owns the container. Here, this does.
 *
 * The rail's children are rendered as direct children of one box, with nothing
 * per child: a rail's content is a run of `Section`s, and `.section + .section`
 * is what rules between them. A wrapper around each would break that adjacency
 * and the run would read as one undivided block.
 *
 * Scrolling is split the way the design's regions are — each scrolls on its
 * own. The rail body does it here; the main region does not, because what
 * scrolls inside it (a table, a log) is the caller's and usually wants a
 * `.scroll` box of its own beneath a header that stays put.
 */
export function SideRail({ children, head, mainHead, rail, width, className }: SideRailProps) {
  const headId = useId();

  return (
    <div className={cx("flex min-h-0 flex-1", className)}>
      <div data-slot="rail-main" className="flex min-h-0 min-w-0 flex-1 flex-col">
        {filled(mainHead) && (
          <div data-slot="main-head" className="pane-head">
            {mainHead}
          </div>
        )}
        {children}
      </div>
      {/* Named by the head rather than by an `aria-label` prop: the head is a
          node, the label would be a second string saying the same thing, and
          two of them drift. */}
      <aside aria-labelledby={headId} className="side-rail" style={{ width }}>
        <div id={headId} className="pane-head">
          {head}
        </div>
        <div data-slot="rail-body" className="side-rail-body">
          {rail}
        </div>
      </aside>
    </div>
  );
}
