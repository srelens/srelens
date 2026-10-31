import { describe, expect, it } from "vitest";
import { serializeAuditExport } from "./auditExport";
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
});

