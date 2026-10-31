import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const core = vi.hoisted(() => ({ retainedAudit: vi.fn(), saveAuditExport: vi.fn() }));
vi.mock("@srelens/core", async orig => ({ ...(await orig<typeof import("@srelens/core")>()), ...core }));
import { AuditExport } from "./AuditExport";
import type { AuditEntry } from "@srelens/core";

const row: AuditEntry = { ts: 100, source: "mcp", transport: "http", tool: "read", args: { token: "secret" }, app: null, cluster: "prod", resource: null, decision: "auto", outcome: "ok", err: null, resultBytes: 5 };

describe("AuditExport", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    core.retainedAudit.mockResolvedValue({ entries: [row, { ...row, ts: 101 }], skippedLines: 1 });
    core.saveAuditExport.mockResolvedValue("/chosen/audit.jsonl");
  });

  it("defaults to all retained entries and JSONL, omitting sensitive details", async () => {
    render(<AuditExport entries={[row]} disabled={false} />);
    await userEvent.click(screen.getByRole("button", { name: "Export audit trail" }));
    await screen.findByRole("status");
    expect(core.retainedAudit).toHaveBeenCalledOnce();
    const [text, format] = core.saveAuditExport.mock.calls[0];
    expect(format).toBe("jsonl");
    expect(JSON.parse(text.split("\n")[0])).toMatchObject({ scope: "all", count: 2, skippedLines: 1 });
    expect(text).not.toContain("secret");
    expect(screen.getByRole("status").textContent).toContain("2 entries");
    expect(screen.getByRole("status").textContent).toContain("1 unreadable line");
  });

  it("exports exactly the displayed snapshot after selecting current view", async () => {
    render(<AuditExport entries={[row]} disabled={false} />);
    await userEvent.selectOptions(screen.getByRole("combobox", { name: "Export scope" }), "current");
    await userEvent.selectOptions(screen.getByRole("combobox", { name: "Export format" }), "csv");
    await userEvent.click(screen.getByRole("checkbox", { name: /include arguments and error details/i }));
    await userEvent.click(screen.getByRole("button", { name: "Export audit trail" }));
    await screen.findByRole("status");
    expect(core.retainedAudit).not.toHaveBeenCalled();
    expect(core.saveAuditExport.mock.calls[0][1]).toBe("csv");
    expect(core.saveAuditExport.mock.calls[0][0]).toContain("secret");
    expect(core.saveAuditExport.mock.calls[0][0]).not.toContain("101");
  });

  it("does not claim success when the native dialog is cancelled", async () => {
    core.saveAuditExport.mockResolvedValue(null);
    render(<AuditExport entries={[]} disabled={false} />);
    await userEvent.click(screen.getByRole("button", { name: "Export audit trail" }));
    await waitFor(() => expect(core.saveAuditExport).toHaveBeenCalled());
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("reports a read failure without opening a save dialog and permits retry", async () => {
    core.retainedAudit.mockRejectedValueOnce(new Error("permission denied"));
    render(<AuditExport entries={[]} disabled={false} />);
    await userEvent.click(screen.getByRole("button", { name: "Export audit trail" }));
    expect((await screen.findByRole("alert")).textContent).toContain("permission denied");
    expect(core.saveAuditExport).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "Export audit trail" }));
    await screen.findByRole("status");
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("disables export while the view is loading or failed", () => {
    render(<AuditExport entries={[]} disabled />);
    expect((screen.getByRole("button", { name: "Export audit trail" }) as HTMLButtonElement).disabled).toBe(true);
  });
});
