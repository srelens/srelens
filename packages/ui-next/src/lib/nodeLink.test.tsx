import { beforeEach, describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ClusterContext } from "@srelens/core";
import { Table, type Column } from "@srelens/ui-kit";
import { resetContexts, setContexts } from "./clusters";
import { detailRoute } from "./detailRoute";
import { NodeLink } from "./nodeLink";
import { defaultState } from "./tabs";
import * as store from "./tabsStore";

const context = (name: string, stableId: string): ClusterContext => ({
  name,
  stableId,
  key: stableId,
  cluster: stableId,
  server: `https://${stableId}`,
  isCurrent: false,
  sourceFile: "/home/dana/.kube/config",
  authKind: "client certificate",
});
const PROD = context("prod-eu", "prod");
const STAGE = context("stage-eu", "stage");

const tabs = () => store.currentWorkspace().tabs;
const nodeTab = (name: string) => tabs().find((t) => t.route === detailRoute("Node", null, name));

beforeEach(() => {
  resetContexts();
  setContexts([PROD, STAGE]);
  store.setState(defaultState([PROD, STAGE]));
});

describe("NodeLink", () => {
  it("is a button named for what it opens, showing the node's own name", () => {
    render(<NodeLink name="worker-2" />);
    const link = screen.getByRole("button", { name: "Open node worker-2" });
    expect(link.textContent).toBe("worker-2");
  });

  it("opens the node's detail in a tab, on the cluster in focus", async () => {
    render(<NodeLink name="worker-2" />);
    await userEvent.click(screen.getByRole("button", { name: "Open node worker-2" }));

    // The route Overview's node rows and the Nodes list open: one node, one
    // tab, whichever way the reader came to it.
    const tab = nodeTab("worker-2");
    expect(tab).toBeDefined();
    expect(tab!.sub).toBe("prod-eu");
    expect(store.currentWorkspace().activeId).toBe(tab!.id);
  });

  it("opens it on the cluster the reader has moved to, not the one first in the list", async () => {
    store.setActiveCluster(STAGE.stableId);
    render(<NodeLink name="worker-2" />);
    await userEvent.click(screen.getByRole("button", { name: "Open node worker-2" }));
    expect(nodeTab("worker-2")!.sub).toBe("stage-eu");
  });

  it("focuses the node's tab rather than opening a second one", async () => {
    render(<NodeLink name="worker-2" />);
    const link = screen.getByRole("button", { name: "Open node worker-2" });
    await userEvent.click(link);
    const count = tabs().length;
    await userEvent.click(link);
    expect(tabs().length).toBe(count);
  });

  it("is plain text when no cluster is resolved — there is nowhere to open it", () => {
    resetContexts();
    setContexts([]);
    store.setState(defaultState([]));
    render(<NodeLink name="worker-2" />);
    expect(screen.queryByRole("button")).toBeNull();
    expect(screen.getByText("worker-2")).toBeDefined();
  });

  describe("inside a table row", () => {
    type Row = { name: string; node: string };
    const COLUMNS: Column<Row>[] = [
      { key: "name", header: "Name" },
      { key: "node", header: "Node", render: (row) => <NodeLink name={row.node} /> },
    ];

    function renderRow() {
      const onRowClick = vi.fn();
      const onRowActivate = vi.fn();
      render(
        <Table
          columns={COLUMNS}
          data={[{ name: "web-0", node: "worker-2" }]}
          getRowKey={(row) => row.name}
          onRowClick={onRowClick}
          onRowActivate={onRowActivate}
        />,
      );
      return { onRowClick, onRowActivate };
    }

    it("opens the node without peeking the pod's row", async () => {
      const { onRowClick, onRowActivate } = renderRow();
      await userEvent.click(screen.getByRole("button", { name: "Open node worker-2" }));
      expect(nodeTab("worker-2")).toBeDefined();
      expect(onRowClick).not.toHaveBeenCalled();
      expect(onRowActivate).not.toHaveBeenCalled();
    });

    it("does not open the pod on a double-click of the node", async () => {
      const { onRowActivate } = renderRow();
      await userEvent.dblClick(screen.getByRole("button", { name: "Open node worker-2" }));
      expect(onRowActivate).not.toHaveBeenCalled();
    });

    it("opens the node, not the pod, on Enter and on Space", async () => {
      const { onRowActivate } = renderRow();
      const link = screen.getByRole("button", { name: "Open node worker-2" });

      link.focus();
      await userEvent.keyboard("{Enter}");
      expect(nodeTab("worker-2")).toBeDefined();

      link.focus();
      await userEvent.keyboard(" ");
      expect(onRowActivate).not.toHaveBeenCalled();
    });
  });
});
