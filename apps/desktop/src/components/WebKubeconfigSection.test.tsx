import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";

const webKubeconfigs = vi.hoisted(() => ({
  list: vi.fn(),
  upload: vi.fn(),
  remove: vi.fn(),
}));
vi.mock("../lib/webKubeconfigs", () => webKubeconfigs);

import { WebKubeconfigSection } from "./WebKubeconfigSection";

beforeEach(() => {
  Object.values(webKubeconfigs).forEach((m) => m.mockReset());
  webKubeconfigs.list.mockResolvedValue([
    { id: 1, name: "prod", createdAt: 0, updatedAt: 0 },
    { id: 2, name: "staging", createdAt: 0, updatedAt: 0 },
  ]);
});

describe("WebKubeconfigSection", () => {
  it("lists the caller's uploaded kubeconfigs", async () => {
    render(<WebKubeconfigSection />);
    expect(await screen.findByText("prod")).toBeDefined();
    expect(screen.getByText("staging")).toBeDefined();
    expect(webKubeconfigs.list).toHaveBeenCalledTimes(1);
  });

  it("uploads a named kubeconfig and refreshes the list", async () => {
    webKubeconfigs.upload.mockResolvedValue(3);
    webKubeconfigs.list
      .mockResolvedValueOnce([])
      .mockResolvedValueOnce([{ id: 3, name: "new-cluster", createdAt: 0, updatedAt: 0 }]);
    render(<WebKubeconfigSection />);
    await waitFor(() => expect(webKubeconfigs.list).toHaveBeenCalledTimes(1));

    fireEvent.change(screen.getByLabelText("Kubeconfig name"), { target: { value: "new-cluster" } });
    fireEvent.change(screen.getByLabelText("Kubeconfig YAML"), {
      target: { value: "apiVersion: v1\nkind: Config\ncontexts: []" },
    });
    fireEvent.click(screen.getByRole("button", { name: /Upload kubeconfig/ }));

    await waitFor(() =>
      expect(webKubeconfigs.upload).toHaveBeenCalledWith("new-cluster", "apiVersion: v1\nkind: Config\ncontexts: []"),
    );
    expect(await screen.findByText("new-cluster")).toBeDefined();
  });

  it("removes a kubeconfig and refreshes the list", async () => {
    webKubeconfigs.remove.mockResolvedValue(undefined);
    webKubeconfigs.list
      .mockResolvedValueOnce([{ id: 1, name: "prod", createdAt: 0, updatedAt: 0 }])
      .mockResolvedValueOnce([]);
    render(<WebKubeconfigSection />);
    await screen.findByText("prod");

    fireEvent.click(screen.getByRole("button", { name: "Remove kubeconfig prod" }));

    await waitFor(() => expect(webKubeconfigs.remove).toHaveBeenCalledWith(1));
    await waitFor(() => expect(screen.queryByText("prod")).toBeNull());
  });

  it("surfaces list failures", async () => {
    webKubeconfigs.list.mockReset();
    webKubeconfigs.list.mockRejectedValue(new Error("network down"));
    render(<WebKubeconfigSection />);
    expect(await screen.findByText(/network down/)).toBeDefined();
  });
});
