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

  /**
   * The two outcomes #555 split `error` into, asserted on the badge and not
   * only on the word. "srelens would not do this" and "the cluster would not"
   * are different answers to "did it happen?", and a wrong variant mapping —
   * a failure drawn in the same amber as a refused argument — would keep every
   * other case in this file green.
   */
  it("colours a refused call and a broken one differently", async () => {
    auditTail.mockResolvedValue([
      { ts: 1780000010, transport: "stdio", tool: "k8s_deletePod", args: {}, decision: "approved", outcome: "failed", err: "the apiserver closed the connection" },
      { ts: 1780000009, transport: "http", tool: "k8s_scale", args: {}, decision: "auto", outcome: "rejected", err: "a replica count is required" },
    ]);
    render(<McpAuditList />);

    const failed = await screen.findByText("failed");
    const rejected = screen.getByText("rejected");
    // The variant reaches the DOM as the shadcn badge's own classes
    // (`apps/desktop/src/components/ui/badge.tsx`): `destructive` for danger,
    // amber for warning. Asserting the mapping, not the palette.
    expect(failed.className).toContain("destructive");
    expect(rejected.className).toContain("amber");
  });

  it("shows an empty state rather than a blank panel", async () => {
    auditTail.mockResolvedValue([]);
    render(<McpAuditList />);
    expect(await screen.findByText(/no capability activity/i)).toBeTruthy();
  });

  /**
   * `auditTail` used to swallow every refusal and resolve to `[]`, so this
   * panel could only ever say it had no activity — including when the
   * trail could not be read at all. It rejects now, and a refusal must not
   * come out looking like a quiet cluster.
   */
  it("says the trail could not be read, instead of reporting no activity", async () => {
    auditTail.mockRejectedValue(new Error("mcp_audit_tail failed: request timeout"));
    render(<McpAuditList />);
    const alert = await screen.findByRole("alert");
    expect(alert.textContent ?? "").toMatch(/could not be read/i);
    expect(screen.queryByText(/no capability activity/i)).toBeNull();
  });

  it("retries the read after a refusal, rather than staying failed", async () => {
    auditTail.mockRejectedValueOnce(new Error("nope"));
    render(<McpAuditList />);
    await screen.findByRole("alert");
    auditTail.mockResolvedValue([
      { ts: 1780000002, transport: "http", tool: "k8s_scale", args: {}, decision: "approved", outcome: "ok", err: null },
    ]);
    fireEvent.click(screen.getByLabelText(/refresh capability activity/i));
    expect(await screen.findByText(/k8s_scale/)).toBeTruthy();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  /// Settings stays open while agents keep calling, so a list fetched once on
  /// mount silently goes stale — an operator watching for an agent's action
  /// sees nothing and concludes it never happened.
  it("re-reads the log when refreshed", async () => {
    auditTail.mockResolvedValue([]);
    render(<McpAuditList />);
    expect(await screen.findByText(/no capability activity/i)).toBeTruthy();
    expect(auditTail).toHaveBeenCalledTimes(1);

    auditTail.mockResolvedValue([
      {
        ts: 1780000001,
        transport: "http",
        tool: "k8s_drainNode",
        args: { name: "node-1" },
        decision: "denied",
        outcome: "rejected",
        err: "user declined",
      },
    ]);
    fireEvent.click(screen.getByLabelText(/refresh capability activity/i));

    expect(await screen.findByText(/k8s_drainNode/)).toBeTruthy();
    expect(auditTail).toHaveBeenCalledTimes(2);
  });
});
