import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi, beforeEach } from "vitest";

const respondToConfirm = vi.fn();
let emit: (payload: unknown) => void = () => {};

vi.mock("../lib/mcpSecurity", () => ({
  respondToConfirm: (...a: unknown[]) => respondToConfirm(...a),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (_name: string, cb: (e: { payload: unknown }) => void) => {
    emit = (payload) => cb({ payload });
    return Promise.resolve(() => {});
  },
}));
const { notify } = vi.hoisted(() => ({
  notify: { success: vi.fn(), error: vi.fn(), info: vi.fn() },
}));
vi.mock("../lib/notify", () => ({ notify }));

import { McpConfirmDialog } from "./McpConfirmDialog";

describe("McpConfirmDialog", () => {
  beforeEach(() => {
    respondToConfirm.mockReset();
    notify.success.mockReset();
    notify.error.mockReset();
    notify.info.mockReset();
  });

  it("renders nothing until a request arrives", () => {
    const { container } = render(<McpConfirmDialog />);
    expect(container.textContent).toBe("");
  });

  it("shows the tool and arguments, and approves", async () => {
    render(<McpConfirmDialog />);
    emit({ id: "r1", tool: "k8s_deletePod", args: { name: "web-1", namespace: "prod" } });
    await screen.findByText(/k8s_deletePod/);
    expect(screen.getByText(/web-1/)).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: /approve/i }));
    await waitFor(() => expect(respondToConfirm).toHaveBeenCalledWith("r1", true));
  });

  it("denies on the deny button", async () => {
    render(<McpConfirmDialog />);
    emit({ id: "r2", tool: "k8s_scale", args: {} });
    await screen.findByText(/k8s_scale/);
    await userEvent.click(screen.getByRole("button", { name: /deny/i }));
    await waitFor(() => expect(respondToConfirm).toHaveBeenCalledWith("r2", false));
  });

  it("queues a second request rather than dropping it", async () => {
    render(<McpConfirmDialog />);
    emit({ id: "a", tool: "toolA", args: {} });
    emit({ id: "b", tool: "toolB", args: {} });
    await screen.findByText(/toolA/);
    await userEvent.click(screen.getByRole("button", { name: /deny/i }));
    await screen.findByText(/toolB/);
  });

  it("surfaces an error instead of silently swallowing a failed response", async () => {
    respondToConfirm.mockRejectedValue(new Error("already timed out"));
    render(<McpConfirmDialog />);
    emit({ id: "r3", tool: "k8s_deletePod", args: {} });
    await screen.findByText(/k8s_deletePod/);
    await userEvent.click(screen.getByRole("button", { name: /approve/i }));
    await waitFor(() => expect(notify.error).toHaveBeenCalled());
    // The user must not be left believing the call was actioned: the
    // request is still dropped from the queue (nothing left to retry), but
    // the failure is surfaced rather than silent.
    expect(screen.queryByText(/k8s_deletePod/)).toBeNull();
  });
});
