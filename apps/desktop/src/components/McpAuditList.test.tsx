import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const auditTail = vi.fn();
vi.mock("@srelens/core/lib/mcpSecurity", () => ({ auditTail: (...a: unknown[]) => auditTail(...a) }));

import { McpAuditList } from "./McpAuditList";

describe("McpAuditList", () => {
  // Without this the spy accumulates across cases, so any test asserting a
  // call count measures the whole file rather than its own component.
  beforeEach(() => {
    auditTail.mockReset();
  });

  it("renders entries newest-first with their decision", async () => {
    auditTail.mockResolvedValue([
      { ts: 1780000000, transport: "http", tool: "k8s_deletePod", args: { name: "web" }, decision: "approved", outcome: "ok" },
      { ts: 1779999999, transport: "stdio", tool: "k8s_listPods", args: {}, decision: "auto", outcome: "ok" },
    ]);
    render(<McpAuditList />);
    expect(await screen.findByText(/k8s_deletePod/)).toBeTruthy();
    expect(screen.getByText(/approved/i)).toBeTruthy();
  });

  it("shows an empty state rather than a blank panel", async () => {
    auditTail.mockResolvedValue([]);
    render(<McpAuditList />);
    expect(await screen.findByText(/no agent activity/i)).toBeTruthy();
  });

  /**
   * `auditTail` used to swallow every refusal and resolve to `[]`, so this
   * panel could only ever say "no agent activity yet" — including when the
   * trail could not be read at all. It rejects now, and a refusal must not
   * come out looking like a quiet cluster.
   */
  it("says the trail could not be read, instead of reporting no activity", async () => {
    auditTail.mockRejectedValue(new Error("mcp_audit_tail failed: request timeout"));
    render(<McpAuditList />);
    const alert = await screen.findByRole("alert");
    expect(alert.textContent ?? "").toMatch(/could not be read/i);
    expect(screen.queryByText(/no agent activity/i)).toBeNull();
  });

  it("retries the read after a refusal, rather than staying failed", async () => {
    auditTail.mockRejectedValueOnce(new Error("nope"));
    render(<McpAuditList />);
    await screen.findByRole("alert");
    auditTail.mockResolvedValue([
      { ts: 1780000002, transport: "http", tool: "k8s_scale", args: {}, decision: "approved", outcome: "ok", err: null },
    ]);
    fireEvent.click(screen.getByLabelText(/refresh agent activity/i));
    expect(await screen.findByText(/k8s_scale/)).toBeTruthy();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  /// Settings stays open while agents keep calling, so a list fetched once on
  /// mount silently goes stale — an operator watching for an agent's action
  /// sees nothing and concludes it never happened.
  it("re-reads the log when refreshed", async () => {
    auditTail.mockResolvedValue([]);
    render(<McpAuditList />);
    expect(await screen.findByText(/no agent activity/i)).toBeTruthy();
    expect(auditTail).toHaveBeenCalledTimes(1);

    auditTail.mockResolvedValue([
      {
        ts: 1780000001,
        transport: "http",
        tool: "k8s_drainNode",
        args: { name: "node-1" },
        decision: "denied",
        outcome: "error",
        err: "user declined",
      },
    ]);
    fireEvent.click(screen.getByLabelText(/refresh agent activity/i));

    expect(await screen.findByText(/k8s_drainNode/)).toBeTruthy();
    expect(auditTail).toHaveBeenCalledTimes(2);
  });
});
