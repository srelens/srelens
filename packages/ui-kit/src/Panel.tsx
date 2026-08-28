import type { ReactNode } from "react";
import { cx } from "./cx";

export interface PanelProps {
  title?: ReactNode;
  children: ReactNode;
  className?: string;
}

/**
 * A bordered surface section with an optional title.
 *
 * The classic version wrapped shadcn's Card; this is the design's own `.card`,
 * which carries the same idea — a lifted surface with a ruled header. The
 * `title`/`children` API is what callers depend on and is unchanged. (#318)
 */
export function Panel({ title, children, className }: PanelProps) {
  return (
    <section className={cx("card", className)}>
      {title != null && (
        <div className="card-head">
          <div className="card-title">{title}</div>
        </div>
      )}
      <div className="section-body">{children}</div>
    </section>
  );
}
