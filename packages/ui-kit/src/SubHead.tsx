import type { ReactNode } from "react";
import { cx } from "./cx";

/**
 * The two voices the design gives a block's name.
 *
 * `bold` is the detail body's line — the resource pane's mock draws "a small
 * bold section heading" and every call site in that pane wants it.
 *
 * `caps` is the design's structural signpost: 10 px, weight 600, `0.07em`,
 * uppercase, `--ink-faint` — the same recipe `.pane-head` wears (design §C.3).
 * The cluster overview's bands are headed in it, so a heading and the pane head
 * above it read as one system rather than two.
 */
export type SubHeadVariant = "bold" | "caps";

export interface SubHeadProps {
  children: ReactNode;
  className?: string;
  /** Which of the two voices names this block. Defaults to `bold`. */
  variant?: SubHeadVariant;
}

/**
 * The bold line that labels a group inside a panel — Labels, Annotations,
 * Conditions, Containers — as distinct from the ruled bar that heads the panel
 * itself.
 *
 * An `h3`, not the mock's styled div. Every call site in the design names the
 * block beneath it, which is what a heading is; rendered as a div they are all
 * invisible to anyone reading the page by its outline, and that is the finding
 * `Panel`'s `h2` came from. The level is fixed for the same reason it is fixed
 * there: this sits inside a panel, so it is the level below one, and no group
 * in the design nests inside another. Preflight strips a heading's own size and
 * weight, so the utilities are what keep it looking like the mock's line rather
 * than a browser heading. (#320)
 *
 * The two variants emit DIFFERENT class sets rather than one overriding the
 * other. Both would be Tailwind utilities in the same layer, where the winner
 * is decided by the order the generated stylesheet happens to put them in and
 * not by the order they are written — so an override of `text-[0.75rem]` from
 * outside is a coin flip. `caps` is a components-layer class for the same
 * reason, and nothing else here sets its size or weight.
 */
export function SubHead({ children, className, variant = "bold" }: SubHeadProps) {
  return (
    <h3 className={cx(variant === "caps" ? "subhead-caps" : "text-[0.75rem] font-semibold", className)}>
      {children}
    </h3>
  );
}
