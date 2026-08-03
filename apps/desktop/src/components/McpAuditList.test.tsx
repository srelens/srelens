import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

const auditTail = vi.fn();
vi.mock("../lib/mcpSecurity", () => ({ auditTail: (...a: unknown[]) => auditTail(...a) }));

import { McpAuditList } from "./McpAuditList";

describe("McpAuditList", () => {
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
});
