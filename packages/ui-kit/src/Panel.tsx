import type { ReactNode } from "react";
import { cx } from "./cx";
import { filled } from "./slot";

export interface PanelProps {
  title?: ReactNode;
  /** A line under the title saying what the section holds. */
  description?: ReactNode;
  children: ReactNode;
  className?: string;
}

/**
 * A bordered surface section with an optional title.
 *
 * The classic version wrapped shadcn's Card; this is the design's own `.card`,
 * which carries the same idea — a lifted surface with a ruled header. The
 * `title`/`children` API is what callers depend on and is unchanged. (#318)
 *
 * `description` and the heading come from the classic `SectionPanel`, which was
 * this component with one extra line and an `h2`. Rather than carry two near-identical panels into the
 * kit, the line moved here — so the header appears when there is anything to
 * put in it, which now includes a description standing on its own. (#318)
 */
export function Panel({ title, description, children, className }: PanelProps) {
  return (
    <section className={cx("card", className)}>
      {(filled(title) || filled(description)) && (
        <div className="card-head flex-col items-start gap-0.5">
          {/* A heading, not a div: these are the sections of a page, and the
              classic SectionPanel this replaces rendered an h2. Fixed rather
              than configurable — no panel in the app nests inside another.
              (#325 review) */}
          {filled(title) && <h2 className="card-title">{title}</h2>}
          {filled(description) && <p className="text-[0.75rem] text-muted">{description}</p>}
        </div>
      )}
      <div className="section-body">{children}</div>
    </section>
  );
}
