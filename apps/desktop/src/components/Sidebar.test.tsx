import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import React from "react";
import { Sidebar } from "./Sidebar";

const base = {
  clusters: ["kind-dev"],
  activeCluster: "kind-dev",
  activeKind: "pods" as const,
  onSelect: () => {},
  onSelectCrd: () => {},
};

describe("Sidebar", () => {
  it("renders grouped resource nav sections with the active kind marked", () => {
    render(<Sidebar {...base} />);
    expect(screen.getByText("Workloads")).toBeDefined();
    expect(screen.getByText("Config")).toBeDefined();
    expect(screen.getByText("Network")).toBeDefined();
    expect(screen.getByText("Custom Resources")).toBeDefined();
    expect(screen.getByRole("button", { name: "Pods" }).getAttribute("aria-current")).toBe("page");
  });

  it("selects a (cluster, kind) on click", () => {
    const onSelect = vi.fn();
    render(<Sidebar {...base} onSelect={onSelect} />);
    fireEvent.click(screen.getByRole("button", { name: "Network" }));
    fireEvent.click(screen.getByRole("button", { name: "Services" }));
    expect(onSelect).toHaveBeenCalledWith("kind-dev", "services");
    fireEvent.click(screen.getByRole("button", { name: "Config" }));
    fireEvent.click(screen.getByRole("button", { name: "ConfigMaps" }));
    expect(onSelect).toHaveBeenCalledWith("kind-dev", "configmaps");
  });

  it("opens only the active resource group by default", () => {
    render(<Sidebar {...base} />);
    expect(screen.getByRole("button", { name: "Pods" })).toBeDefined();
    expect(screen.queryByRole("button", { name: "Services" })).toBeNull();
    expect(screen.queryByRole("button", { name: "ConfigMaps" })).toBeNull();
  });

  it("announces the region and which rows expand (#160)", () => {
    render(<Sidebar {...base} />);
    // Two navigation regions in the app; "complementary" alone names neither.
    expect(screen.getByRole("complementary", { name: "Cluster resources" })).toBeDefined();
    // The caret is a decorative icon, so the state has to be in the markup.
    const group = screen.getByRole("button", { name: "Workloads" });
    expect(group.getAttribute("aria-expanded")).toBe("true");
    fireEvent.click(group);
    expect(screen.getByRole("button", { name: "Workloads" }).getAttribute("aria-expanded")).toBe(
      "false",
    );
  });

  it("renders a tree per opened cluster", () => {
    render(<Sidebar {...base} clusters={["kind-dev", "prod-east"]} />);
    expect(screen.getByRole("button", { name: /kind-dev/ })).toBeDefined();
    expect(screen.getByRole("button", { name: /prod-east/ })).toBeDefined();
  });

  it("collapses and expands a group", () => {
    render(<Sidebar {...base} />);
    expect(screen.getByRole("button", { name: "Pods" })).toBeDefined();
    fireEvent.click(screen.getByRole("button", { name: "Workloads" }));
    expect(screen.queryByRole("button", { name: "Pods" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Workloads" }));
    expect(screen.getByRole("button", { name: "Pods" })).toBeDefined();
  });

  it("collapses the whole tree from the cluster node", () => {
    render(<Sidebar {...base} />);
    fireEvent.click(screen.getByRole("button", { name: /kind-dev/ }));
    expect(screen.queryByText("Workloads")).toBeNull();
  });

  it("keeps resizing across the re-render that the width change causes", () => {
    const onResize = vi.fn();
    const { container, rerender } = render(<Sidebar {...base} width={200} onResize={onResize} />);
    const handle = container.querySelector(".fl-sidebar__resize") as HTMLElement;

    fireEvent.mouseDown(handle, { clientX: 500 });
    fireEvent.mouseMove(window, { clientX: 530 });
    expect(onResize).toHaveBeenLastCalledWith(230);

    rerender(<Sidebar {...base} width={230} onResize={onResize} />);
    fireEvent.mouseMove(window, { clientX: 560 });
    expect(onResize).toHaveBeenLastCalledWith(260);

    fireEvent.mouseUp(window);
    fireEvent.mouseMove(window, { clientX: 600 });
    expect(onResize).toHaveBeenLastCalledWith(260);
  });

  it("clamps the width to the 168–480 range", () => {
    const onResize = vi.fn();
    const { container } = render(<Sidebar {...base} width={200} onResize={onResize} />);
    const handle = container.querySelector(".fl-sidebar__resize") as HTMLElement;
    fireEvent.mouseDown(handle, { clientX: 500 });
    fireEvent.mouseMove(window, { clientX: 100 });
    expect(onResize).toHaveBeenLastCalledWith(168);
    fireEvent.mouseMove(window, { clientX: 900 });
    expect(onResize).toHaveBeenLastCalledWith(480);
  });
});
