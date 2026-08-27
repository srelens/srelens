import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

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

  /**
   * The pair the pane exists to keep apart, asserted as a pair. `auditTail`
   * used to catch every refusal and resolve to `[]`, so an unreadable trail
   * rendered "A fresh install has made none — this is not an error." as fact,
   * with no alert — guaranteed on the web build, where every `invoke` rejects.
   * The reject test above proves the alert appears; this one proves the empty
   * state does NOT, which is the half that was actually broken.
   */
  it("never says the trail is empty when it could not be read", async () => {
    core.auditTail.mockRejectedValue(new Error("no such command"));
    render(<AuditPane />);
    await screen.findByRole("alert");
    expect(screen.queryByText(/no capability calls/i)).toBeNull();
    expect(screen.queryByText(/a fresh install has made none/i)).toBeNull();
  });

  /**
   * Classic's `McpAuditList` wrote the reason down and the new pane lost it:
   * "a list read once on mount quietly goes stale — an operator looking for an
   * agent's action would conclude it never happened."
   */
  it("re-reads the trail when asked, rather than answering from mount", async () => {
    const user = userEvent.setup();
    render(<AuditPane />);
    await screen.findByText("secret.read");
    expect(core.auditTail).toHaveBeenCalledTimes(1);
    core.auditTail.mockResolvedValue([{ ...ALLOWED, tool: "node.cordon" }]);
    await user.click(screen.getByRole("button", { name: /refresh/i }));
    expect(await screen.findByText("node.cordon")).toBeTruthy();
    expect(core.auditTail).toHaveBeenCalledTimes(2);
  });

  it("recovers from a refusal when the trail is re-read", async () => {
    const user = userEvent.setup();
    core.auditTail.mockRejectedValueOnce(new Error("no such command"));
    render(<AuditPane />);
    await screen.findByRole("alert");
    core.auditTail.mockResolvedValue([ALLOWED]);
    await user.click(screen.getByRole("button", { name: /refresh/i }));
    expect(await screen.findByText("resource.list")).toBeTruthy();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  /**
   * The log rotates at 5 MB and spans days, so `14:02:11` alone cannot say
   * which day a call landed on — the one question an operator reading this
   * after an incident actually has.
   */
  it("dates each call, not just the time of day", async () => {
    render(<AuditPane />);
    const cell = await screen.findByTestId("audit-time-1700000100");
    const stamp = new Date(1_700_000_100 * 1000);
    const pad = (n: number) => String(n).padStart(2, "0");
    expect(cell.textContent).toContain(
      `${stamp.getFullYear()}-${pad(stamp.getMonth() + 1)}-${pad(stamp.getDate())}`,
    );
    expect(cell.textContent).toContain(`${pad(stamp.getHours())}:${pad(stamp.getMinutes())}`);
  });

  /**
   * #369: srelens does not track which client connected, and `AuditEntry`
   * carries the transport a call arrived on. A `Client` header over `stdio` /
   * `http` claims exactly what the issue says srelens cannot know.
   */
  it("names the transport column for the value it holds", async () => {
    render(<AuditPane />);
    await screen.findByText("secret.read");
    const headers = screen.getAllByRole("columnheader").map((h) => h.textContent);
    expect(headers).toContain("Transport");
    expect(headers).not.toContain("Client");
  });
});
