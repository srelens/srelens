import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import React from "react";

vi.mock("../lib/access", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../lib/access")>();
  return { ...actual, useAccess: vi.fn() };
});
import { useAccess } from "../lib/access";

import { NodeCordonAction } from "./NodeCordonAction";

beforeEach(() => {
  // Default: everything allowed, so pre-existing behavioural tests (written
  // before RBAC gating existed) keep exercising enabled controls.
  vi.mocked(useAccess).mockReturnValue({
    allowed: () => true,
    reason: () => "",
    known: () => true,
    loading: false,
  });
  // jsdom has no clipboard API; stub it fresh per test (issue #158).
  Object.defineProperty(navigator, "clipboard", {
    value: { writeText: vi.fn().mockResolvedValue(undefined) },
    configurable: true,
  });
});

describe("NodeCordonAction", () => {
  it("offers Cordon for a schedulable node and applies it", async () => {
    const getObjectFn = vi.fn().mockResolvedValue({ object: { spec: {} } });
    const cordonFn = vi.fn().mockResolvedValue({ ok: true });
    render(<NodeCordonAction context="kind-dev" name="node-a" getObjectFn={getObjectFn} cordonFn={cordonFn} />);

    await waitFor(() => expect(screen.getByRole("button", { name: "Cordon" })).toBeDefined());
    fireEvent.click(screen.getByRole("button", { name: "Cordon" }));
    // confirm dialog → the dialog's Cordon button is the only reachable one
    expect(screen.getByRole("dialog")).toBeDefined();
    fireEvent.click(screen.getByRole("button", { name: "Cordon" }));
    await waitFor(() => expect(cordonFn).toHaveBeenCalledWith("kind-dev", "node-a", true));
    // label flips to Uncordon
    await waitFor(() => expect(screen.getByRole("button", { name: "Uncordon" })).toBeDefined());
  });

  it("offers Uncordon for a cordoned node", async () => {
    const getObjectFn = vi.fn().mockResolvedValue({ object: { spec: { unschedulable: true } } });
    const cordonFn = vi.fn().mockResolvedValue({ ok: true });
    render(<NodeCordonAction context="kind-dev" name="node-a" getObjectFn={getObjectFn} cordonFn={cordonFn} />);
    await waitFor(() => expect(screen.getByRole("button", { name: "Uncordon" })).toBeDefined());
    fireEvent.click(screen.getByRole("button", { name: "Uncordon" }));
    fireEvent.click(screen.getByRole("button", { name: "Uncordon" }));
    await waitFor(() => expect(cordonFn).toHaveBeenCalledWith("kind-dev", "node-a", false));
  });

  it("drains a node behind a confirm", async () => {
    const getObjectFn = vi.fn().mockResolvedValue({ object: { spec: {} } });
    const drainFn = vi.fn().mockResolvedValue({ evicted: 3, skipped: 1 });
    render(<NodeCordonAction context="kind-dev" name="node-a" getObjectFn={getObjectFn} drainFn={drainFn} />);
    await waitFor(() => expect(screen.getByRole("button", { name: "Drain" })).toBeDefined());
    fireEvent.click(screen.getByRole("button", { name: "Drain" }));
    // dialog open → the dialog's Drain button is the only reachable one
    fireEvent.click(screen.getByRole("button", { name: "Drain" }));
    await waitFor(() => expect(drainFn).toHaveBeenCalledWith("kind-dev", "node-a"));
  });

  it("shows the kubectl equivalent in the cordon confirm dialog and copies it", async () => {
    const getObjectFn = vi.fn().mockResolvedValue({ object: { spec: {} } });
    render(<NodeCordonAction context="kind-dev" name="node-a" getObjectFn={getObjectFn} />);
    fireEvent.click(await screen.findByRole("button", { name: "Cordon" }));
    expect(screen.getByText("kubectl cordon node-a --context kind-dev")).toBeDefined();
    fireEvent.click(screen.getByRole("button", { name: "Copy kubectl command" }));
    expect(navigator.clipboard.writeText).toHaveBeenCalledWith("kubectl cordon node-a --context kind-dev");
  });

  it("shows the kubectl equivalent in the uncordon confirm dialog", async () => {
    const getObjectFn = vi.fn().mockResolvedValue({ object: { spec: { unschedulable: true } } });
    render(<NodeCordonAction context="kind-dev" name="node-a" getObjectFn={getObjectFn} />);
    fireEvent.click(await screen.findByRole("button", { name: "Uncordon" }));
    expect(screen.getByText("kubectl uncordon node-a --context kind-dev")).toBeDefined();
  });

  it("shows the kubectl equivalent in the drain confirm dialog", async () => {
    const getObjectFn = vi.fn().mockResolvedValue({ object: { spec: {} } });
    render(<NodeCordonAction context="kind-dev" name="node-a" getObjectFn={getObjectFn} />);
    fireEvent.click(await screen.findByRole("button", { name: "Drain" }));
    expect(
      screen.getByText(
        "kubectl drain node-a --ignore-daemonsets --delete-emptydir-data --force --context kind-dev",
      ),
    ).toBeDefined();
  });

  it("does not throw when the clipboard write fails from the drain dialog preview", async () => {
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText: vi.fn().mockRejectedValue(new Error("denied")) },
      configurable: true,
    });
    const getObjectFn = vi.fn().mockResolvedValue({ object: { spec: {} } });
    render(<NodeCordonAction context="kind-dev" name="node-a" getObjectFn={getObjectFn} />);
    fireEvent.click(await screen.findByRole("button", { name: "Drain" }));
    fireEvent.click(screen.getByRole("button", { name: "Copy kubectl command" }));
    await waitFor(() => expect(navigator.clipboard.writeText).toHaveBeenCalled());
  });
});

describe("NodeCordonAction RBAC gating", () => {
  it("disables Cordon and explains why when the user can't patch nodes", async () => {
    vi.mocked(useAccess).mockReturnValue({
      allowed: () => false,
      reason: () => "RBAC: no rule",
      known: () => true,
      loading: false,
    });
    const getObjectFn = vi.fn().mockResolvedValue({ object: { spec: {} } });
    render(<NodeCordonAction context="kind-dev" name="node-a" getObjectFn={getObjectFn} />);
    const cordon = await screen.findByRole("button", { name: "Cordon" });
    expect((cordon as HTMLButtonElement).disabled).toBe(true);
    expect(cordon.getAttribute("title")).toEqual(expect.stringContaining("patch nodes"));
  });

  it("enables Cordon when allowed", async () => {
    vi.mocked(useAccess).mockReturnValue({
      allowed: () => true,
      reason: () => "",
      known: () => true,
      loading: false,
    });
    const getObjectFn = vi.fn().mockResolvedValue({ object: { spec: {} } });
    render(<NodeCordonAction context="kind-dev" name="node-a" getObjectFn={getObjectFn} />);
    const cordon = await screen.findByRole("button", { name: "Cordon" });
    expect((cordon as HTMLButtonElement).disabled).toBe(false);
  });

  it("opens a node shell: creates a debug pod and opens a terminal that deletes it on close", async () => {
    const getObjectFn = vi.fn().mockResolvedValue({ object: { spec: {} } });
    const createNodeDebugPodFn = vi.fn().mockResolvedValue({ namespace: "default", pod: "srelens-node-debug-x1" });
    const onOpenShell = vi.fn();
    render(
      <NodeCordonAction
        context="kind-dev"
        name="node-a"
        getObjectFn={getObjectFn}
        createNodeDebugPodFn={createNodeDebugPodFn}
        onOpenShell={onOpenShell}
      />,
    );
    fireEvent.click(await screen.findByRole("button", { name: "Node shell" }));
    fireEvent.click(screen.getByRole("button", { name: "Open shell" }));
    await waitFor(() => expect(createNodeDebugPodFn).toHaveBeenCalledWith("kind-dev", "node-a", null, null));
    await waitFor(() =>
      expect(onOpenShell).toHaveBeenCalledWith({
        context: "kind-dev",
        namespace: "default",
        pod: "srelens-node-debug-x1",
        container: "debug",
        execCommand: expect.arrayContaining(["nsenter", "/bin/sh"]),
        deleteOnClose: { context: "kind-dev", namespace: "default", pod: "srelens-node-debug-x1" },
      }),
    );
  });

  it("hides Node shell when no opener is provided", async () => {
    const getObjectFn = vi.fn().mockResolvedValue({ object: { spec: {} } });
    render(<NodeCordonAction context="kind-dev" name="node-a" getObjectFn={getObjectFn} />);
    await screen.findByRole("button", { name: "Cordon" });
    expect(screen.queryByRole("button", { name: "Node shell" })).toBeNull();
  });

});
