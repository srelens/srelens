import { describe, it, expect, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import React from "react";

// Usage polls metrics; stub it so the status-bar tests stay focused.
vi.mock("./ClusterUsage", () => ({ ClusterUsage: () => <span data-testid="usage" /> }));

import { StatusBar } from "./StatusBar";

describe("StatusBar", () => {
  it("shows the active cluster, view, and tab count", () => {
    render(<StatusBar activeCluster="kind-dev" activeLabel="Pods" tabCount={3} />);
    expect(screen.getByText("kind-dev")).toBeDefined();
    expect(screen.getByText("Pods")).toBeDefined();
    expect(screen.getByText("3 tabs")).toBeDefined();
  });

  it("shows a not-connected state with no cluster", () => {
    render(<StatusBar activeCluster={null} tabCount={0} />);
    expect(screen.getByText("Not connected")).toBeDefined();
    expect(screen.getByText("0 tabs")).toBeDefined();
  });

  it("uses the singular for a single tab", () => {
    render(<StatusBar activeCluster="dev" tabCount={1} />);
    expect(screen.getByText("1 tab")).toBeDefined();
  });

  it("offers a terminal launcher only when a handler is provided", () => {
    const onOpenTerminal = vi.fn();
    const contexts = [{ name: "dev", label: "dev" }];
    const { rerender } = render(
      <StatusBar activeCluster="dev" tabCount={1} terminalContexts={contexts} />,
    );
    expect(screen.queryByRole("button", { name: "Open kubectl terminal" })).toBeNull();

    rerender(
      <StatusBar
        activeCluster="dev"
        tabCount={1}
        terminalContexts={contexts}
        onOpenTerminal={onOpenTerminal}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Open kubectl terminal" }));
    expect(onOpenTerminal).toHaveBeenCalledWith("dev");
  });

  it("asks which context when several are configured", async () => {
    const onOpenTerminal = vi.fn();
    render(
      <StatusBar
        activeCluster="dev"
        tabCount={1}
        terminalContexts={[
          { name: "dev", label: "dev" },
          { name: "prod", label: "production" },
        ]}
        onOpenTerminal={onOpenTerminal}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Open kubectl terminal" }));
    // A shell for a second cluster used to require opening a tab for it first.
    fireEvent.click(await screen.findByRole("menuitem", { name: "production" }));
    expect(onOpenTerminal).toHaveBeenCalledWith("prod");
  });

  it("puts the open context first, wherever it sits in the list", async () => {
    // With a kubeconfig full of contexts the one you are already in would
    // otherwise be somewhere down a scrolling menu.
    render(
      <StatusBar
        activeCluster="prod"
        tabCount={1}
        terminalContexts={[
          { name: "dev", label: "dev" },
          { name: "staging", label: "staging" },
          { name: "prod", label: "production" },
        ]}
        onOpenTerminal={vi.fn()}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Open kubectl terminal" }));
    const items = await screen.findAllByRole("menuitem");
    expect(items.map((i) => i.textContent)).toEqual(["production", "dev", "staging"]);
  });

  it("still launches a terminal on a tab with no cluster", async () => {
    // Settings/Toolbox/Assistant tabs have no cluster; the launcher used to
    // disappear on them entirely, even with clusters configured (#257).
    const onOpenTerminal = vi.fn();
    render(
      <StatusBar
        activeCluster={null}
        tabCount={1}
        terminalContexts={[
          { name: "dev", label: "dev" },
          { name: "prod", label: "production" },
        ]}
        onOpenTerminal={onOpenTerminal}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Open kubectl terminal" }));
    fireEvent.click(await screen.findByRole("menuitem", { name: "dev" }));
    expect(onOpenTerminal).toHaveBeenCalledWith("dev");
  });

  it("hides the launcher when nothing is configured", () => {
    render(<StatusBar activeCluster={null} tabCount={0} onOpenTerminal={vi.fn()} />);
    expect(screen.queryByRole("button", { name: "Open kubectl terminal" })).toBeNull();
  });
});
