import { cx } from "./cx";

export interface PairListProps {
  pairs: Array<[key: string, value: string]>;
  /** Let a long value wrap over several lines instead of truncating it. */
  breakValues?: boolean;
  className?: string;
}

/**
 * Labels and annotations, printed as `key=value` the way kubectl prints them —
 * the form anyone who has run `describe` already reads without thinking.
 *
 * A `ul` of `li`, not a stack of divs: this is a set, and how many are in it is
 * part of reading it — an annotation block is scanned for what is there as much
 * as for what it says. Each row truncates by default, because the block is
 * scanned by key and a value wrapping over four lines buries the next one;
 * `breakValues` is for the pane wide enough to show one in full, and it is the
 * only way to read a long value here. An empty set renders nothing rather than
 * an empty `.pairs`, whose line height would leave a gap between the two blocks
 * it sits between. (#320)
 *
 * Every row used to carry `title={`${k}=${v}`}`, on the reasoning that the
 * truncated row is the one that most needs reading in full. That was a hole. A
 * `kubectl apply`-managed Secret keeps its entire base64 `data` map inside the
 * `kubectl.kubernetes.io/last-applied-configuration` annotation, and an
 * annotation map arrives here as pairs — so a value the reader had been shown
 * three characters of was sitting whole in the DOM, past every redaction above
 * it. The detail pane grew a toggle to hide annotations behind, which was a
 * screen working around a defect in the kit.
 *
 * There is no prop to put it back. Truncation is a visual affordance, and the
 * moment it is also a disclosure boundary the only safe number of ways to
 * defeat it is none — an opt-in is a flag someone eventually passes on a
 * Secret. (#331)
 */
export function PairList({ pairs, breakValues, className }: PairListProps) {
  if (pairs.length === 0) return null;
  return (
    <ul className={cx("pairs", className)}>
      {pairs.map(([k, v]) => (
        <li key={k} className={breakValues ? undefined : "truncate"}>
          <span className="k">{k}=</span>
          <span className={cx("v", breakValues && "break-all")}>{v}</span>
        </li>
      ))}
    </ul>
  );
}
