import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, within } from "@testing-library/react";

const core = vi.hoisted(() => ({
  auditTail: vi.fn(),
}));
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  ...core,
}));

import { AuditPane, LIMIT } from "./AuditPane";
import type { AuditEntry } from "@srelens/core";

/**
 * `auditTail`'s own doc comment (`packages/core/src/lib/mcpSecurity.ts`): newest
 * first. Two rows are enough to exercise every column without this file turning
 * into a second copy of the audit log.
 */
const DENIED: AuditEntry = {
  ts: 1_700_000_100,
  transport: "http",
  tool: "secret.read",
  args: { context: "prod-eu", namespace: "checkout", name: "checkout-db" },
  decision: "denied",
  outcome: "error",
  err: "sensitive reads are off for this session",
};

const ALLOWED: AuditEntry = {
  ts: 1_700_000_000,
  transport: "stdio",
  tool: "resource.list",
  args: { context: "prod-eu", namespace: "payments" },
  decision: "auto",
  outcome: "ok",
  err: null,
};

describe("AuditPane", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    core.auditTail.mockResolvedValue([DENIED, ALLOWED]);
  });

  it("says how much of the trail it is showing", async () => {
    render(<AuditPane />);
    expect(await screen.findByText(new RegExp(String(LIMIT)))).toBeTruthy();
    expect(core.auditTail).toHaveBeenCalledWith(LIMIT);
  });

  it("draws each call with its verdict, taken from the entry itself", async () => {
    render(<AuditPane />);
    expect(await screen.findByText("secret.read")).toBeTruthy();
    expect(screen.getByText(/denied · sensitive reads are off/i)).toBeTruthy();
    // An allowed row carries no reason, so the word stands alone.
    expect(screen.getByText("allowed")).toBeTruthy();
  });

  it("caps and truncates the target, with the full value in a title", async () => {
    render(<AuditPane />);
    const targets = await screen.findAllByTestId("audit-target");
    const deniedTarget = targets.find((el) => el.title === "prod-eu/checkout/checkout-db");
    expect(deniedTarget).toBeTruthy();
    expect(deniedTarget?.className).toContain("truncate");
    expect(deniedTarget?.className).toMatch(/max-w-\[\d+px\]/);
  });

  it("treats an empty trail as ordinary, not as a failure", async () => {
    core.auditTail.mockResolvedValue([]);
    render(<AuditPane />);
    expect(await screen.findByText(/no capability calls/i)).toBeTruthy();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("says Export is not offered here, once", async () => {
    render(<AuditPane />);
    await screen.findByText("secret.read");
    expect(screen.queryByRole("button", { name: /export/i })).toBeNull();
    expect(screen.getAllByText(/no way to export/i)).toHaveLength(1);
  });

  it("reports a failed read through describeError, never the backend's raw string", async () => {
    core.auditTail.mockRejectedValue(new Error("mcp_audit_tail failed: request timeout"));
    render(<AuditPane />);
    const alert = await screen.findByRole("alert");
    expect(within(alert).getByText(/could not/i)).toBeTruthy();
    expect(alert.querySelector("[data-slot=detail]")?.textContent).not.toContain(
      "mcp_audit_tail failed",
    );
    expect(alert.querySelector("details")?.textContent).toContain("mcp_audit_tail failed");
  });
});
