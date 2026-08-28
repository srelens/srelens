import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { WorkspaceTree, type WorkspaceCluster } from "./WorkspaceTree";

const clusters: WorkspaceCluster[] = [
  { id: "prod", name: "prod-eu", detail: "gke_prod_eu", count: 42 },
  { id: "stage", name: "stage", detail: "gke_stage", count: 12 },
  { id: "kind", name: "kind-local", detail: "kind-kind", link: "disconnected" },
];

function rowNames() {
  return screen.getAllByRole("button", { name: /^(prod-eu|stage|kind-local)/ }).map((b) => b.textContent);
}

describe("WorkspaceTree", () => {
  it("renders the workspace and every cluster under it, in order, with its count", () => {
    render(<WorkspaceTree name="Platform" clusters={clusters} onActivate={() => {}} />);
    expect(screen.getByRole("button", { name: /Platform/ }).textContent).toContain("3");
    // The offline cluster shows its link state where a count would go, and it
    // is part of the row's name rather than a colour a screen reader misses.
    expect(rowNames()).toEqual(["prod-eu42", "stage12", "kind-localOffline"]);
  });

  it("names the list after the workspace", () => {
    render(<WorkspaceTree name="Platform" clusters={clusters} onActivate={() => {}} />);
    expect(screen.getByRole("list", { name: /Platform/ })).toBeDefined();
  });

  it("reports which cluster was chosen", async () => {
    const onActivate = vi.fn();
    render(<WorkspaceTree name="Platform" clusters={clusters} onActivate={onActivate} />);
    await userEvent.click(screen.getByRole("button", { name: /^stage/ }));
    expect(onActivate).toHaveBeenCalledWith("stage");
  });

  it("marks the active cluster to assistive technology, not only in the styling", () => {
    render(<WorkspaceTree name="Platform" clusters={clusters} active="stage" onActivate={() => {}} />);
    expect(screen.getByRole("button", { name: /^stage/ }).getAttribute("aria-current")).toBe("true");
    expect(screen.getByRole("button", { name: /^prod-eu/ }).hasAttribute("aria-current")).toBe(false);
  });

  it("shows whatever mark the caller puts against a cluster", () => {
    render(
      <WorkspaceTree
        name="Platform"
        clusters={[{ id: "prod", name: "prod-eu", chip: <span data-testid="chip" />, meta: <span data-testid="meta" /> }]}
        onActivate={() => {}}
      />,
    );
    // Which colour a kubeconfig-backed cluster wears is the app's vocabulary.
    expect(screen.getByTestId("chip")).toBeDefined();
    expect(screen.getByTestId("meta")).toBeDefined();
  });
});

describe("WorkspaceTree folding", () => {
  it("folds a cluster open and shut from a caret that says which cluster it is", async () => {
    const onExpandedChange = vi.fn();
    render(
      <WorkspaceTree
        name="Platform"
        clusters={clusters}
        onActivate={() => {}}
        onExpandedChange={onExpandedChange}
        renderExpanded={(c) => <p>resources for {c.name}</p>}
      />,
    );
    const caret = screen.getByRole("button", { name: "Expand prod-eu" });
    expect(caret.getAttribute("aria-expanded")).toBe("false");
    expect(screen.queryByText("resources for prod-eu")).toBeNull();

    await userEvent.click(caret);
    expect(onExpandedChange).toHaveBeenCalledWith("prod", true);
    expect(screen.getByText("resources for prod-eu")).toBeDefined();
    expect(screen.getByRole("button", { name: "Collapse prod-eu" })).toBeDefined();

    await userEvent.click(screen.getByRole("button", { name: "Collapse prod-eu" }));
    expect(onExpandedChange).toHaveBeenLastCalledWith("prod", false);
    expect(screen.queryByText("resources for prod-eu")).toBeNull();
  });

  it("ties the caret to the region it opens", async () => {
    render(
      <WorkspaceTree
        name="Platform"
        clusters={clusters}
        onActivate={() => {}}
        renderExpanded={(c) => <p>resources for {c.name}</p>}
      />,
    );
    const caret = screen.getByRole("button", { name: "Expand prod-eu" });
    await userEvent.click(caret);
    const controlled = document.getElementById(caret.getAttribute("aria-controls") ?? "");
    expect(controlled?.textContent).toBe("resources for prod-eu");
  });

  it("lets the caller own which clusters are open", async () => {
    const onExpandedChange = vi.fn();
    render(
      <WorkspaceTree
        name="Platform"
        clusters={clusters}
        expanded={["stage"]}
        onExpandedChange={onExpandedChange}
        onActivate={() => {}}
        renderExpanded={(c) => <p>resources for {c.name}</p>}
      />,
    );
    expect(screen.getByText("resources for stage")).toBeDefined();
    await userEvent.click(screen.getByRole("button", { name: "Collapse stage" }));
    expect(onExpandedChange).toHaveBeenCalledWith("stage", false);
    // Still open: a controlled tree does not move until the caller says so.
    expect(screen.getByText("resources for stage")).toBeDefined();
  });

  it("folds the whole workspace away", async () => {
    render(<WorkspaceTree name="Platform" clusters={clusters} onActivate={() => {}} />);
    const root = screen.getByRole("button", { name: /Platform/ });
    expect(root.getAttribute("aria-expanded")).toBe("true");
    await userEvent.click(root);
    expect(root.getAttribute("aria-expanded")).toBe("false");
    expect(screen.queryByRole("button", { name: /^prod-eu/ })).toBeNull();
  });

  it("offers no caret when there is nothing to put under a cluster", () => {
    render(<WorkspaceTree name="Platform" clusters={clusters} onActivate={() => {}} />);
    // No renderExpanded: a disclosure that opens an empty box is a lie.
    expect(screen.queryByRole("button", { name: "Expand prod-eu" })).toBeNull();
  });
});

describe("WorkspaceTree connection states", () => {
  it("will not pretend a disconnected cluster can be opened", () => {
    render(
      <WorkspaceTree
        name="Platform"
        clusters={clusters}
        onActivate={() => {}}
        renderExpanded={() => <p>resources</p>}
      />,
    );
    const caret = screen.getByRole("button", { name: "Expand kind-local" });
    expect(caret.hasAttribute("disabled")).toBe(true);
    expect(caret.getAttribute("title")).toMatch(/connect/i);
  });

  it("says a cluster is offline", () => {
    render(<WorkspaceTree name="Platform" clusters={clusters} onActivate={() => {}} />);
    expect(screen.getByRole("button", { name: /^kind-local/ }).textContent).toContain("Offline");
  });

  it("offers to connect an offline cluster from a button that says so", async () => {
    const onConnect = vi.fn();
    const onActivate = vi.fn();
    render(
      <WorkspaceTree name="Platform" clusters={clusters} onActivate={onActivate} onConnect={onConnect} />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Connect kind-local" }));
    expect(onConnect).toHaveBeenCalledWith("kind");
    // The row still means "show me this cluster", whatever its link state. In
    // the mock the same click meant connect or select depending on state the
    // user could only infer from a dimmed row.
    expect(onActivate).not.toHaveBeenCalled();
  });

  it("only offers to connect what is not connected", () => {
    const onConnect = vi.fn();
    render(<WorkspaceTree name="Platform" clusters={clusters} onActivate={() => {}} onConnect={onConnect} />);
    expect(screen.queryByRole("button", { name: "Connect prod-eu" })).toBeNull();
  });

  it("announces a cluster that is still connecting", () => {
    render(
      <WorkspaceTree
        name="Platform"
        clusters={[{ id: "prod", name: "prod-eu", link: "connecting" }]}
        onActivate={() => {}}
      />,
    );
    expect(screen.getByRole("status", { name: "Connecting to prod-eu" })).toBeDefined();
  });

  it("ties a failed connection's reason to the row it belongs to", () => {
    render(
      <WorkspaceTree
        name="Platform"
        clusters={[{ id: "prod", name: "prod-eu", link: "error", error: "certificate expired" }]}
        onActivate={() => {}}
        onConnect={() => {}}
      />,
    );
    const row = screen.getByRole("button", { name: /^prod-eu/ });
    const reason = document.getElementById(row.getAttribute("aria-describedby") ?? "");
    expect(reason?.textContent).toBe("certificate expired");
    // A failure is a way back in, not a dead end.
    expect(screen.getByRole("button", { name: "Connect prod-eu" })).toBeDefined();
  });

  it("leaves a healthy row undescribed", () => {
    render(<WorkspaceTree name="Platform" clusters={clusters} onActivate={() => {}} />);
    expect(screen.getByRole("button", { name: /^prod-eu/ }).hasAttribute("aria-describedby")).toBe(
      false,
    );
  });
});

describe("WorkspaceTree drill-in", () => {
  it("reports the cluster to drill into, from a button that names it", async () => {
    const onDrillIn = vi.fn();
    render(<WorkspaceTree name="Platform" clusters={clusters} onActivate={() => {}} onDrillIn={onDrillIn} />);
    await userEvent.click(screen.getByRole("button", { name: "Drill into prod-eu" }));
    expect(onDrillIn).toHaveBeenCalledWith("prod");
  });

  it("is there for a keyboard to reach, not only for a hovering pointer", () => {
    // The mock's drill-in sits at opacity 0 until the row is hovered, so a
    // keyboard user tabs onto a control they cannot see.
    render(<WorkspaceTree name="Platform" clusters={clusters} onActivate={() => {}} onDrillIn={() => {}} />);
    const drill = screen.getByRole("button", { name: "Drill into prod-eu" });
    expect(drill.className).not.toContain("drill-in");
  });

  it("offers nothing to drill into a cluster that is not connected", () => {
    render(<WorkspaceTree name="Platform" clusters={clusters} onActivate={() => {}} onDrillIn={() => {}} />);
    expect(screen.queryByRole("button", { name: "Drill into kind-local" })).toBeNull();
  });

  it("says nothing about drilling in when the caller has nowhere to drill", () => {
    render(<WorkspaceTree name="Platform" clusters={clusters} onActivate={() => {}} />);
    expect(screen.queryByRole("button", { name: /Drill into/ })).toBeNull();
  });
});

describe("WorkspaceTree with no clusters", () => {
  it("says the workspace is empty rather than showing a bare heading", () => {
    render(<WorkspaceTree name="Platform" clusters={[]} onActivate={() => {}} />);
    expect(screen.queryByRole("list")).toBeNull();
    expect(screen.getByText("No clusters")).toBeDefined();
  });

  it("takes the caller's words, and the way out of the emptiness", async () => {
    const onClick = vi.fn();
    render(
      <WorkspaceTree
        name="Platform"
        clusters={[]}
        onActivate={() => {}}
        emptyTitle="Nothing in this workspace"
        emptyHint="Add a kubeconfig to get started."
        emptyAction={<button type="button" onClick={onClick}>Add cluster</button>}
      />,
    );
    expect(screen.getByText("Nothing in this workspace")).toBeDefined();
    expect(screen.getByText("Add a kubeconfig to get started.")).toBeDefined();
    await userEvent.click(screen.getByRole("button", { name: "Add cluster" }));
    expect(onClick).toHaveBeenCalled();
  });
});

/**
 * The keyboard contract. This one is a list rather than a `role="tree"`, so
 * what it owes is a sane tab order over ordinary controls — see the note on the
 * component for why the tree pattern does not fit a row holding three of them.
 */
describe("WorkspaceTree keyboard behaviour", () => {
  it("puts every control of a row in the tab order, left to right", async () => {
    render(
      <WorkspaceTree
        name="Platform"
        clusters={[clusters[0]]}
        onActivate={() => {}}
        onDrillIn={() => {}}
        renderExpanded={() => <p>resources</p>}
      />,
    );
    screen.getByRole("button", { name: /Platform/ }).focus();
    await userEvent.tab();
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Expand prod-eu" }));
    await userEvent.tab();
    expect(document.activeElement).toBe(screen.getByRole("button", { name: /^prod-eu/ }));
    await userEvent.tab();
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Drill into prod-eu" }));
  });

  it("opens a cluster from the keyboard", async () => {
    render(
      <WorkspaceTree
        name="Platform"
        clusters={[clusters[0]]}
        onActivate={() => {}}
        renderExpanded={() => <p>resources</p>}
      />,
    );
    screen.getByRole("button", { name: "Expand prod-eu" }).focus();
    await userEvent.keyboard("{Enter}");
    expect(screen.getByText("resources")).toBeDefined();
  });

  it("chooses a cluster from the keyboard", async () => {
    const onActivate = vi.fn();
    render(<WorkspaceTree name="Platform" clusters={clusters} onActivate={onActivate} />);
    screen.getByRole("button", { name: /^stage/ }).focus();
    await userEvent.keyboard(" ");
    expect(onActivate).toHaveBeenCalledWith("stage");
  });
});

describe("WorkspaceTree inside a form", () => {
  it("never submits the form it is standing in", async () => {
    const onSubmit = vi.fn((e: React.FormEvent) => e.preventDefault());
    const { container } = render(
      <form onSubmit={onSubmit}>
        <WorkspaceTree
          name="Platform"
          clusters={clusters}
          onActivate={() => {}}
          onConnect={() => {}}
          onDrillIn={() => {}}
          renderExpanded={() => <p>resources</p>}
        />
      </form>,
    );
    await userEvent.click(screen.getByRole("button", { name: /^prod-eu/ }));
    await userEvent.click(screen.getByRole("button", { name: "Expand prod-eu" }));
    await userEvent.click(screen.getByRole("button", { name: /Platform/ }));
    expect(onSubmit).not.toHaveBeenCalled();
    for (const button of container.querySelectorAll("button")) {
      expect(button.getAttribute("type")).toBe("button");
    }
  });
});
