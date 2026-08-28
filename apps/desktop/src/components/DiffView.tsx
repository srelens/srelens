import React from "react";
import type { DiffDoc } from "@srelens/core";

/**
 * Side-by-side render of one document's dry-run diff: current (live) lines on
 * the left, proposed on the right, with insert/delete/replace rows highlighted
 * and unchanged rows dimmed. Pure — driven entirely by `doc.rows`.
 */
export function DiffView({ doc }: { doc: DiffDoc }) {
  const header = (
    <div className="fl-diff__title">
      <span className="font-medium">
        {doc.kind}/{doc.name}
      </span>
      {doc.namespace && <span className="text-xs text-muted-foreground"> · {doc.namespace}</span>}
      {!doc.exists && <span className="fl-diff__badge fl-diff__badge--new">New resource</span>}
    </div>
  );

  if (doc.exists && !doc.changed) {
    return (
      <div className="fl-diff">
        {header}
        <p className="fl-diff__empty">No changes</p>
      </div>
    );
  }

  return (
    <div className="fl-diff">
      {header}
      <table className="fl-diff__table">
        <tbody>
          {doc.rows.map((row, i) => (
            <tr key={i} className={`fl-diff__row fl-diff__row--${row.tag}`}>
              <td className="fl-diff__cell fl-diff__cell--left">{row.left ?? ""}</td>
              <td className="fl-diff__cell fl-diff__cell--right">{row.right ?? ""}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
