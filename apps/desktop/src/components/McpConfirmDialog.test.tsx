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

import { McpConfirmDialog } from "./McpConfirmDialog";

describe("McpConfirmDialog", () => {
  beforeEach(() => respondToConfirm.mockReset());

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
});
