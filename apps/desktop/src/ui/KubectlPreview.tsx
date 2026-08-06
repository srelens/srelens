import React from "react";
import { Copy } from "lucide-react";

export interface KubectlPreviewProps {
  /** The kubectl-equivalent command. Omit (and pass `note` instead) when there's no faithful one-liner. */
  command?: string;
  /** Shown instead of a command when no clean kubectl equivalent exists (e.g. evict). */
  note?: string;
  /** Copy-to-clipboard handler; renders a copy affordance next to `command` when set. */
  onCopy?: () => void;
}

/** Small monospace preview of a kubectl-equivalent command, shown inside confirm dialogs. */
export function KubectlPreview({ command, note, onCopy }: KubectlPreviewProps) {
  if (!command) {
    return (
      <p className="text-muted-foreground text-xs" style={{ marginTop: 8 }}>
        {note}
      </p>
    );
  }
  return (
    <p
      className="text-muted-foreground text-xs"
      style={{ marginTop: 8, display: "flex", alignItems: "center", gap: 4 }}
    >
      <span>Equivalent kubectl:</span>
      <code>{command}</code>
      {onCopy && (
        <button
          type="button"
          aria-label="Copy kubectl command"
          title="Copy kubectl command"
          onClick={onCopy}
          style={{
            display: "inline-flex",
            padding: 2,
            background: "none",
            border: "none",
            cursor: "pointer",
            color: "inherit",
          }}
        >
          <Copy size={12} aria-hidden="true" />
        </button>
      )}
    </p>
  );
}
