import { useRef, useState } from "react";
import { retainedAudit, saveAuditExport, serializeAuditExport, type AuditEntry, type AuditExportFormat, type AuditExportScope } from "@srelens/core";
import { Button, Select } from "@srelens/ui-kit";
import { FailureAlert } from "../../lib/errorCopy";

/** Export is local and deliberate; details stay out unless the reader opts in. */
export function AuditExport({ entries, disabled }: { entries: AuditEntry[]; disabled: boolean }) {
  const [scope, setScope] = useState<AuditExportScope>("all");
  const [format, setFormat] = useState<AuditExportFormat>("jsonl");
  const [details, setDetails] = useState(false);
  const [busy, setBusy] = useState(false);
  const pending = useRef(false);
  const [error, setError] = useState<unknown>(null);
  const [saved, setSaved] = useState<string | null>(null);

  async function exportTrail() {
    if (pending.current) return;
    pending.current = true;
    setBusy(true);
    setError(null);
    setSaved(null);
    try {
      const snapshot = scope === "all" ? await retainedAudit() : { entries, skippedLines: 0 };
      const content = serializeAuditExport(snapshot.entries, scope, format, snapshot.skippedLines, details);
      const path = await saveAuditExport(content, format);
      if (path !== null) setSaved(`Saved ${snapshot.entries.length} entries to ${path}.${snapshot.skippedLines ? ` Skipped ${snapshot.skippedLines} unreadable line(s).` : ""}`);
    } catch (e) {
      setError(e);
    } finally {
      pending.current = false;
      setBusy(false);
    }
  }

  return (
    <div className="mt-3 border-t border-rule pt-3 text-[0.75rem]">
      <div className="flex flex-wrap items-center gap-2">
        <Select aria-label="Export scope" value={scope} disabled={busy}
          onValueChange={value => setScope(value as AuditExportScope)}
          options={[{ value: "all", label: "All retained entries" }, { value: "current", label: "Current view" }]} />
        <Select aria-label="Export format" value={format} disabled={busy}
          onValueChange={value => setFormat(value as AuditExportFormat)}
          options={[{ value: "jsonl", label: "JSONL" }, { value: "csv", label: "CSV" }]} />
        <Button variant="secondary" size="sm" disabled={disabled || busy} onClick={exportTrail}>
          {busy ? "Exporting…" : "Export audit trail"}
        </Button>
      </div>
      <label className="mt-2 flex items-center gap-2">
        <input type="checkbox" checked={details} disabled={busy} onChange={event => setDetails(event.target.checked)} />
        Include arguments and error details (may contain sensitive data)
      </label>
      <p className="mt-2 leading-relaxed text-muted">
        Saves a local file through the desktop save dialog. All retained entries includes the active and rotated log;
        current view saves the loaded rows. Older deleted records cannot be recovered.
        Exports are not tamper-evident. Review the file before sharing it.
      </p>
      {error !== null && <FailureAlert tone="sev" title="The audit trail could not be exported" error={error} />}
      {saved !== null && <p role="status" className="mt-2 break-all">{saved}</p>}
    </div>
  );
}
