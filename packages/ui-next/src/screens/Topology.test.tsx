import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { TopologyGraph } from "@srelens/core";

/**
 * The graph is supplied at core's boundary, which is the only thing this
 * screen reads. `layoutGraph` stays REAL: where a node lands is the half of
 * this screen that can be wrong, and a test mocking the layout would assert
 * that the screen calls a stub rather than that a reader sees a Deployment
 * beside the Service that fronts it.
 */
const core = vi.hoisted(() => ({ topologyGraph: vi.fn(), prometheusDiscover: vi.fn() }));
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  topologyGraph: core.topologyGraph,
  prometheusDiscover: core.prometheusDiscover,
}));

/**
 * The namespace list comes from the app's shared hook, the same one the
 * resource lists and the events screen read, so it is supplied at that seam
 * rather than at core's listNamespaces.
 */
const options = vi.hoisted(() => ({ namespaces: [] as string[] | null, error: "" }));
vi.mock("@srelens/core/react", async (orig) => ({
  ...(await orig<typeof import("@srelens/core/react")>()),
  useNamespaceOptions: () => ({ namespaces: options.namespaces, scope: "", error: options.error }),
}));

/**
 * The workspace store is the reader's namespace FILTER, not the list of
 * namespaces a cluster has — an empty entry there means "all namespaces", and
 * a cluster only appears in it once something narrows it. The screen read it
 * as the list once and shipped a picker with no options against a healthy
 * cluster, so both facts are supplied separately here.
 */
const workspace = vi.hoisted(() => ({ scoped: [] as string[], setNamespaces: vi.fn() }));
vi.mock("../lib/workspace", async (orig) => ({
  ...(await orig<typeof import("../lib/workspace")>()),
  useNamespaces: () => workspace.scoped,
  setNamespaces: workspace.setNamespaces,
}));

vi.mock("../lib/clusters", async (orig) => ({
  ...(await orig<typeof import("../lib/clusters")>()),
  useActiveContext: () => ({ name: "prod-eu", stableId: "prod-eu" }),
}));

import { Topology } from "./Topology";

// jsdom has no ResizeObserver, and the namespace picker is a Radix popper,
// which watches its trigger and content with one. Inert here: jsdom does no
// layout, so there is never a resize to report. The kit's own popper tests
// carry the identical stub.
if (!("ResizeObserver" in globalThis)) {
  (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}
// The same four jsdom is missing that `MultiSelect.test.tsx` stubs, for the
// same control and the same reason.
const proto = window.HTMLElement.prototype as unknown as Record<string, unknown>;
proto.scrollIntoView ??= () => {};
proto.hasPointerCapture ??= () => false;
proto.setPointerCapture ??= () => {};
proto.releasePointerCapture ??= () => {};

function graph(): TopologyGraph {
  return {
    nodes: [
      {
        id: "Ingress/checkout/web",
        kind: "Ingress",
        name: "web",
        namespace: "checkout",
        lane: "route",
        detail: "checkout.acme.io",
        ready: null,
        desired: null,
        health: "unknown",
      },
      {
        id: "Service/checkout/checkout-api",
        kind: "Service",
        name: "checkout-api",
        namespace: "checkout",
        lane: "service",
        detail: ":80",
        ready: null,
        desired: null,
        health: "degraded",
      },
      {
        id: "Deployment/checkout/checkout-api",
        kind: "Deployment",
        name: "checkout-api",
        namespace: "checkout",
        lane: "workload",
        detail: "9/12",
        ready: 9,
        desired: 12,
        health: "degraded",
      },
    ],
    edges: [
      {
        from: "Ingress/checkout/web",
        to: "Service/checkout/checkout-api",
        kind: "routes",
        provenance: "topology",
        detail: "",
        health: "degraded",
        weight: null,
        unit: null,
      },
      {
        from: "Service/checkout/checkout-api",
        to: "Deployment/checkout/checkout-api",
        kind: "routes",
        provenance: "topology",
        detail: "",
        health: "degraded",
        weight: null,
        unit: null,
      },
    ],
  };
}

beforeEach(() => {
  workspace.scoped = [];
  workspace.setNamespaces.mockReset();
  options.namespaces = ["checkout", "default", "payments"];
  options.error = "";
  core.topologyGraph.mockReset().mockResolvedValue({ graph: graph() });
  // Most clusters run no metrics backend; that is the ordinary case.
  core.prometheusDiscover.mockReset().mockResolvedValue({ candidates: [] });
});

describe("Topology", () => {
  it("draws every node the namespace returned", async () => {
    render(<Topology />);
    // Each node is a control, because selecting one is an action and the graph
    // has to be reachable without a mouse.
    expect(await screen.findByRole("button", { name: "Ingress web" })).toBeDefined();
    expect(screen.getByRole("button", { name: "Service checkout-api" })).toBeDefined();
    expect(screen.getByRole("button", { name: "Deployment checkout-api" })).toBeDefined();
  });

  it("offers every namespace the cluster has, through the app's own picker", async () => {
    // This screen first grew its own listNamespaces call and its own Select,
    // which made it the one place the namespace control behaved differently.
    render(<Topology />);
    await userEvent.click(await screen.findByRole("combobox", { name: "Namespaces" }));
    for (const ns of ["checkout", "default", "payments"]) {
      expect(screen.getByRole("option", { name: new RegExp(`^${ns}`) })).toBeDefined();
    }
    // The picker's own "All namespaces" row is part of the shared control.
    expect(screen.getByRole("option", { name: /All namespaces/ })).toBeDefined();
  });

  it("draws every namespace when the reader has narrowed nothing", async () => {
    // An empty selection is the shared picker's "All namespaces", and this
    // screen honours it rather than quietly picking one for them.
    render(<Topology />);
    await waitFor(() =>
      expect(core.topologyGraph).toHaveBeenCalledWith("prod-eu", ["checkout", "default", "payments"], [], false),
    );
  });

  it("opens on the reader's standing scope ahead of `default`", async () => {
    // The filter store is a standing choice about what they want to see, so a
    // reader who narrowed to `payments` elsewhere lands there here too.
    workspace.scoped = ["payments"];
    render(<Topology />);
    await waitFor(() => expect(core.topologyGraph).toHaveBeenCalledWith("prod-eu", ["payments"], [], false));
  });

  it("caps `All namespaces` on a big cluster, and says that it did", async () => {
    // A graph of every namespace on a real cluster is a picture of nothing,
    // and the capability makes seven list calls per namespace to build it.
    // Cutting silently would leave the picker reading "All namespaces" over a
    // graph that is not all of them.
    options.namespaces = Array.from({ length: 20 }, (_, i) => `ns-${i}`);
    render(<Topology />);
    await waitFor(() => expect(core.topologyGraph).toHaveBeenCalled());
    expect(core.topologyGraph.mock.calls[0][1]).toHaveLength(12);
    expect(await screen.findByText(/Showing 12 of 20 namespaces/)).toBeDefined();
  });

  it("reports a failed namespace listing the way every other screen does", async () => {
    // The shared alert rather than a sentence of this screen's own, so a 403
    // on namespaces reads identically here and on the resource lists.
    options.error = "connection refused";
    render(<Topology />);
    expect(await screen.findByText(/Namespaces could not be listed/)).toBeDefined();
  });

  it("draws a declared dependency differently from a structural edge", async () => {
    // The whole point of provenance: a host found in an environment variable
    // and a Service selector join must not render alike, or a reader trusts
    // them equally.
    core.topologyGraph.mockResolvedValue({
      graph: {
        nodes: [
          {
            id: "Deployment/default/checkout",
            kind: "Deployment",
            name: "checkout",
            namespace: "default",
            lane: "workload",
            detail: "1/1",
            ready: 1,
            desired: 1,
            health: "ok",
          },
          {
            id: "External/default/db.example.com",
            kind: "External",
            name: "db.example.com",
            namespace: "default",
            lane: "external",
            detail: "",
            ready: null,
            desired: null,
            health: "unknown",
          },
        ],
        edges: [
          {
            from: "Deployment/default/checkout",
            to: "External/default/db.example.com",
            kind: "calls",
            provenance: "declared",
            detail: "",
            health: "unknown",
            weight: null,
            unit: null,
          },
        ],
      },
    });
    render(<Topology />);

    const external = await screen.findByRole("button", { name: "External db.example.com" });
    expect(external).toBeDefined();
    // Scoped to the canvas: the legend draws the same dotted stroke as its own
    // sample, which is the point of the legend and not an edge.
    const canvas = screen.getByRole("group", { name: "Namespace topology" });
    expect(canvas.querySelectorAll('path[stroke-dasharray="2 4"]')).toHaveLength(1);
  });

  it("lanes the namespaces apart, and marks the call that crosses between them", async () => {
    core.topologyGraph.mockResolvedValue({
      graph: {
        nodes: [
          ...graph().nodes,
          {
            id: "Service/payments/payments-api",
            kind: "Service",
            name: "payments-api",
            namespace: "payments",
            lane: "service",
            detail: ":443",
            ready: null,
            desired: null,
            health: "ok",
          },
        ],
        edges: [
          ...graph().edges,
          {
            from: "Deployment/checkout/checkout-api",
            to: "Service/payments/payments-api",
            kind: "calls",
            provenance: "declared",
            detail: "",
            health: "unknown",
            weight: null,
            unit: null,
          },
        ],
      },
    });
    render(<Topology />);
    await screen.findByRole("button", { name: "Service payments-api" });
    const canvas = screen.getByRole("group", { name: "Namespace topology" });
    expect([...canvas.querySelectorAll("[data-lane]")].map((t) => t.textContent)).toEqual([
      "checkout",
      "payments",
    ]);
    expect(canvas.querySelectorAll("[data-crossing]")).toHaveLength(1);

    // The inspector says which namespace the other end is in, only because
    // it differs.
    await userEvent.click(screen.getByRole("button", { name: "Deployment checkout-api" }));
    const panel = screen.getByRole("complementary", { name: "Selected node" });
    expect(within(panel).getByRole("button", { name: /payments-api · payments/ })).toBeDefined();
    expect(within(panel).getByRole("button", { name: /^checkout-api routes/ })).toBeDefined();
  });

  it("withholds a hub's callers until a selection puts them on a path", async () => {
    // Four external hosts each named by forty tiers drew a hundred and sixty
    // curves across the whole canvas. The number is the fact; the lines are
    // detail, and detail is what selecting is for.
    const callers = Array.from({ length: 9 }, (_, i) => `svc-${i}`);
    core.topologyGraph.mockResolvedValue({
      graph: {
        nodes: [
          {
            id: "External//api.example.com",
            kind: "External",
            name: "api.example.com",
            namespace: "",
            lane: "external",
            detail: "",
            ready: null,
            desired: null,
            health: "unknown",
          },
          ...callers.map((name) => ({
            id: `Deployment/default/${name}`,
            kind: "Deployment",
            name,
            namespace: "default",
            lane: "workload" as const,
            detail: "1/1",
            ready: 1,
            desired: 1,
            health: "ok" as const,
          })),
        ],
        edges: callers.map((name) => ({
          from: `Deployment/default/${name}`,
          to: "External//api.example.com",
          kind: "calls" as const,
          provenance: "declared" as const,
          detail: "",
          health: "unknown" as const,
          weight: null,
          unit: null,
        })),
      },
    });
    render(<Topology />);
    const hub = await screen.findByRole("button", { name: "External api.example.com" });
    const canvas = screen.getByRole("group", { name: "Namespace topology" });
    expect(canvas.querySelectorAll("path").length).toBe(0);
    expect([...hub.querySelectorAll("text")].map((t) => t.textContent)).toContain("9 callers");

    await userEvent.click(screen.getByRole("button", { name: "Deployment svc-3" }));
    expect(canvas.querySelectorAll("path")).toHaveLength(1);
    await userEvent.click(hub);
    expect(canvas.querySelectorAll("path")).toHaveLength(9);
  });

  it("draws no lane when one namespace is all there is", async () => {
    render(<Topology />);
    await screen.findByRole("button", { name: "Ingress web" });
    const canvas = screen.getByRole("group", { name: "Namespace topology" });
    expect(canvas.querySelectorAll("[data-lane]")).toHaveLength(0);
    expect(canvas.querySelectorAll("[data-crossing]")).toHaveLength(0);
  });

  it("draws a policy-allowed call as neither traffic nor config", async () => {
    // A NetworkPolicy says the call would be let through if made. That is
    // weaker than a host in an env var and much weaker than a measurement,
    // and the line and the inspector both have to say so.
    core.topologyGraph.mockResolvedValue({
      graph: {
        nodes: [
          {
            id: "Deployment/default/checkout",
            kind: "Deployment",
            name: "checkout",
            namespace: "default",
            lane: "workload",
            detail: "1/1",
            ready: 1,
            desired: 1,
            health: "ok",
          },
          {
            id: "Service/default/payments",
            kind: "Service",
            name: "payments",
            namespace: "default",
            lane: "service",
            detail: ":80",
            ready: null,
            desired: null,
            health: "ok",
          },
        ],
        edges: [
          {
            from: "Deployment/default/checkout",
            to: "Service/default/payments",
            kind: "calls",
            provenance: "allowed",
            detail: "",
            health: "unknown",
            weight: null,
            unit: null,
          },
        ],
      },
    });
    render(<Topology />);
    const caller = await screen.findByRole("button", { name: "Deployment checkout" });
    const canvas = screen.getByRole("group", { name: "Namespace topology" });
    expect(canvas.querySelectorAll('path[stroke-dasharray="7 4"]')).toHaveLength(1);
    expect(canvas.querySelectorAll('path[stroke-dasharray="2 4"]')).toHaveLength(0);

    await userEvent.click(caller);
    const panel = screen.getByRole("complementary", { name: "Selected node" });
    expect(within(panel).getByRole("button", { name: /payments allowed/ })).toBeDefined();
  });

  it("feeds a discovered metrics backend to the graph, and draws its rates", async () => {
    // A measured edge is accented and solid where a declared one is faint and
    // dotted, and it is the only kind that carries a number.
    core.prometheusDiscover.mockResolvedValue({
      candidates: [{ namespace: "monitoring", service: "prometheus", port: 9090, flavour: "prometheus" }],
    });
    core.topologyGraph.mockResolvedValue({
      graph: {
        nodes: [
          {
            id: "Deployment/default/storefront",
            kind: "Deployment",
            name: "storefront",
            namespace: "default",
            lane: "workload",
            detail: "1/1",
            ready: 1,
            desired: 1,
            health: "ok",
          },
          {
            id: "Service/default/checkout",
            kind: "Service",
            name: "checkout",
            namespace: "default",
            lane: "service",
            detail: ":80",
            ready: null,
            desired: null,
            health: "ok",
          },
        ],
        edges: [
          {
            from: "Deployment/default/storefront",
            to: "Service/default/checkout",
            kind: "calls",
            provenance: "observed",
            detail: "41 rpm",
            health: "unknown",
            weight: 0.6867,
            unit: "rps",
          },
        ],
      },
    });
    render(<Topology />);

    await waitFor(() =>
      expect(core.topologyGraph).toHaveBeenCalledWith("prod-eu", expect.any(Array), [{
        namespace: "monitoring",
        service: "prometheus",
        port: 9090,
        flavour: "prometheus",
      }], false),
    );
    expect(await screen.findByText("41 rpm")).toBeDefined();
    const canvas = screen.getByRole("group", { name: "Namespace topology" });
    // Not the dotted line that says "someone wrote this in a config file". A
    // measured edge is accented and it MOVES — the dashes and their animation
    // both come from the kit's `.flow`, which is also what a reader who has
    // asked for less motion gets held still.
    expect(canvas.querySelectorAll("path[stroke-dasharray]")).toHaveLength(0);
    expect(canvas.querySelectorAll("path.flow")).toHaveLength(1);
  });

  it("draws a measured edge as thick as the rate is large", async () => {
    // Writing `41 rpm` beside a line is a number a reader has to stop and
    // read. Thickness is the same fact at a glance, and it is why the
    // capability sends the quantity beside the label rather than leaving the
    // screen to parse `41 rpm` back into one.
    core.prometheusDiscover.mockResolvedValue({
      candidates: [{ namespace: "monitoring", service: "prometheus", port: 9090, flavour: "prometheus" }],
    });
    const workload = (name: string) => ({
      id: `Deployment/default/${name}`,
      kind: "Deployment",
      name,
      namespace: "default",
      lane: "workload" as const,
      detail: "1/1",
      ready: 1,
      desired: 1,
      health: "ok" as const,
    });
    const service = (name: string) => ({
      id: `Service/default/${name}`,
      kind: "Service",
      name,
      namespace: "default",
      lane: "service" as const,
      detail: ":80",
      ready: null,
      desired: null,
      health: "ok" as const,
    });
    core.topologyGraph.mockResolvedValue({
      graph: {
        nodes: [workload("storefront"), service("busy"), service("quiet")],
        edges: [
          {
            from: "Deployment/default/storefront",
            to: "Service/default/busy",
            kind: "calls",
            provenance: "observed",
            detail: "600 rpm",
            health: "unknown",
            weight: 10,
            unit: "rps",
          },
          {
            from: "Deployment/default/storefront",
            to: "Service/default/quiet",
            kind: "calls",
            provenance: "observed",
            detail: "6 rpm",
            health: "unknown",
            weight: 0.1,
            unit: "rps",
          },
        ],
      },
    });
    render(<Topology />);

    await screen.findByText("600 rpm");
    const canvas = screen.getByRole("group", { name: "Namespace topology" });
    const widths = [...canvas.querySelectorAll("path.flow")].map((p) =>
      Number(p.getAttribute("stroke-width")),
    );
    expect(widths).toHaveLength(2);
    expect(Math.max(...widths)).toBeGreaterThan(Math.min(...widths));
  });

  it("probes connections only when asked, and says what it costs", async () => {
    // One `kubectl exec` per pod, and an audit-log entry on each. Nothing turns
    // that on for a reader.
    render(<Topology />);
    await waitFor(() =>
      expect(core.topologyGraph).toHaveBeenCalledWith("prod-eu", expect.any(Array), [], false),
    );

    const toggle = screen.getByRole("button", { name: "Probe connections" });
    expect(toggle.getAttribute("title")).toContain("pods/exec");
    await userEvent.click(toggle);

    await waitFor(() =>
      expect(core.topologyGraph).toHaveBeenCalledWith("prod-eu", expect.any(Array), [], true),
    );
    expect(screen.getByRole("button", { name: "Probing connections" })).toBeDefined();
  });

  it("says in words that an external node is config, not observed traffic", async () => {
    // A dotted line is not enough on its own — the panel must not let a reader
    // believe srelens watched a byte go to this host.
    core.topologyGraph.mockResolvedValue({
      graph: {
        nodes: [
          {
            id: "External/default/db.example.com",
            kind: "External",
            name: "db.example.com",
            namespace: "default",
            lane: "external",
            detail: "",
            ready: null,
            desired: null,
            health: "unknown",
          },
        ],
        edges: [],
      },
    });
    render(<Topology />);
    await userEvent.click(await screen.findByRole("button", { name: "External db.example.com" }));
    expect(
      screen.getByRole("complementary", { name: "Selected node" }).textContent,
    ).toContain("has not observed traffic");
  });

  it("lists what a node reaches, and walks the graph from there", async () => {
    // The panel is how you follow a dependency when the canvas has crossings in
    // it. Reading an edge off a tangle is hard; following a list is not.
    render(<Topology />);
    await userEvent.click(await screen.findByRole("button", { name: "Service checkout-api" }));

    const panel = screen.getByRole("complementary", { name: "Selected node" });
    expect(within(panel).getByText("Reaches")).toBeDefined();
    expect(within(panel).getByText("Reached by")).toBeDefined();

    // And the rows are live: clicking one moves the selection to that node.
    await userEvent.click(within(panel).getByRole("button", { name: /checkout-api routes/ }));
    expect(
      screen.getByRole("complementary", { name: "Selected node" }).textContent,
    ).toContain("Deployment");
  });

  it("traces the whole chain from a selection, not one hop from it", async () => {
    // "If this is broken, what else is" is a transitive question. The version
    // this replaces lit up immediate neighbours and faded the rest, which
    // dimmed the far end of the very path a reader was following.
    core.topologyGraph.mockResolvedValue({
      graph: {
        nodes: [
          ...graph().nodes,
          {
            id: "Service/checkout/unrelated",
            kind: "Service",
            name: "unrelated",
            namespace: "checkout",
            lane: "service",
            detail: ":80",
            ready: null,
            desired: null,
            health: "ok",
          },
        ],
        edges: graph().edges,
      },
    });
    render(<Topology />);
    await userEvent.click(await screen.findByRole("button", { name: "Ingress web" }));

    const canvas = screen.getByRole("group", { name: "Namespace topology" });
    const faded = [...canvas.querySelectorAll("g[opacity='0.13'][aria-label]")];
    // The Deployment is two hops downstream and stays bright with the rest of
    // the path. Only the Service on no path at all goes quiet.
    expect(faded.map((g) => g.getAttribute("aria-label"))).toEqual(["Service unrelated"]);
  });

  it("counts what a node takes with it, both ways", async () => {
    render(<Topology />);
    await userEvent.click(await screen.findByRole("button", { name: "Ingress web" }));

    const panel = screen.getByRole("complementary", { name: "Selected node" });
    expect(panel.textContent).toContain("Entry point");
    // The Service and the Deployment behind it, not just the Service.
    expect(panel.textContent).toContain("2 nodes");
    expect(panel.textContent).toContain("0 nodes");
  });

  it("lays a call chain out left to right, by hops rather than by kind", async () => {
    // The argument this whole layout makes. `payments`' Service is called BY a
    // Deployment, so it belongs after it; the old kind-per-column layout put
    // every Service in column two and drew the call backwards.
    core.topologyGraph.mockResolvedValue({
      graph: {
        nodes: [
          {
            id: "Deployment/checkout/checkout",
            kind: "Deployment",
            name: "checkout",
            namespace: "checkout",
            lane: "workload",
            detail: "1/1",
            ready: 1,
            desired: 1,
            health: "ok",
          },
          {
            id: "Service/payments/payments",
            kind: "Service",
            name: "payments",
            namespace: "payments",
            lane: "service",
            detail: ":80",
            ready: null,
            desired: null,
            health: "ok",
          },
        ],
        edges: [
          {
            from: "Deployment/checkout/checkout",
            to: "Service/payments/payments",
            kind: "calls",
            provenance: "declared",
            detail: "",
            health: "unknown",
            weight: null,
            unit: null,
          },
        ],
      },
    });
    render(<Topology />);

    const caller = await screen.findByRole("button", { name: "Deployment checkout" });
    const called = screen.getByRole("button", { name: "Service payments" });
    const x = (node: Element) => Number(node.querySelector("rect")?.getAttribute("x"));
    expect(x(called)).toBeGreaterThan(x(caller));
    // And the arrow says which way it goes, which the version without
    // arrowheads left the reader to infer from the columns.
    const canvas = screen.getByRole("group", { name: "Namespace topology" });
    expect(canvas.querySelectorAll("polygon")).toHaveLength(1);
  });

  it("folds a ReplicaSet into the workload that owns it", async () => {
    // A ReplicaSet is not a place traffic goes, so it is a chip on the
    // workload rather than a dead-end box on every path.
    core.topologyGraph.mockResolvedValue({
      graph: {
        nodes: [
          ...graph().nodes,
          {
            id: "ReplicaSet/checkout/checkout-api-7d9",
            kind: "ReplicaSet",
            name: "rev 119",
            namespace: "checkout",
            lane: "replicaset",
            detail: "9/12 ready",
            ready: 9,
            desired: 12,
            health: "degraded",
          },
        ],
        edges: [
          ...graph().edges,
          {
            from: "Deployment/checkout/checkout-api",
            to: "ReplicaSet/checkout/checkout-api-7d9",
            kind: "owns",
            provenance: "topology",
            detail: "",
            health: "degraded",
            weight: null,
            unit: null,
          },
        ],
      },
    });
    render(<Topology />);

    const workload = await screen.findByRole("button", { name: "Deployment checkout-api" });
    expect(screen.queryByRole("button", { name: "ReplicaSet rev 119" })).toBeNull();
    // The revision takes the node's top-right slot, which on a graph inside
    // one namespace is going spare. On a graph spanning several the namespace
    // wins it: the two ran into each other when both were drawn there.
    expect([...workload.querySelectorAll("text")].map((t) => t.textContent)).toContain("rev 119");

    await userEvent.click(workload);
    expect(
      screen.getByRole("complementary", { name: "Selected node" }).textContent,
    ).toContain("Revisions");
  });

  it("draws replicas as squares, so `9/12` needs no arithmetic", async () => {
    render(<Topology />);
    const workload = await screen.findByRole("button", { name: "Deployment checkout-api" });
    // Twelve squares: the node's own outline, the health spine, and one per
    // desired replica.
    const pips = [...workload.querySelectorAll("rect")].filter(
      (rect) => rect.getAttribute("width") === "6",
    );
    expect(pips).toHaveLength(12);
    expect(pips.filter((p) => p.getAttribute("class")?.includes("fill-rule"))).toHaveLength(3);
  });

  it("pans and zooms by moving one composited layer, not by re-rendering the graph", async () => {
    // On a few hundred nodes, a wheel tick that went through React state and
    // an SVG transform re-rendered the component and re-rasterised every
    // path. The view is now a CSS transform on the wrapper, written by hand.
    render(<Topology />);
    const canvas = await screen.findByRole("group", { name: "Namespace topology" });
    const layer = canvas.parentElement as HTMLElement;
    const frame = layer.parentElement as HTMLElement;
    const before = layer.style.transform;

    frame.dispatchEvent(new WheelEvent("wheel", { deltaY: -240, bubbles: true, cancelable: true }));
    expect(layer.style.transform).not.toBe(before);
    expect(layer.style.transform).toMatch(/scale\(1\.\d+\)/);
    // The graph itself carries no transform of its own any more.
    expect(canvas.getAttribute("transform")).toBeNull();
    expect(canvas.querySelector("g[transform]")).toBeNull();
  });

  it("opens showing the whole graph, and can be zoomed off it", async () => {
    // A production namespace does not fit at 1:1, and the version this
    // replaces gave a reader no way to see the shape of one — only to scroll
    // around inside it.
    render(<Topology />);
    await screen.findByRole("button", { name: "Ingress web" });
    expect(screen.getByRole("button", { name: "Zoom in" })).toBeDefined();
    await userEvent.click(screen.getByRole("button", { name: "Zoom in" }));
    expect(screen.getByText("125%")).toBeDefined();
    await userEvent.click(screen.getByRole("button", { name: "Fit" }));
    // jsdom measures every box as zero, so Fit can only answer 1:1 here.
    expect(screen.getByText("100%")).toBeDefined();
  });

  it("cuts a long name to the box, and keeps the whole of it reachable", async () => {
    // SVG text neither wraps nor clips, so a long host was drawn straight over
    // the next column.
    core.topologyGraph.mockResolvedValue({
      graph: {
        nodes: [
          {
            id: "External/default/orders-db.internal.example.com",
            kind: "External",
            name: "orders-db.internal.example.com",
            namespace: "default",
            lane: "external",
            detail: "",
            ready: null,
            desired: null,
            health: "unknown",
          },
        ],
        edges: [],
      },
    });
    render(<Topology />);
    const node = await screen.findByRole("button", { name: "External orders-db.internal.example.com" });
    // The DRAWN name is cut — `textContent` would also pick up the title below.
    const drawn = [...node.querySelectorAll("text")].map((t) => t.textContent);
    expect(drawn).toContain("orders-db.internal.exam…");
    expect(drawn).not.toContain("orders-db.internal.example.com");
    // The full name is still there for a pointer, and the panel never cuts it.
    expect(node.querySelector("title")?.textContent).toContain("orders-db.internal.example.com");
  });

  it("shows a node in the inspector once it is chosen", async () => {
    render(<Topology />);
    await userEvent.click(await screen.findByRole("button", { name: "Deployment checkout-api" }));

    const panel = screen.getByRole("complementary", { name: "Selected node" });
    expect(panel.textContent).toContain("Deployment");
    expect(panel.textContent).toContain("9 / 12");
    expect(panel.textContent).toContain("degraded");
  });

  it("says the namespaces are empty rather than sitting on a blank canvas", async () => {
    // An empty namespace is a real answer, not a failure to read one.
    options.namespaces = ["solo"];
    core.topologyGraph.mockResolvedValue({ graph: { nodes: [], edges: [] } });
    render(<Topology />);
    expect(await screen.findByText("Nothing to draw in solo")).toBeDefined();
  });

  it("surfaces a refusal as an error instead of an empty namespace", async () => {
    // The two must never look alike: "we could not ask" is not "there is
    // nothing there".
    core.topologyGraph.mockResolvedValue({ error: "namespaces \"checkout\" is forbidden" });
    render(<Topology />);
    expect(await screen.findByText(/Could not draw this topology/)).toBeDefined();
    expect(screen.queryByText(/Nothing to draw/)).toBeNull();
  });

  it("writes the namespace choice to the workspace, not to itself", async () => {
    // The selection is shared: narrowing here narrows the resource lists and
    // the events screen too, and a picker with private state would have made
    // this the one screen where the choice did not travel.
    render(<Topology />);
    await userEvent.click(await screen.findByRole("button", { name: "Deployment checkout-api" }));
    expect(
      screen.getByRole("complementary", { name: "Selected node" }).textContent,
    ).toContain("9 / 12");

    await userEvent.click(screen.getByRole("combobox", { name: "Namespaces" }));
    await userEvent.click(screen.getByRole("option", { name: /^payments/ }));
    expect(workspace.setNamespaces).toHaveBeenCalledWith("prod-eu", ["payments"]);

    // The old selection may name a node in a namespace no longer drawn, so the
    // panel lets go of it rather than describing something off screen.
    await waitFor(() =>
      expect(
        screen.getByRole("complementary", { name: "Selected node" }).textContent,
      ).toContain("No node selected"),
    );
  });

  it("draws nothing, and asks for nothing, on a cluster with no namespaces", async () => {
    options.namespaces = [];
    render(<Topology />);
    expect(await screen.findByText(/Nothing to draw/)).toBeDefined();
    expect(core.topologyGraph).not.toHaveBeenCalled();
  });
});
