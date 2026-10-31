import { invokeCommand } from "../transport/transport";
import type { AuditEntry } from "./mcpSecurity";

export type AuditExportScope = "all" | "current";
export type AuditExportFormat = "jsonl" | "csv";
export interface RetainedAudit {
  entries: AuditEntry[];
  skippedLines: number;
}

/** Desktop-only: the audit trail belongs to this machine, never the web host. */
export function retainedAudit(): Promise<RetainedAudit> {
  return invokeCommand<RetainedAudit>("mcp_audit_retained");
}

/** Uses a native save dialog and an owner-only, atomic file write. Null is cancellation. */
export function saveAuditExport(content: string, format: AuditExportFormat): Promise<string | null> {
  return invokeCommand<string | null>("save_audit_export", { content, format });
}

/** JSONL starts with a metadata record; CSV repeats the metadata in each row. */
export function serializeAuditExport(
  entries: AuditEntry[], scope: AuditExportScope, format: AuditExportFormat,
  skippedLines: number, detailsIncluded: boolean,
): string {
  let firstTs: number | null = null;
  let lastTs: number | null = null;
  for (const entry of entries) {
    firstTs = firstTs === null ? entry.ts : Math.min(firstTs, entry.ts);
    lastTs = lastTs === null ? entry.ts : Math.max(lastTs, entry.ts);
  }
  const metadata = { type: "audit-export", version: 1, scope, count: entries.length, firstTs, lastTs, skippedLines, detailsIncluded };
  const rows = entries.map(({ args, err, ...entry }) => detailsIncluded ? { ...entry, args, err } : entry);
  if (format === "jsonl") return [metadata, ...rows].map(row => JSON.stringify(row)).join("\n") + "\n";

  const keys = ["ts", "source", "transport", "tool", "app", "cluster", "resource", "decision", "outcome", "resultBytes", ...(detailsIncluded ? ["args", "err"] : [])];
  const metaKeys = ["scope", "count", "firstTs", "lastTs", "skippedLines", "detailsIncluded"] as const;
  // Quoting is not sufficient to stop a spreadsheet evaluating a formula.
  const cell = (value: unknown) => {
    let text = value == null ? "" : typeof value === "object" ? JSON.stringify(value) : String(value);
    if (/^[\s]*[=+\-@]/.test(text)) text = "'" + text;
    return '"' + text.replaceAll('"', '""') + '"';
  };
  const record = (row: Record<string, unknown>) => [...metaKeys.map(key => metadata[key]), ...keys.map(key => row[key])].map(cell).join(",");
  // An empty export still carries its scope and count, with blank record cells.
  return [[...metaKeys, ...keys].join(","), ...(rows.length ? rows : [{}]).map(record)].join("\r\n") + "\r\n";
}
