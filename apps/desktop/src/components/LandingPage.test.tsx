import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import React from "react";

const listContexts = vi.fn();

vi.mock("../lib/clusters", () => ({
  listContexts: () => listContexts(),
}));

import { LandingPage } from "./LandingPage";

const contexts = [
  { name: "kind-dev", cluster: "kind-dev", server: "https://127.0.0.1:6443", isCurrent: true },
  { name: "production-eu", cluster: "prod-eu", server: "https://prod.example.com", isCurrent: false },
];

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
    await screen.findByText("No contexts match this filter.");
    fireEvent.click(screen.getByRole("button", { name: "Workspace preferences" }));
    expect(onOpenSettings).toHaveBeenCalledOnce();
  });
});
