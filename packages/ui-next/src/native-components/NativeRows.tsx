import { useState, type ReactNode } from "react";
import { Button } from "@srelens/ui-kit";

/** Only the visible page mounts; the count always names the remaining rows. */
export function NativeRows<T>({ items, children }: { items: T[]; children: (visible: T[]) => ReactNode }) {
  const [expanded, setExpanded] = useState({ items, count: 20 });
  const count = expanded.items === items ? expanded.count : 20;
  const remaining = items.length - count;
  return <>
    {children(items.slice(0, count))}
    {items.length > 20 && <div className="native-component-more">
      {remaining > 0 && <Button type="button" aria-expanded={count > 20} variant="secondary" size="sm" onClick={() => setExpanded({ items, count: count + 20 })}>Show {Math.min(20, remaining)} more ({remaining} remaining)</Button>}
      {count > 20 && <Button type="button" aria-expanded="true" variant="ghost" size="sm" onClick={() => setExpanded({ items, count: 20 })}>Show fewer</Button>}
    </div>}
  </>;
}
