import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import React from "react";

const listContexts = vi.fn();

vi.mock("../lib/clusters", () => ({
  listContexts: () => listContexts(),
}));

import { LandingPage } from "./LandingPage";

const contexts = [
  { name: "kind-dev", stableId: "/k/kind-dev.yaml#kind-dev", cluster: "kind-dev", server: "https://127.0.0.1:6443", isCurrent: true },
  { name: "production-eu", stableId: "/k/production-eu.yaml#production-eu", cluster: "prod-eu", server: "https://prod.example.com", isCurrent: false },
];

beforeEach(() => localStorage.clear());

describe("LandingPage", () => {
  it("prioritizes and opens the current context", async () => {
    listContexts.mockResolvedValue({ contexts });
    const onOpenContext = vi.fn();

    render(<LandingPage onOpenContext={onOpenContext} onOpenSettings={vi.fn()} />);

    expect(await screen.findByRole("button", { name: "Open current context kind-dev" })).toBeDefined();
    expect(screen.getByText("2 contexts")).toBeDefined();
    fireEvent.click(screen.getByRole("button", { name: "Open current context kind-dev" }));
    expect(onOpenContext).toHaveBeenCalledWith("kind-dev");
  });

  it("filters contexts and opens a matching result", async () => {
    listContexts.mockResolvedValue({ contexts });
    const onOpenContext = vi.fn();

    render(<LandingPage onOpenContext={onOpenContext} onOpenSettings={vi.fn()} />);
    await screen.findByText("production-eu");

    fireEvent.change(screen.getByPlaceholderText("Filter contexts"), { target: { value: "production" } });
    await waitFor(() => expect(screen.queryByRole("button", { name: "Open context kind-dev" })).toBeNull());
    fireEvent.click(screen.getByRole("button", { name: "Open context production-eu" }));
    expect(onOpenContext).toHaveBeenCalledWith("production-eu");
  });

  it("splits local and remote contexts into groups with a provider badge", async () => {
    listContexts.mockResolvedValue({
      contexts: [
        {
          name: "kind-dev",
          cluster: "kind-dev",
          server: "https://127.0.0.1:6443",
          isCurrent: false,
          isLocal: true,
          provider: "kind",
        },
        {
          name: "production-eu",
          cluster: "prod-eu",
          server: "https://prod.example.com",
          isCurrent: true,
        },
      ],
    });

    render(<LandingPage onOpenContext={vi.fn()} onOpenSettings={vi.fn()} />);
    await screen.findByRole("button", { name: "Open context kind-dev" });

    // Both groups are labelled, and the local row carries its provider badge.
    expect(screen.getByText("Local")).toBeDefined();
    expect(screen.getByText("Remote")).toBeDefined();
    expect(screen.getByText("kind")).toBeDefined();
  });

  it("opens workspace preferences from the masthead", async () => {
    listContexts.mockResolvedValue({ contexts: [] });
    const onOpenSettings = vi.fn();

    render(<LandingPage onOpenContext={vi.fn()} onOpenSettings={onOpenSettings} />);
    await screen.findByText(/No clusters yet/);
    fireEvent.click(screen.getByRole("button", { name: "Workspace preferences" }));
    expect(onOpenSettings).toHaveBeenCalledOnce();
  });

  it("offers a way in when the kubeconfig has no contexts (#161)", async () => {
    // The old text blamed a filter the user had not set, and offered nothing
    // to do about it.
    listContexts.mockResolvedValue({ contexts: [] });
    const onOpenSettings = vi.fn();

    render(<LandingPage onOpenContext={vi.fn()} onOpenSettings={onOpenSettings} />);
    fireEvent.click(await screen.findByRole("button", { name: /Add or paste a kubeconfig/ }));
    expect(onOpenSettings).toHaveBeenCalledOnce();
    // And no first-run tips: this user has a more specific problem.
    expect(screen.queryByText("New here?")).toBeNull();
  });

  it("shows first-run help once, and retires it when a cluster is opened", async () => {
    listContexts.mockResolvedValue({
      contexts: [{ name: "kind-dev", cluster: "kind", server: "https://x", isCurrent: true }],
    });
    const onOpenContext = vi.fn();

    const { unmount } = render(
      <LandingPage onOpenContext={onOpenContext} onOpenSettings={vi.fn()} />,
    );
    expect(await screen.findByText("New here?")).toBeDefined();
    fireEvent.click(screen.getByRole("button", { name: "Open context kind-dev" }));
    expect(onOpenContext).toHaveBeenCalledWith("kind-dev");

    unmount();
    render(<LandingPage onOpenContext={vi.fn()} onOpenSettings={vi.fn()} />);
    await screen.findByRole("button", { name: "Open context kind-dev" });
    expect(screen.queryByText("New here?")).toBeNull();
  });

  it("dismisses first-run help for good", async () => {
    listContexts.mockResolvedValue({
      contexts: [{ name: "kind-dev", cluster: "kind", server: "https://x", isCurrent: true }],
    });
    const { unmount } = render(<LandingPage onOpenContext={vi.fn()} onOpenSettings={vi.fn()} />);
    fireEvent.click(await screen.findByRole("button", { name: "Dismiss" }));
    expect(screen.queryByText("New here?")).toBeNull();

    unmount();
    render(<LandingPage onOpenContext={vi.fn()} onOpenSettings={vi.fn()} />);
    await screen.findByRole("button", { name: "Open context kind-dev" });
    expect(screen.queryByText("New here?")).toBeNull();
  });
});
