import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ResourceTree, filterResourceNodes, type ResourceNode } from "./ResourceTree";

function Glyph({ className }: { className?: string }) {
  return <svg data-testid="glyph" className={className} />;
}

const nodes: ResourceNode[] = [
  {
    id: "cluster",
    label: "Cluster",
    children: [
      { id: "nodes", label: "Nodes", count: 42, icon: Glyph },
      { id: "namespaces", label: "Namespaces", count: 28 },
    ],
  },
  {
    id: "workloads",
    label: "Workloads",
    children: [
      { id: "pods", label: "Pods", count: 1284, icon: Glyph },
      { id: "deployments", label: "Deployments", count: 63 },
    ],
  },
];

function labels() {
  return screen.getAllByRole("treeitem").map((row) => row.textContent?.trim());
}

describe("ResourceTree", () => {
  it("renders every node in the order it was given, with its count", () => {
    render(<ResourceTree label="Resources" nodes={nodes} onActivate={() => {}} />);
    expect(labels()).toEqual([
      "Cluster",
      "Nodes42",
      "Namespaces28",
      "Workloads",
      "Pods1284",
      "Deployments63",
    ]);
  });

  it("reports which node was activated", async () => {
    const onActivate = vi.fn();
    render(<ResourceTree label="Resources" nodes={nodes} onActivate={onActivate} />);
    await userEvent.click(screen.getByRole("treeitem", { name: "Pods 1284" }));
    expect(onActivate).toHaveBeenCalledWith("pods");
  });

  it("does not activate the group it is folding", async () => {
    const onActivate = vi.fn();
    render(<ResourceTree label="Resources" nodes={nodes} onActivate={onActivate} />);
    await userEvent.click(screen.getByRole("treeitem", { name: "Workloads" }));
    expect(onActivate).not.toHaveBeenCalled();
  });

  it("marks the active node to assistive technology, not only in the styling", () => {
    render(<ResourceTree label="Resources" nodes={nodes} active="pods" onActivate={() => {}} />);
    const pods = screen.getByRole("treeitem", { name: "Pods 1284" });
    expect(pods.getAttribute("aria-selected")).toBe("true");
    expect(screen.getByRole("treeitem", { name: "Nodes 42" }).getAttribute("aria-selected")).toBe(
      "false",
    );
  });

  it("carries the depth of each row", () => {
    render(<ResourceTree label="Resources" nodes={nodes} onActivate={() => {}} />);
    expect(screen.getByRole("treeitem", { name: "Cluster" }).getAttribute("aria-level")).toBe("1");
    expect(screen.getByRole("treeitem", { name: "Pods 1284" }).getAttribute("aria-level")).toBe("2");
  });

  it("names the tree", () => {
    render(<ResourceTree label="Resources" nodes={nodes} onActivate={() => {}} />);
    expect(screen.getByRole("tree", { name: "Resources" })).toBeDefined();
  });

  it("shows the icon a node was given, and hides it from assistive technology", () => {
    render(<ResourceTree label="Resources" nodes={nodes} onActivate={() => {}} />);
    // Two nodes carry one; the tree invents none for the rest, because which
    // glyph means "pods" is the app's vocabulary.
    expect(screen.getAllByTestId("glyph")).toHaveLength(2);
    expect(screen.getByRole("treeitem", { name: "Nodes 42" }).textContent).toBe("Nodes42");
  });
});

describe("ResourceTree folding", () => {
  it("says whether a group is open, and hides the items of a closed one", async () => {
    render(<ResourceTree label="Resources" nodes={nodes} onActivate={() => {}} />);
    const workloads = screen.getByRole("treeitem", { name: "Workloads" });
    expect(workloads.getAttribute("aria-expanded")).toBe("true");

    await userEvent.click(workloads);
    expect(workloads.getAttribute("aria-expanded")).toBe("false");
    expect(screen.queryByRole("treeitem", { name: "Pods 1284" })).toBeNull();
  });

  it("does not claim a leaf can be expanded", () => {
    render(<ResourceTree label="Resources" nodes={nodes} onActivate={() => {}} />);
    expect(
      screen.getByRole("treeitem", { name: "Pods 1284" }).hasAttribute("aria-expanded"),
    ).toBe(false);
  });

  it("reports every fold and unfold", async () => {
    const onExpandedChange = vi.fn();
    render(
      <ResourceTree
        label="Resources"
        nodes={nodes}
        onActivate={() => {}}
        onExpandedChange={onExpandedChange}
      />,
    );
    await userEvent.click(screen.getByRole("treeitem", { name: "Cluster" }));
    expect(onExpandedChange).toHaveBeenLastCalledWith("cluster", false);
    await userEvent.click(screen.getByRole("treeitem", { name: "Cluster" }));
    expect(onExpandedChange).toHaveBeenLastCalledWith("cluster", true);
  });

  it("starts a group closed when it asks to be", () => {
    const folded: ResourceNode[] = [
      { id: "cluster", label: "Cluster", defaultExpanded: false, children: [{ id: "nodes", label: "Nodes" }] },
    ];
    render(<ResourceTree label="Resources" nodes={folded} onActivate={() => {}} />);
    expect(screen.getByRole("treeitem", { name: "Cluster" }).getAttribute("aria-expanded")).toBe(
      "false",
    );
    expect(screen.queryByRole("treeitem", { name: "Nodes" })).toBeNull();
  });

  it("lets the caller own the fold state", async () => {
    const onExpandedChange = vi.fn();
    const { rerender } = render(
      <ResourceTree
        label="Resources"
        nodes={nodes}
        expanded={["cluster"]}
        onExpandedChange={onExpandedChange}
        onActivate={() => {}}
      />,
    );
    // Workloads is not in the caller's set, so it is closed however the tree
    // would have defaulted it.
    expect(screen.queryByRole("treeitem", { name: "Pods 1284" })).toBeNull();

    await userEvent.click(screen.getByRole("treeitem", { name: "Workloads" }));
    expect(onExpandedChange).toHaveBeenCalledWith("workloads", true);
    // Still closed: a controlled tree does not move until the caller says so.
    expect(screen.queryByRole("treeitem", { name: "Pods 1284" })).toBeNull();

    rerender(
      <ResourceTree
        label="Resources"
        nodes={nodes}
        expanded={["cluster", "workloads"]}
        onExpandedChange={onExpandedChange}
        onActivate={() => {}}
      />,
    );
    expect(screen.getByRole("treeitem", { name: "Pods 1284" })).toBeDefined();
  });
});

describe("ResourceTree with nothing to show", () => {
  it("says the tree is empty rather than rendering an empty box", () => {
    render(<ResourceTree label="Resources" nodes={[]} onActivate={() => {}} />);
    expect(screen.queryByRole("tree")).toBeNull();
    expect(screen.getByText("Nothing here")).toBeDefined();
  });

  it("takes the caller's words for the empty state", () => {
    render(
      <ResourceTree
        label="Resources"
        nodes={[]}
        onActivate={() => {}}
        emptyTitle="No resources"
        emptyHint="Connect a cluster to see its resources."
      />,
    );
    expect(screen.getByText("No resources")).toBeDefined();
    expect(screen.getByText("Connect a cluster to see its resources.")).toBeDefined();
  });

  it("announces a failed load and offers a way out of it", async () => {
    const onRetry = vi.fn();
    render(
      <ResourceTree
        label="Resources"
        nodes={[]}
        onActivate={() => {}}
        error={{ title: "Could not list resources", detail: "connection refused", onRetry }}
      />,
    );
    const alert = screen.getByRole("alert");
    expect(alert.textContent).toContain("Could not list resources");
    expect(alert.textContent).toContain("connection refused");
    await userEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(onRetry).toHaveBeenCalled();
  });

  it("shows the failure even when it has nodes from a previous load", () => {
    render(
      <ResourceTree
        label="Resources"
        nodes={nodes}
        onActivate={() => {}}
        error={{ title: "Could not list resources" }}
      />,
    );
    // Stale rows that quietly stopped updating are worse than an honest error.
    expect(screen.queryByRole("tree")).toBeNull();
    expect(screen.getByRole("alert")).toBeDefined();
  });
});

describe("ResourceTree filtering", () => {
  it("keeps a matching leaf and the groups above it", () => {
    render(<ResourceTree label="Resources" nodes={nodes} query="pod" onActivate={() => {}} />);
    expect(labels()).toEqual(["Workloads", "Pods1284"]);
  });

  it("opens the groups a match is buried in", () => {
    const folded = nodes.map((n) => ({ ...n, defaultExpanded: false }));
    render(<ResourceTree label="Resources" nodes={folded} query="pod" onActivate={() => {}} />);
    // A search that finds a row and then hides it behind a fold has found nothing.
    expect(screen.getByRole("treeitem", { name: "Pods 1284" })).toBeDefined();
  });

  it("keeps the contents of a group whose own name matches", () => {
    // The mock filtered a matching group's children by the same query, so
    // searching for "Workloads" showed the group with nothing under it — the
    // one result you asked for, emptied out.
    render(<ResourceTree label="Resources" nodes={nodes} query="workload" onActivate={() => {}} />);
    expect(labels()).toEqual(["Workloads", "Pods1284", "Deployments63"]);
  });

  it("says nothing matched, quoting what was searched for", () => {
    render(<ResourceTree label="Resources" nodes={nodes} query="zzz" onActivate={() => {}} />);
    expect(screen.queryByRole("tree")).toBeNull();
    expect(screen.getByText(/zzz/)).toBeDefined();
  });

  it("ignores case and surrounding space", () => {
    render(<ResourceTree label="Resources" nodes={nodes} query="  PODS " onActivate={() => {}} />);
    expect(labels()).toEqual(["Workloads", "Pods1284"]);
  });
});

describe("filterResourceNodes", () => {
  it("is pure, so the app can reuse the same matching rule", () => {
    const result = filterResourceNodes(nodes, "node");
    expect(result).toEqual([
      { id: "cluster", label: "Cluster", children: [{ id: "nodes", label: "Nodes", count: 42, icon: Glyph }] },
    ]);
    // The input is untouched.
    expect(nodes[0].children).toHaveLength(2);
  });

  it("hands back everything for an empty query", () => {
    expect(filterResourceNodes(nodes, "   ")).toBe(nodes);
  });
});

/**
 * The tree's keyboard contract. `role="tree"` promises this — arrow keys that
 * walk the rows and fold the groups — so a tree without it is worse than a
 * column of plain buttons, because the role tells assistive technology to
 * expect behaviour that is not there.
 */
describe("ResourceTree keyboard behaviour", () => {
  it("is a single tab stop, landing on the active row", () => {
    render(<ResourceTree label="Resources" nodes={nodes} active="pods" onActivate={() => {}} />);
    expect(screen.getByRole("treeitem", { name: "Pods 1284" }).getAttribute("tabindex")).toBe("0");
    const stops = screen
      .getAllByRole("treeitem")
      .filter((row) => row.getAttribute("tabindex") === "0");
    expect(stops).toHaveLength(1);
  });

  it("lands on the first row when nothing is active", () => {
    render(<ResourceTree label="Resources" nodes={nodes} onActivate={() => {}} />);
    expect(screen.getByRole("treeitem", { name: "Cluster" }).getAttribute("tabindex")).toBe("0");
  });

  it("moves down and up through the rows that are showing", async () => {
    render(<ResourceTree label="Resources" nodes={nodes} onActivate={() => {}} />);
    screen.getByRole("treeitem", { name: "Cluster" }).focus();
    await userEvent.keyboard("{ArrowDown}");
    expect(document.activeElement).toBe(screen.getByRole("treeitem", { name: "Nodes 42" }));
    await userEvent.keyboard("{ArrowDown}{ArrowDown}");
    expect(document.activeElement).toBe(screen.getByRole("treeitem", { name: "Workloads" }));
    await userEvent.keyboard("{ArrowUp}");
    expect(document.activeElement).toBe(screen.getByRole("treeitem", { name: "Namespaces 28" }));
  });

  it("steps over a closed group's items rather than into them", async () => {
    const folded: ResourceNode[] = [
      { id: "cluster", label: "Cluster", defaultExpanded: false, children: [{ id: "nodes", label: "Nodes" }] },
      { id: "workloads", label: "Workloads", children: [{ id: "pods", label: "Pods" }] },
    ];
    render(<ResourceTree label="Resources" nodes={folded} onActivate={() => {}} />);
    screen.getByRole("treeitem", { name: "Cluster" }).focus();
    await userEvent.keyboard("{ArrowDown}");
    expect(document.activeElement).toBe(screen.getByRole("treeitem", { name: "Workloads" }));
  });

  it("stops at both ends instead of wrapping", async () => {
    // A tree is a structure, not a ring: wrapping from the last row to the
    // first loses the reader's place in it.
    render(<ResourceTree label="Resources" nodes={nodes} onActivate={() => {}} />);
    screen.getByRole("treeitem", { name: "Cluster" }).focus();
    await userEvent.keyboard("{ArrowUp}");
    expect(document.activeElement).toBe(screen.getByRole("treeitem", { name: "Cluster" }));
    await userEvent.keyboard("{End}{ArrowDown}");
    expect(document.activeElement).toBe(screen.getByRole("treeitem", { name: "Deployments 63" }));
  });

  it("opens a closed group with Right, then walks into it", async () => {
    const folded: ResourceNode[] = [
      { id: "cluster", label: "Cluster", defaultExpanded: false, children: [{ id: "nodes", label: "Nodes" }] },
    ];
    render(<ResourceTree label="Resources" nodes={folded} onActivate={() => {}} />);
    const cluster = screen.getByRole("treeitem", { name: "Cluster" });
    cluster.focus();
    await userEvent.keyboard("{ArrowRight}");
    expect(cluster.getAttribute("aria-expanded")).toBe("true");
    expect(document.activeElement).toBe(cluster);
    await userEvent.keyboard("{ArrowRight}");
    expect(document.activeElement).toBe(screen.getByRole("treeitem", { name: "Nodes" }));
  });

  it("closes an open group with Left, then climbs to its parent", async () => {
    render(<ResourceTree label="Resources" nodes={nodes} onActivate={() => {}} />);
    screen.getByRole("treeitem", { name: "Pods 1284" }).focus();
    await userEvent.keyboard("{ArrowLeft}");
    const workloads = screen.getByRole("treeitem", { name: "Workloads" });
    expect(document.activeElement).toBe(workloads);
    expect(workloads.getAttribute("aria-expanded")).toBe("true");
    await userEvent.keyboard("{ArrowLeft}");
    expect(workloads.getAttribute("aria-expanded")).toBe("false");
  });

  it("does nothing with Right on a leaf", async () => {
    const onActivate = vi.fn();
    render(<ResourceTree label="Resources" nodes={nodes} onActivate={onActivate} />);
    const pods = screen.getByRole("treeitem", { name: "Pods 1284" });
    pods.focus();
    await userEvent.keyboard("{ArrowRight}");
    expect(document.activeElement).toBe(pods);
    expect(onActivate).not.toHaveBeenCalled();
  });

  it("jumps to the first and last showing rows with Home and End", async () => {
    render(<ResourceTree label="Resources" nodes={nodes} onActivate={() => {}} />);
    screen.getByRole("treeitem", { name: "Nodes 42" }).focus();
    await userEvent.keyboard("{End}");
    expect(document.activeElement).toBe(screen.getByRole("treeitem", { name: "Deployments 63" }));
    await userEvent.keyboard("{Home}");
    expect(document.activeElement).toBe(screen.getByRole("treeitem", { name: "Cluster" }));
  });

  it("activates a row with Enter and with Space", async () => {
    const onActivate = vi.fn();
    render(<ResourceTree label="Resources" nodes={nodes} onActivate={onActivate} />);
    screen.getByRole("treeitem", { name: "Pods 1284" }).focus();
    await userEvent.keyboard("{Enter}");
    expect(onActivate).toHaveBeenLastCalledWith("pods");
    await userEvent.keyboard(" ");
    expect(onActivate).toHaveBeenLastCalledWith("pods");
    expect(onActivate).toHaveBeenCalledTimes(2);
  });

  it("leaves other keys alone", async () => {
    // The sidebar's filter box and the surrounding screen need them.
    const onActivate = vi.fn();
    const onExpandedChange = vi.fn();
    render(
      <ResourceTree
        label="Resources"
        nodes={nodes}
        onActivate={onActivate}
        onExpandedChange={onExpandedChange}
      />,
    );
    const cluster = screen.getByRole("treeitem", { name: "Cluster" });
    cluster.focus();
    await userEvent.keyboard("{PageDown}");
    await userEvent.keyboard("a");
    expect(document.activeElement).toBe(cluster);
    expect(onActivate).not.toHaveBeenCalled();
    expect(onExpandedChange).not.toHaveBeenCalled();
  });

  it("keeps the tab stop on a row that is still there after a fold", async () => {
    render(<ResourceTree label="Resources" nodes={nodes} active="pods" onActivate={() => {}} />);
    await userEvent.click(screen.getByRole("treeitem", { name: "Workloads" }));
    // The active row went away with the fold, so the stop must land somewhere
    // real or the tree drops out of the tab order entirely.
    const stops = screen
      .getAllByRole("treeitem")
      .filter((row) => row.getAttribute("tabindex") === "0");
    expect(stops).toHaveLength(1);
  });
});

describe("ResourceTree inside a form", () => {
  it("never submits the form it is standing in", async () => {
    // A bare <button> defaults to type="submit"; a sidebar tree that reloads
    // the surrounding form on every click is how this bug shows up.
    const onSubmit = vi.fn((e: React.FormEvent) => e.preventDefault());
    const { container } = render(
      <form onSubmit={onSubmit}>
        <ResourceTree label="Resources" nodes={nodes} onActivate={() => {}} />
      </form>,
    );
    await userEvent.click(screen.getByRole("treeitem", { name: "Pods 1284" }));
    await userEvent.click(screen.getByRole("treeitem", { name: "Workloads" }));
    expect(onSubmit).not.toHaveBeenCalled();
    for (const button of container.querySelectorAll("button")) {
      expect(button.getAttribute("type")).toBe("button");
    }
  });
});
