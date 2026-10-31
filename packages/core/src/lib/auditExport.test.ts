import { describe, expect, it, vi } from "vitest";
const invoke = vi.hoisted(() => vi.fn());
vi.mock("../transport/transport", () => ({ invokeCommand: invoke }));
import { retainedAudit, saveAuditExport, serializeAuditExport } from "./auditExport";
import type { AuditEntry } from "./mcpSecurity";

const entry: AuditEntry = {
  ts: 1700000000, source: "mcp", transport: "http", tool: "=untrusted,\"tool\"\nnext",
  args: { token: "private" }, app: null, cluster: "prod", resource: "web",
  decision: "approved", outcome: "failed", err: "private failure", resultBytes: 42,
};

describe("audit export", () => {
  it("writes metadata and exact verdicts, omitting arguments and errors by default", () => {
    const text = serializeAuditExport([entry], "all", "jsonl", 2, false);
    const [meta, row] = text.trim().split("\n").map(line => JSON.parse(line));
    expect(meta).toMatchObject({ type: "audit-export", scope: "all", count: 1, firstTs: entry.ts, lastTs: entry.ts, skippedLines: 2, detailsIncluded: false });
    expect(row).toMatchObject({ decision: "approved", outcome: "failed", resultBytes: 42 });
    expect(row).not.toHaveProperty("args");
    expect(row).not.toHaveProperty("err");
    expect(text).not.toContain("private");
    expect(text.endsWith("\n")).toBe(true);
  });

  it("includes details only after explicit opt-in, without changing input", () => {
    const text = serializeAuditExport([entry], "current", "jsonl", 0, true);
    expect(JSON.parse(text.trim().split("\n")[1])).toEqual(entry);
    expect(entry.args.token).toBe("private");
  });

  it("escapes CSV cells and neutralizes spreadsheet formulas", () => {
    const csv = serializeAuditExport([entry], "current", "csv", 0, true);
    expect(csv).toContain('"\'=untrusted,""tool""\nnext"');
    expect(csv).toContain('"approved","failed"');
    expect(csv).toContain('"{""token"":""private""}"');
  });

  it("describes an empty trail without inventing timestamps", () => {
    const text = serializeAuditExport([], "all", "jsonl", 0, false);
    expect(JSON.parse(text)).toMatchObject({ count: 0, firstTs: null, lastTs: null });
  });

  it("uses the desktop command contract and preserves cancellation and failures", async () => {
    invoke.mockResolvedValueOnce({ entries: [entry], skippedLines: 0 });
    await expect(retainedAudit()).resolves.toMatchObject({ entries: [entry] });
    expect(invoke).toHaveBeenLastCalledWith("mcp_audit_retained");
    invoke.mockResolvedValueOnce(null);
    await expect(saveAuditExport("contents", "csv")).resolves.toBeNull();
    expect(invoke).toHaveBeenLastCalledWith("save_audit_export", { content: "contents", format: "csv" });
    invoke.mockRejectedValueOnce(new Error("unreadable"));
    await expect(retainedAudit()).rejects.toThrow("unreadable");
  });

  it("keeps an empty CSV export's metadata and computes bounds independently of row order", () => {
    expect(serializeAuditExport([], "current", "csv", 0, false)).toContain('"current","0","","","0","false"');
    const meta = JSON.parse(serializeAuditExport([{ ...entry, ts: 30 }, { ...entry, ts: 10 }], "current", "jsonl", 0, false).split("\n")[0]);
    expect(meta).toMatchObject({ firstTs: 10, lastTs: 30 });
  });
});
