import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import React from "react";

const { listContextsMock } = vi.hoisted(() => ({ listContextsMock: vi.fn() }));
vi.mock("@srelens/core/lib/clusters", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@srelens/core/lib/clusters")>();
  return { ...actual, listContexts: listContextsMock };
});

import { ClusterHotbar } from "./ClusterHotbar";

beforeEach(() => listContextsMock.mockReset());

const theme = { name: "slate" as const, mode: "dark" as const };

describe("ClusterHotbar", () => {
  it("renders an avatar per cluster and opens one on click", async () => {
    listContextsMock.mockResolvedValue({
      contexts: [
        { name: "kind-dev", stableId: "/k/kind-dev.yaml#kind-dev", cluster: "k", server: "s", isCurrent: true },
        { name: "prod", stableId: "/k/prod.yaml#prod", cluster: "p", server: "s", isCurrent: false },
      ],
    });
    const onOpenContext = vi.fn();
    render(
      <ClusterHotbar
        openContext="kind-dev"
        onOpenContext={onOpenContext}
        theme={theme}
        onToggleTheme={() => {}}
        onOpenSettings={() => {}}
      />,
    );

    await waitFor(() => expect(screen.getByLabelText("kind-dev")).toBeDefined());
    // initials rendered
    expect(screen.getByText("KD")).toBeDefined();
    fireEvent.click(screen.getByLabelText("prod"));
    expect(onOpenContext).toHaveBeenCalledWith("prod");
  });

  it("separates local clusters from remote ones with a divider", async () => {
    listContextsMock.mockResolvedValue({
      contexts: [
        { name: "prod", stableId: "/k/prod.yaml#prod", cluster: "p", server: "s", isCurrent: false },
        { name: "kind-dev", stableId: "/k/kind-dev.yaml#kind-dev", cluster: "k", server: "s", isCurrent: true, isLocal: true, provider: "kind" },
      ],
    });
    render(
      <ClusterHotbar
        openContext="kind-dev"
        onOpenContext={() => {}}
        theme={theme}
        onToggleTheme={() => {}}
        onOpenSettings={() => {}}
      />,
    );

    await waitFor(() => expect(screen.getByLabelText("kind-dev (local)")).toBeDefined());
    expect(screen.getByRole("separator", { name: "Local clusters" })).toBeDefined();
    // Remote clusters keep their plain label.
    expect(screen.getByLabelText("prod")).toBeDefined();
  });

  it("toggles the theme", async () => {
    listContextsMock.mockResolvedValue({ contexts: [] });
    const onToggleTheme = vi.fn();
    render(
      <ClusterHotbar
        openContext={null}
        onOpenContext={() => {}}
        theme={theme}
        onToggleTheme={onToggleTheme}
        onOpenSettings={() => {}}
      />,
    );
    fireEvent.click(screen.getByLabelText("Switch to light mode"));
    expect(onToggleTheme).toHaveBeenCalled();
  });

  it("opens global settings from the gear button without a cluster context", async () => {
    listContextsMock.mockResolvedValue({ contexts: [] });
    const onOpenSettings = vi.fn();
    render(
      <ClusterHotbar
        openContext={null}
        onOpenContext={() => {}}
        theme={theme}
        onToggleTheme={() => {}}
        onOpenSettings={onOpenSettings}
      />,
    );
    fireEvent.click(screen.getByLabelText("Open settings"));
    expect(onOpenSettings).toHaveBeenCalled();
  });
});
