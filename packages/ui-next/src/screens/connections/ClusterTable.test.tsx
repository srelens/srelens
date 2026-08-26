import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ClusterContext, ClusterFacts } from "@srelens/core";
import type { Probe } from "../../lib/probe";
import { ClusterTable } from "./ClusterTable";

const ctx = (over: Partial<ClusterContext> = {}): ClusterContext => ({
  name: "prod-eu",
  stableId: "prod-eu",
  cluster: "prod",
  server: "https://prod:6443",
  namespace: "",
  isCurrent: false,
  isLocal: false,
  sourceFile: "/home/dana/.kube/config",
  authKind: "exec plugin · gcloud",
  ...over,
});

/** `clusterFacts`'s own shape: provider and region are strings, empty when unread. */
const facts = (over: Partial<ClusterFacts> = {}): ClusterFacts => ({
  context: "prod-eu",
  provider: "gke",
  region: "europe-west4",
  metricsServer: { state: "present", version: "v0.7.1" },
  ...over,
});

describe("ClusterTable", () => {
  it("names the file a kubeconfig context came from", () => {
    render(<ClusterTable rows={[{ context: ctx(), probe: { state: "unread" } }]} onOpen={() => {}} />);
    expect(screen.getByText("/home/dana/.kube/config")).toBeTruthy();
  });

  it("shows no latency for a cluster it has not read", () => {
    render(<ClusterTable rows={[{ context: ctx(), probe: { state: "unread" } }]} onOpen={() => {}} />);
    expect(screen.queryByText(/0\s*ms/)).toBeNull();
  });

  it("shows no latency for a cluster that did not answer", () => {
    render(
      <ClusterTable
        rows={[{ context: ctx(), probe: { state: "unreachable", error: "…" } }]}
        onOpen={() => {}}
      />,
    );
    expect(screen.queryByText(/0\s*ms/)).toBeNull();
    expect(screen.queryByText(/\d+\s*ms/)).toBeNull();
  });

  /**
   * The guard the column needs that the two tests above cannot give it: a
   * probe carrying `latencyMs: 0` ALONGSIDE a state that is not "reachable".
   *
   * `probe.ts` documents latency as absent unless the state is "reachable", so
   * this shape should not exist — but the table is the thing a reader trusts,
   * and a drift in the store (or a future prober that reports a timed-out
   * round trip as its elapsed time) must not become `0 ms` on screen. Gating
   * on the state as well as the number is what makes it structural.
   */
  it("shows no latency for a zero reading on a cluster that did not answer", () => {
    render(
      <ClusterTable
        rows={[{ context: ctx(), probe: { state: "unreachable", latencyMs: 0, error: "…" } }]}
        onOpen={() => {}}
      />,
    );
    expect(screen.queryByText(/0\s*ms/)).toBeNull();
    expect(screen.queryByText(/\d+\s*ms/)).toBeNull();
  });

  it("shows the round trip it timed for a cluster that answered", () => {
    render(
      <ClusterTable
        rows={[{ context: ctx(), probe: { state: "reachable", latencyMs: 12 } }]}
        onOpen={() => {}}
      />,
    );
    expect(screen.getByText("12 ms")).toBeTruthy();
  });

  /**
   * A cluster on this laptop can answer in under half a millisecond. That is a
   * real reading and is not thrown away — but rounding it would print the one
   * string this column may never show, so it is drawn as a bound instead.
   */
  it("draws a sub-millisecond round trip as a bound rather than as zero", () => {
    render(
      <ClusterTable
        rows={[{ context: ctx(), probe: { state: "reachable", latencyMs: 0.4 } }]}
        onOpen={() => {}}
      />,
    );
    expect(screen.getByText("<1 ms")).toBeTruthy();
    expect(screen.queryByText(/\b0\s*ms/)).toBeNull();
  });

  it("opens a row from the keyboard as well as from its button", async () => {
    const onOpen = vi.fn();
    render(<ClusterTable rows={[{ context: ctx(), probe: { state: "unread" } }]} onOpen={onOpen} />);
    await userEvent.dblClick(screen.getByTestId("cluster-name-prod-eu"));
    expect(onOpen).toHaveBeenCalledWith("prod-eu");
  });

  it("never calls a cluster healthy or degraded", () => {
    render(
      <ClusterTable
        rows={[
          { context: ctx(), probe: { state: "reachable", latencyMs: 12 } },
          { context: ctx({ stableId: "b", name: "b" }), probe: { state: "unreachable", error: "…" } },
        ]}
        onOpen={() => {}}
      />,
    );
    expect(screen.queryByText(/healthy|degraded/i)).toBeNull();
  });

  it("says what the probe said, and tones it without a health claim", () => {
    render(
      <ClusterTable
        rows={[
          { context: ctx(), probe: { state: "reachable", latencyMs: 12 } },
          { context: ctx({ stableId: "b", name: "b" }), probe: { state: "unreachable", error: "…" } },
          { context: ctx({ stableId: "c", name: "c" }), probe: { state: "unread" } },
        ]}
        onOpen={() => {}}
      />,
    );
    expect(screen.getByText("reachable").getAttribute("data-tone")).toBe("ok");
    expect(screen.getByText("unreachable").getAttribute("data-tone")).toBe("sev");
    // The absence, named as an absence — not a status word invented for it.
    expect(screen.getByText("no reading").getAttribute("data-tone")).toBe("muted");
  });

  it("never labels a source Team server", () => {
    render(<ClusterTable rows={[{ context: ctx(), probe: { state: "unread" } }]} onOpen={() => {}} />);
    expect(screen.queryByText(/team server/i)).toBeNull();
  });

  it("calls a kubeconfig context's source Kubeconfig and a local cluster's Local", () => {
    render(
      <ClusterTable
        rows={[
          { context: ctx(), probe: { state: "unread" } },
          {
            context: ctx({ stableId: "kind-dev", name: "kind-dev", isLocal: true, provider: "kind" }),
            probe: { state: "unread" },
          },
        ]}
        onOpen={() => {}}
      />,
    );
    expect(screen.getByText("Kubeconfig")).toBeTruthy();
    expect(screen.getByText("Local")).toBeTruthy();
  });

  it("assembles the second line from only the parts it has", () => {
    render(
      <ClusterTable
        rows={[{ context: ctx(), probe: { state: "reachable", latencyMs: 5, version: "v1.29.4" } }]}
        onOpen={() => {}}
      />,
    );
    const line = screen.getByTestId("cluster-detail-prod-eu").textContent ?? "";
    expect(line).toContain("v1.29.4");
    expect(line).not.toMatch(/·\s*·/); // no bare separators for absent parts
  });

  it("joins provider, version and region in that order when it has all three", () => {
    render(
      <ClusterTable
        rows={[
          {
            context: ctx(),
            probe: { state: "reachable", latencyMs: 5, version: "v1.29.4" },
            facts: facts(),
          },
        ]}
        onOpen={() => {}}
      />,
    );
    expect(screen.getByTestId("cluster-detail-prod-eu").textContent).toBe(
      "gke · v1.29.4 · europe-west4",
    );
  });

  it("drops the separator around a part the facts do not carry", () => {
    render(
      <ClusterTable
        rows={[
          {
            context: ctx(),
            probe: { state: "reachable", latencyMs: 5, version: "v1.29.4" },
            // `clusterFacts` normalises an unread fact to "", not to absent.
            facts: facts({ region: "" }),
          },
        ]}
        onOpen={() => {}}
      />,
    );
    expect(screen.getByTestId("cluster-detail-prod-eu").textContent).toBe("gke · v1.29.4");
  });

  it("renders no second line at all when it has none of the three", () => {
    render(<ClusterTable rows={[{ context: ctx(), probe: { state: "unread" } }]} onOpen={() => {}} />);
    // Not an empty line, and not a lone separator: nothing.
    expect(screen.queryByTestId("cluster-detail-prod-eu")).toBeNull();
  });

  it("routes a local cluster through its provider and the host it answers on", () => {
    render(
      <ClusterTable
        rows={[
          {
            context: ctx({
              stableId: "kind-dev",
              name: "kind-dev",
              isLocal: true,
              provider: "kind",
              server: "https://127.0.0.1:6443",
            }),
            probe: { state: "unread" },
          },
        ]}
        onOpen={() => {}}
      />,
    );
    expect(screen.getByText("kind · 127.0.0.1:6443")).toBeTruthy();
    // The kubeconfig path is NOT what a local cluster is reached through.
    expect(screen.queryByText("/home/dana/.kube/config")).toBeNull();
  });

  it("names a local cluster by its host alone when no provider was detected", () => {
    render(
      <ClusterTable
        rows={[
          {
            context: ctx({
              stableId: "local",
              name: "local",
              isLocal: true,
              server: "https://127.0.0.1:6443",
            }),
            probe: { state: "unread" },
          },
        ]}
        onOpen={() => {}}
      />,
    );
    expect(screen.getByText("127.0.0.1:6443")).toBeTruthy();
    expect(screen.queryByText(/^·|·$/)).toBeNull();
  });

  it("falls back to the server as written when it is not a URL", () => {
    render(
      <ClusterTable
        rows={[
          {
            context: ctx({ stableId: "odd", name: "odd", isLocal: true, server: "not-a-url" }),
            probe: { state: "unread" },
          },
        ]}
        onOpen={() => {}}
      />,
    );
    expect(screen.getByText("not-a-url")).toBeTruthy();
  });

  it("names the credential mechanism the context uses", () => {
    render(<ClusterTable rows={[{ context: ctx(), probe: { state: "unread" } }]} onOpen={() => {}} />);
    expect(screen.getByText("exec plugin · gcloud")).toBeTruthy();
  });

  /**
   * §6 groups `team` → `file` → `local`, and with the team server out of scope
   * that is kubeconfig before local — the order Pane 2 lists its sections in
   * too. The requirement is that the two groups do not interleave; the order
   * between them follows the design (see `bySource`).
   */
  it("groups kubeconfig contexts ahead of local clusters, keeping each group's order", () => {
    render(
      <ClusterTable
        rows={[
          { context: ctx({ stableId: "prod-eu", name: "prod-eu" }), probe: { state: "unread" } },
          {
            context: ctx({ stableId: "kind-dev", name: "kind-dev", isLocal: true, provider: "kind" }),
            probe: { state: "unread" },
          },
          { context: ctx({ stableId: "staging", name: "staging" }), probe: { state: "unread" } },
          {
            context: ctx({ stableId: "k3d-lab", name: "k3d-lab", isLocal: true, provider: "k3d" }),
            probe: { state: "unread" },
          },
        ]}
        onOpen={() => {}}
      />,
    );
    const names = screen.getAllByTestId(/^cluster-name-/).map((el) => el.textContent);
    expect(names).toEqual(["prod-eu", "staging", "kind-dev", "k3d-lab"]);
  });

  it("opens the cluster the row belongs to", async () => {
    const onOpen = vi.fn();
    render(<ClusterTable rows={[{ context: ctx(), probe: { state: "unread" } }]} onOpen={onOpen} />);
    await userEvent.click(screen.getByRole("button", { name: "Open" }));
    expect(onOpen).toHaveBeenCalledWith("prod-eu");
  });

  it("opens the row that was clicked, not the first one", async () => {
    const onOpen = vi.fn();
    render(
      <ClusterTable
        rows={[
          { context: ctx(), probe: { state: "unread" } },
          { context: ctx({ stableId: "staging#name", name: "staging" }), probe: { state: "unread" } },
        ]}
        onOpen={onOpen}
      />,
    );
    await userEvent.click(screen.getAllByRole("button", { name: "Open" })[1]);
    // The stableId, which is `{file}#{name}` in production — never the name.
    expect(onOpen).toHaveBeenCalledWith("staging#name");
  });

  /**
   * **The `min-width: auto` guard, asserted on the classes because jsdom
   * cannot see it.** Eight defects on this migration. `Via` holds a full
   * filesystem path and `Cluster` a display name, both beside §6's fixed 292px
   * rail: without a cap the cell's intrinsic width is the whole string, and
   * a flex item's implicit `min-width: auto` refuses to shrink below it, so
   * the rail is pushed off the window instead. `block` is what makes
   * `truncate`'s `overflow: hidden` apply at all — it does nothing on an
   * inline box.
   */
  it("caps and truncates the two cells that hold unbounded text", () => {
    render(
      <ClusterTable
        rows={[
          {
            context: ctx({
              sourceFile: "/Users/dana/Library/Application Support/srelens/kubeconfigs/acme-prod.yaml",
            }),
            probe: { state: "unread" },
          },
        ]}
        onOpen={() => {}}
      />,
    );

    const via = screen.getByTestId("cluster-via-prod-eu");
    expect(via.className).toContain("block");
    expect(via.className).toMatch(/max-w-\[\d+px\]/);
    expect(via.className).toContain("truncate");

    const name = screen.getByTestId("cluster-name-prod-eu");
    expect(name.className).toContain("block");
    expect(name.className).toMatch(/max-w-\[\d+px\]/);
    expect(name.className).toContain("truncate");

    // The flex row that holds the mark beside the name, and the text column
    // inside it: both need `min-w-0` or the truncation above never engages.
    const cell = screen.getByTestId("cluster-cell-prod-eu");
    expect(cell.className).toContain("min-w-0");
    expect(name.parentElement?.className).toContain("min-w-0");
    // The mark is the one thing in the row that may not be squeezed away.
    expect(cell.firstElementChild?.className).toContain("shrink-0");
  });

  it("keeps its own frame shrinkable, and takes the caller's classes", () => {
    render(
      <ClusterTable
        rows={[{ context: ctx(), probe: { state: "unread" } }]}
        onOpen={() => {}}
        className="flex-1"
      />,
    );
    const frame = screen.getByTestId("cluster-table");
    expect(frame.className).toContain("min-w-0");
    expect(frame.className).toContain("flex-1");
  });
});
