import React from "react";

export interface KubectlPreviewProps {
  command: string;
}

/** Small monospace preview of a kubectl-equivalent command, shown inside confirm dialogs. */
export function KubectlPreview({ command }: KubectlPreviewProps) {
  return (
    <p className="text-muted-foreground text-xs" style={{ marginTop: 8 }}>
      <code>{command}</code>
    </p>
  );
}
