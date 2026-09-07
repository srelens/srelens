import { cx } from "./cx";

export interface BreadcrumbProps {
  /** The trail from the outermost scope to the thing being looked at. */
  parts: string[];
  className?: string;
}

/**
 * Where you are: context, namespace, resource.
 *
 * An ordered list rather than the mock's row of sibling spans. A trail has a
 * sequence and a depth, and a screen reader reading a list says how many steps
 * there are and which one it is on; spans say only their words. The separators
 * are drawn but hidden, so nobody hears "prod-eu slash kube-system slash".
 *
 * The last part carries `aria-current="page"`. The mock only gave it a
 * different colour, which tells a sighted reader where they are and tells
 * everyone else nothing — and the colour is the decoration, not the fact.
 * (#320)
 *
 * An empty trail renders nothing rather than an empty `nav`: a landmark is
 * offered in the list a screen reader builds of the page, and one that leads
 * nowhere is worse than an absent one.
 */
export function Breadcrumb({ parts, className }: BreadcrumbProps) {
  if (parts.length === 0) return null;
  return (
    <nav aria-label="Breadcrumb" className={cx("crumb", className)}>
      <ol className="flex items-center gap-1.5">
        {parts.map((part, i) => {
          const current = i === parts.length - 1;
          return (
            <li key={`${part}-${i}`} className="flex items-center gap-1.5">
              {i > 0 && (
                <span aria-hidden="true" className="text-faint">
                  /
                </span>
              )}
              <span
                aria-current={current ? "page" : undefined}
                className={current ? "text-ink" : undefined}
              >
                {part}
              </span>
            </li>
          );
        })}
      </ol>
    </nav>
  );
}
