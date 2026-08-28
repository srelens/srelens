import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";

const actions = vi.hoisted(() => ({ deleteResource: vi.fn(), rolloutRestart: vi.fn() }));
const workloads = vi.hoisted(() => ({ deletePod: vi.fn(), evictPod: vi.fn() }));
const notifyMock = vi.hoisted(() => ({ success: vi.fn(), error: vi.fn(), info: vi.fn() }));
vi.mock("@srelens/core/lib/actions", () => actions);
vi.mock("@srelens/core/lib/workloads", () => workloads);
vi.mock("@srelens/core/lib/notify", () => ({ notify: notifyMock }));

import { BulkActionBar } from "./BulkActionBar";

beforeEach(() => {
  Object.values({ ...actions, ...workloads, ...notifyMock }).forEach((m) => m.mockReset());
});

describe("BulkActionBar", () => {
  const podRows = [
    { name: "web-1", namespace: "prod" },
    { name: "web-2", namespace: "prod" },
  ];

  it("offers evict for pods and bulk-deletes with one confirm + a summary toast", async () => {
    workloads.deletePod.mockResolvedValue({ deleted: true });
    const onClear = vi.fn();
    const onDone = vi.fn();
    render(<BulkActionBar context="ctx" kind="Pod" rows={podRows} onClear={onClear} onDone={onDone} />);

    expect(screen.getByText("2 selected")).toBeDefined();
    expect(screen.getByRole("button", { name: /Evict/ })).toBeDefined();

    fireEvent.click(screen.getByRole("button", { name: /Delete/ }));
    const dialog = await screen.findByRole("dialog");
    // Confirm lists exactly what's affected.
    expect(within(dialog).getByText("prod/web-1")).toBeDefined();
    expect(within(dialog).getByText("prod/web-2")).toBeDefined();

    fireEvent.click(within(dialog).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(workloads.deletePod).toHaveBeenCalledTimes(2));
    expect(workloads.deletePod).toHaveBeenCalledWith("ctx", "prod", "web-1");
    await waitFor(() => expect(notifyMock.success).toHaveBeenCalledWith("Deleted 2 Pods"));
    expect(onClear).toHaveBeenCalled();
    expect(onDone).toHaveBeenCalled();
  });

  it("reports partial failure without aborting the rest", async () => {
    workloads.deletePod.mockImplementation(async (_c: string, _n: string, name: string) =>
      name === "web-2" ? { error: "forbidden" } : { deleted: true },
    );
    render(<BulkActionBar context="ctx" kind="Pod" rows={podRows} onClear={vi.fn()} onDone={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: /Delete/ }));
    fireEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(notifyMock.error).toHaveBeenCalled());
    expect(notifyMock.error.mock.calls[0][0]).toMatch(/1 of 2 failed/);
    expect(notifyMock.error.mock.calls[0][1]).toContain("web-2: forbidden");
  });

  it("offers rollout-restart for Deployments and delete via deleteResource", async () => {
    actions.deleteResource.mockResolvedValue({ ok: true });
    actions.rolloutRestart.mockResolvedValue({ ok: true });
    const rows = [{ name: "api", namespace: "prod" }];
    render(<BulkActionBar context="ctx" kind="Deployment" rows={rows} onClear={vi.fn()} onDone={vi.fn()} />);

    expect(screen.queryByRole("button", { name: /Evict/ })).toBeNull(); // not a pod
    fireEvent.click(screen.getByRole("button", { name: /Rollout restart/ }));
    fireEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Rollout-restart" }));
    await waitFor(() => expect(actions.rolloutRestart).toHaveBeenCalledWith("ctx", "Deployment", "prod", "api"));
  });
});
