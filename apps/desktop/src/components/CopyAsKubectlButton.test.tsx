import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import React from "react";

const { notifyMock } = vi.hoisted(() => ({
  notifyMock: { success: vi.fn(), error: vi.fn(), info: vi.fn() },
}));
vi.mock("../lib/notify", () => ({ notify: notifyMock }));

import { CopyAsKubectlButton } from "./CopyAsKubectlButton";

beforeEach(() => {
  notifyMock.success.mockReset();
  Object.defineProperty(navigator, "clipboard", {
    value: { writeText: vi.fn().mockResolvedValue(undefined) },
    configurable: true,
  });
});

describe("CopyAsKubectlButton", () => {
  it("copies the get command (as yaml) from the menu", async () => {
    render(<CopyAsKubectlButton kind="Pod" name="web-1" namespace="default" context="kind-dev" />);
    fireEvent.click(screen.getByRole("button", { name: "Copy as kubectl" }));
    fireEvent.click(await screen.findByRole("button", { name: "Copy get" }));
    expect(navigator.clipboard.writeText).toHaveBeenCalledWith(
      "kubectl get pods web-1 -n default --context kind-dev -o yaml",
    );
    await waitFor(() => expect(notifyMock.success).toHaveBeenCalledWith("Copied kubectl command"));
  });

  it("copies the describe command from the menu", async () => {
    render(<CopyAsKubectlButton kind="Pod" name="web-1" namespace="default" context="kind-dev" />);
    fireEvent.click(screen.getByRole("button", { name: "Copy as kubectl" }));
    fireEvent.click(await screen.findByRole("button", { name: "Copy describe" }));
    expect(navigator.clipboard.writeText).toHaveBeenCalledWith(
      "kubectl describe pods web-1 -n default --context kind-dev",
    );
    await waitFor(() => expect(notifyMock.success).toHaveBeenCalledWith("Copied kubectl command"));
  });

  it("closes the menu after copying", async () => {
    render(<CopyAsKubectlButton kind="Node" name="node-1" context="kind-dev" />);
    fireEvent.click(screen.getByRole("button", { name: "Copy as kubectl" }));
    fireEvent.click(await screen.findByRole("button", { name: "Copy get" }));
    await waitFor(() => expect(screen.queryByRole("button", { name: "Copy get" })).toBeNull());
  });
});
