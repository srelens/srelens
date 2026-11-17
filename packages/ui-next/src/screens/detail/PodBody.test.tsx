import { afterEach, beforeEach, describe, it, expect, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ClusterContext, K8sObject } from "@srelens/core";
import { KV } from "@srelens/ui-kit";

// What §A.4's dialog reaches for once a container's port opens it. This file
// has no cluster; every formatter (`portText`, `containerStateText`, …) and
// both forward rules (`toKubectl`, `kindToForwardTarget`) stay REAL, since
// "a Pod is `pod/`, not `svc/`" is one of the things under test.
const forwardCore = vi.hoisted(() => ({
  listNamespaces: vi.fn(async () => ({ namespaces: ["default"] })),
  listServices: vi.fn(async () => ({ services: [] })),
  listPods: vi.fn(async () => ({ pods: [{ name: "web-1", namespace: "default" }] })),
  startPortForward: vi.fn(async () => ({ id: 1, localPort: 9090 })),
}));
vi.mock("@srelens/core", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@srelens/core")>()),
  ...forwardCore,
}));

import { resetContexts, setContexts } from "../../lib/clusters";
import { detailRoute } from "../../lib/detailRoute";
import { defaultState } from "../../lib/tabs";
import * as store from "../../lib/tabsStore";
import { Section } from "./Section";
import { PodContainersBody, PodContainersTable, PodDetailsBody, podFacts } from "./PodBody";

const APP_CONTAINER = {
  name: "app",
  image: "ghcr.io/example/app:1.2.3",
  ports: [{ name: "http", containerPort: 8080, protocol: "TCP" }],
  env: [{ name: "LOG_LEVEL", value: "info" }],
  volumeMounts: [{ name: "cache", mountPath: "/var/cache" }],
  livenessProbe: { httpGet: { path: "/healthz", port: 8080 }, periodSeconds: 10 },
  readinessProbe: { httpGet: { path: "/ready", port: 8080 } },
};

const APP_STATUS = {
  name: "app",
  ready: true,
  restartCount: 3,
  state: { running: { startedAt: "2026-08-20T00:00:00Z" } },
};

const SIDECAR_CONTAINER = {
  name: "sidecar",
  image: "ghcr.io/example/sidecar:1.0",
};

const SIDECAR_STATUS = {
  name: "sidecar",
  ready: true,
  restartCount: 0,
  state: { running: { startedAt: "2026-08-20T00:00:00Z" } },
};

function pod(
  spec: Record<string, unknown>,
  status: Record<string, unknown> = {},
  metadata: NonNullable<K8sObject["metadata"]> = { name: "web-1", namespace: "default" },
): K8sObject {
  return {
    kind: "Pod",
    apiVersion: "v1",
    metadata,
    spec,
    status,
  } as K8sObject;
}

describe("PodContainersBody", () => {
  it("names every container", () => {
    render(
      <PodContainersBody
        context="ctx"
        object={pod(
          { containers: [APP_CONTAINER, SIDECAR_CONTAINER] },
          { containerStatuses: [APP_STATUS, SIDECAR_STATUS] },
        )}
      />,
    );
    expect(screen.getByText("app")).toBeDefined();
    expect(screen.getByText("sidecar")).toBeDefined();
  });

  it("shows a container's state and its restart count", () => {
    render(
      <PodContainersBody
        context="ctx"
        object={pod({ containers: [APP_CONTAINER] }, { containerStatuses: [APP_STATUS] })}
      />,
    );
    // containerStateText({running: {...}, ready: true}) -> "running, ready"
    expect(screen.getByText("running, ready")).toBeDefined();
    expect(screen.getByText("3")).toBeDefined();
  });

  it("shows ports, probes, environment and mounts for a container that has them", () => {
    render(
      <PodContainersBody
        context="ctx"
        object={pod({ containers: [APP_CONTAINER] }, { containerStatuses: [APP_STATUS] })}
      />,
    );
    expect(screen.getByText("Ports")).toBeDefined();
    expect(screen.getByText("http: 8080/TCP")).toBeDefined();
    expect(screen.getByText("Environment")).toBeDefined();
    expect(screen.getByText("LOG_LEVEL=info")).toBeDefined();
    expect(screen.getByText("Mounts")).toBeDefined();
    expect(screen.getByText("/var/cache ← cache")).toBeDefined();
    expect(screen.getByText("Liveness")).toBeDefined();
    expect(screen.getByText("Readiness")).toBeDefined();
    expect(screen.queryByText("Startup")).toBeNull();
  });

  it("omits ports, probes, environment and mounts for a container that has none", () => {
    render(
      <PodContainersBody
        context="ctx"
        object={pod({ containers: [SIDECAR_CONTAINER] }, { containerStatuses: [SIDECAR_STATUS] })}
      />,
    );
    expect(screen.getByText("sidecar")).toBeDefined();
    expect(screen.queryByText("Ports")).toBeNull();
    expect(screen.queryByText("Environment")).toBeNull();
    expect(screen.queryByText("Mounts")).toBeNull();
    expect(screen.queryByText("Liveness")).toBeNull();
    expect(screen.queryByText("Readiness")).toBeNull();
    expect(screen.queryByText("Startup")).toBeNull();
  });

  it("shows No containers when the pod has none", () => {
    render(<PodContainersBody context="ctx" object={pod({})} />);
    expect(screen.getByText("No containers")).toBeDefined();
  });

  it("shows Running since as a distinct fact from Last restart", () => {
    const status = {
      name: "app",
      ready: true,
      restartCount: 2,
      state: { running: { startedAt: "2026-08-20T12:00:00Z" } },
      lastState: { terminated: { finishedAt: "2026-08-19T00:00:00Z" } },
    };
    render(
      <PodContainersBody
        context="ctx"
        object={pod({ containers: [APP_CONTAINER] }, { containerStatuses: [status] })}
      />,
    );
    expect(screen.getByText("Last restart")).toBeDefined();
    expect(screen.getByText("Running since")).toBeDefined();
    // containerLastRestartTime reads lastState (the previous run's
    // termination); Running since reads state.running.startedAt (the
    // current run) — two different timestamps, not the same fact twice.
    const lastRestartRow = screen.getByText("Last restart").closest("dl");
    const runningSinceRow = screen.getByText("Running since").closest("dl");
    expect(lastRestartRow?.textContent).not.toEqual(runningSinceRow?.textContent);
  });

  it("shows which container an ephemeral container is debugging", () => {
    const debugContainer = { name: "debugger", image: "busybox", targetContainerName: "app" };
    render(<PodContainersBody context="ctx" object={pod({ ephemeralContainers: [debugContainer] })} />);
    expect(screen.getByText("Ephemeral containers")).toBeDefined();
    expect(screen.getByText("debugger")).toBeDefined();
    expect(screen.getByText("Debugging")).toBeDefined();
    expect(screen.getByText("app")).toBeDefined();
  });

  it("shows a container's command and args", () => {
    const commandContainer = { ...APP_CONTAINER, command: ["/bin/sh", "-c"], args: ["sleep 3600"] };
    render(
      <PodContainersBody
        context="ctx"
        object={pod({ containers: [commandContainer] }, { containerStatuses: [APP_STATUS] })}
      />,
    );
    expect(screen.getByText("Command")).toBeDefined();
    expect(screen.getByText("/bin/sh -c sleep 3600")).toBeDefined();
  });

  it("omits Debugging and Command when a container has neither", () => {
    render(
      <PodContainersBody
        context="ctx"
        object={pod({ containers: [SIDECAR_CONTAINER] }, { containerStatuses: [SIDECAR_STATUS] })}
      />,
    );
    expect(screen.queryByText("Debugging")).toBeNull();
    expect(screen.queryByText("Command")).toBeNull();
  });
});

/**
 * A container's ports, on both surfaces that draw them.
 *
 * Two ports on two containers, and NEITHER number is one of §A.4's own
 * placeholders (9090 local, 8080 remote) — so a prefill that came from the
 * field's default rather than from the click cannot pass.
 */
const PORTED_CONTAINER = {
  name: "api",
  image: "ghcr.io/example/api:2",
  ports: [
    { name: "http", containerPort: 9376, protocol: "TCP" },
    { name: "metrics", containerPort: 5432, protocol: "TCP" },
  ],
};

const PORTED_STATUS = {
  name: "api",
  ready: true,
  restartCount: 0,
  state: { running: { startedAt: "2026-08-20T00:00:00Z" } },
};

const dialogSelect = (name: string) => screen.getByLabelText(name) as HTMLSelectElement;
const dialogInput = (name: string) => screen.getByLabelText(name) as HTMLInputElement;

/**
 * The container's ports are the other place the reader is already looking at
 * the exact port they want. They used to be a run of inert strings — on the
 * peek's pane a list, on the full tab's table one comma-joined cell — with the
 * affordance classic offers inline missing from both.
 */
describe("a container's ports as the way in to a forward", () => {
  it("makes every port of every container its own forward, on the peek's pane", async () => {
    render(
      <PodContainersBody
        context="ctx"
        object={pod(
          { containers: [PORTED_CONTAINER, SIDECAR_CONTAINER] },
          { containerStatuses: [PORTED_STATUS, SIDECAR_STATUS] },
        )}
      />,
    );
    // Core's own `portText`, still — the words did not change, only what they
    // are attached to.
    expect(screen.getByRole("button", { name: "Forward http: 9376/TCP on api" })).toBeDefined();
    expect(screen.getByRole("button", { name: "Forward metrics: 5432/TCP on api" })).toBeDefined();
    // A container with no ports gains no affordance and no empty row.
    expect(screen.queryByRole("button", { name: /on sidecar/ })).toBeNull();
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("opens §A.4's dialog on this POD, in its namespace, on the port that was clicked", async () => {
    render(
      <PodContainersBody
        context="ctx"
        object={pod({ containers: [PORTED_CONTAINER] }, { containerStatuses: [PORTED_STATUS] })}
      />,
    );
    // The SECOND port, so a cell that always handed over its first cannot pass.
    await userEvent.click(screen.getByRole("button", { name: "Forward metrics: 5432/TCP on api" }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("New port forward")).toBeDefined();
    await waitFor(() => expect(dialogSelect("Target").value).toBe("pod/web-1"));
    await waitFor(() => expect(dialogSelect("Namespace").value).toBe("default"));
    expect(dialogInput("Remote port").value).toBe("5432");
    // Offered, not demanded, and NOT the far end again — a free port from a
    // range nothing claims by convention. The property, not the draw.
    const offered = Number(dialogInput("Local port").value);
    expect(offered).toBeGreaterThanOrEqual(10000);
    expect(offered).toBeLessThanOrEqual(32767);
    expect(offered).not.toBe(5432);
  });

  it("names the Pod as a Pod — pod/, from its kind, not the container's name", async () => {
    render(
      <PodContainersBody
        context="ctx"
        object={pod({ containers: [PORTED_CONTAINER] }, { containerStatuses: [PORTED_STATUS] })}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Forward http: 9376/TCP on api" }));
    await screen.findByRole("dialog");
    await userEvent.clear(screen.getByLabelText("Local port"));
    await userEvent.type(screen.getByLabelText("Local port"), "9091");
    await waitFor(() =>
      expect(screen.getByText(/port-forward pod\/web-1 9091:9376/)).toBeDefined(),
    );
    expect(screen.queryByText(/port-forward pod\/api/)).toBeNull();
  });

  it("does the same in the full tab's containers table, where they were one joined string", async () => {
    render(
      <PodContainersTable
        context="ctx"
        object={pod({ containers: [PORTED_CONTAINER] }, { containerStatuses: [PORTED_STATUS] })}
      />,
    );
    expect(screen.queryByRole("dialog")).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Forward metrics: 5432/TCP on api" }));
    await screen.findByRole("dialog");
    await waitFor(() => expect(dialogSelect("Target").value).toBe("pod/web-1"));
    expect(dialogInput("Remote port").value).toBe("5432");
  });

  it("still says a portless container has no ports, on both surfaces", () => {
    const { unmount } = render(
      <PodContainersTable
        context="ctx"
        object={pod({ containers: [SIDECAR_CONTAINER] }, { containerStatuses: [SIDECAR_STATUS] })}
      />,
    );
    // The Ports cell's own dash — several columns render one for a container
    // this bare, so the count is what says the cell did not vanish.
    expect(screen.getAllByText("—").length).toBeGreaterThan(0);
    unmount();
    render(
      <PodContainersBody
        context="ctx"
        object={pod({ containers: [SIDECAR_CONTAINER] }, { containerStatuses: [SIDECAR_STATUS] })}
      />,
    );
    expect(screen.queryByText("Ports")).toBeNull();
  });

  it("puts no port or command in a title attribute", async () => {
    render(
      <PodContainersBody
        context="ctx"
        object={pod({ containers: [PORTED_CONTAINER] }, { containerStatuses: [PORTED_STATUS] })}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Forward http: 9376/TCP on api" }));
    await screen.findByRole("dialog");
    const joined = Array.from(document.querySelectorAll("[title]"))
      .map((el) => el.getAttribute("title") ?? "")
      .join("\n");
    for (const leak of ["9376", "5432", "port-forward", "--context"]) {
      expect(joined).not.toContain(leak);
    }
  });
});

/** The label column of one flat block, in the order it reads. `heading`
 *  names the block; without one, the pane's first block — which the design
 *  heads with nothing at all. */
function factLabels(container: HTMLElement, heading?: string): string[] {
  const block = heading
    ? screen.getByRole("heading", { name: heading }).closest("section")
    : container.querySelector("section.section");
  return [...(block?.querySelectorAll(".kv-k") ?? [])].map((el) => el.textContent ?? "");
}

/** The text of the fact labelled `label`, or `undefined` when there is none. */
function factValue(container: HTMLElement, label: string): string | undefined {
  const row = [...container.querySelectorAll("dl.kv")].find((dl) => dl.querySelector(".kv-k")?.textContent === label);
  return row?.querySelector(".kv-v")?.textContent ?? undefined;
}

/**
 * A pod's facts, drawn.
 *
 * `podFacts` hands back DATA — a list of label/value pairs — because the two
 * detail screens lay one list out two different ways: the peek reads it down a
 * column of label-beside-value rows, the full tab across three columns of
 * label-above-value. Neither layout is this file's business, so the cases
 * below render the list through the plainest possible rows and assert what was
 * DERIVED. Each screen's own test pins its own layout. (#331)
 */
function renderFacts(object: K8sObject) {
  return render(
    <Section>
      {podFacts({ kind: "Pod", object }).map((fact) => (
        <KV key={fact.label} k={fact.label} v={fact.value} mono={fact.mono} />
      ))}
    </Section>,
  );
}

describe("PodDetailsBody", () => {
  const FULL_POD = pod(
    {
      containers: [APP_CONTAINER],
      nodeName: "node-a",
      serviceAccountName: "web-sa",
      priorityClassName: "high",
      runtimeClassName: "gvisor",
      imagePullSecrets: [{ name: "registry-creds" }],
    },
    {
      phase: "Running",
      podIP: "10.0.0.5",
      podIPs: [{ ip: "10.0.0.5" }, { ip: "fd00::5" }],
      qosClass: "Burstable",
      containerStatuses: [APP_STATUS],
    },
    {
      name: "web-1",
      namespace: "default",
      creationTimestamp: "2026-08-20T00:00:00Z",
      labels: { app: "web", tier: "frontend" },
      annotations: { "kubectl.kubernetes.io/note": "deployed via ci" },
      ownerReferences: [{ kind: "ReplicaSet", name: "web-abc123" }],
    },
  );

  describe("the pod's facts, which each screen lays out itself", () => {
    it("leads with the identity every kind leads with, and heads it with nothing", () => {
      // Created, Controlled by, then the pod's own. The extras srelens shows
      // beyond the design frame sit beside their own kin (Last restart after
      // Restarts) rather than at the end. A dual-stack pod's addresses are
      // one fact, not the first address and then all of them (#1070).
      const { container } = renderFacts(FULL_POD);
      expect(factLabels(container)).toEqual([
        "Created",
        "Controlled by",
        "Node",
        "Pod IPs",
        "QoS class",
        "Service account",
        "Priority class",
        "Runtime class",
        "Image pull secrets",
        "Restarts",
        "Image",
      ]);
      // No heading over the first block: the pane's header has already said
      // which pod this is.
      expect(screen.queryByRole("heading", { name: "Properties" })).toBeNull();
    });

    it("gives a single-stack pod's address once, as Pod IP (#1070)", () => {
      // `status.podIPs` always holds `status.podIP` as its first entry, so on
      // the ordinary pod the two facts printed one address twice.
      const { container } = renderFacts(
        pod({}, { podIP: "10.0.0.5", podIPs: [{ ip: "10.0.0.5" }] }, { name: "web-1" }),
      );
      expect(factLabels(container)).toContain("Pod IP");
      expect(factLabels(container)).not.toContain("Pod IPs");
      expect(screen.getAllByText("10.0.0.5")).toHaveLength(1);
    });

    it("lists every address of a dual-stack pod under Pod IPs, each once (#1070)", () => {
      const { container } = renderFacts(FULL_POD);
      expect(factLabels(container)).not.toContain("Pod IP");
      expect(screen.getAllByText("10.0.0.5")).toHaveLength(1);
      expect(screen.getAllByText("fd00::5")).toHaveLength(1);
    });

    it("shows the pod's image, which used to live only on the Containers pane", () => {
      renderFacts(FULL_POD);
      expect(screen.getByText("Image")).toBeDefined();
      expect(screen.getByText("ghcr.io/example/app:1.2.3")).toBeDefined();
    });

    it("names every image a multi-container pod runs, not just the first", () => {
      renderFacts(pod({ containers: [APP_CONTAINER, SIDECAR_CONTAINER] }, {}, { name: "web-1" }));
      expect(screen.getByText("ghcr.io/example/app:1.2.3")).toBeDefined();
      expect(screen.getByText("ghcr.io/example/sidecar:1.0")).toBeDefined();
    });

    it("leaves the status, the ready count and the namespace to the header", () => {
      const { container } = renderFacts(FULL_POD);
      for (const label of ["Status", "Containers ready", "Namespace"]) {
        expect(factLabels(container)).not.toContain(label);
      }
    });

    it("says Restarts, the word the design uses", () => {
      renderFacts(FULL_POD);
      expect(screen.getByText("Restarts")).toBeDefined();
      expect(screen.queryByText("Container restarts")).toBeNull();
      expect(screen.getByText("3")).toBeDefined();
    });

    it("drops the Name row, which repeated the pane's own header", () => {
      const { container } = renderFacts(FULL_POD);
      expect(factLabels(container)).not.toContain("Name");
      expect(screen.queryByText("web-1")).toBeNull();
    });

    it("reads Created as an age alone", () => {
      renderFacts(FULL_POD);
      const created = screen.getByText("Created").closest("dl");
      expect(created?.textContent).toMatch(/^Created\d/);
      expect(created?.textContent).not.toMatch(/\(/);
    });

    it("shows the remaining facts as plain text, with nothing that navigates", () => {
      renderFacts(FULL_POD);
      expect(screen.getByText("ReplicaSet/web-abc123")).toBeDefined();
      expect(screen.getAllByText("node-a").length).toBeGreaterThan(0);
      expect(screen.getByText("fd00::5")).toBeDefined();
      expect(screen.getByText("web-sa")).toBeDefined();
      expect(screen.getByText("high")).toBeDefined();
      expect(screen.getByText("gvisor")).toBeDefined();
      expect(screen.getByText("Secret/registry-creds")).toBeDefined();
      expect(screen.getByText("Burstable")).toBeDefined();
      // Node, Service account, Priority class, Runtime class and Controlled
      // by are `ResourceLink`s in classic; nothing here can
      // navigate (see the task report) — bar the Node, which is a link once a
      // cluster is resolved (#822). None is here, so it too is text.
      expect(screen.queryByRole("button", { name: /^Open / })).toBeNull();
    });

    it("omits absent facts rather than showing them empty", () => {
      const bare = pod({}, {}, { name: "bare-1", namespace: "default" });
      const { container } = renderFacts(bare);
      expect(factLabels(container)).toEqual([]);
    });

    it("leads the pod's own facts with its reason and message when the kubelet set them (#619)", () => {
      // An evicted pod: the container statuses say nothing useful, and the
      // why lives only on `status.reason` / `status.message` — the lines
      // `kubectl describe pod` prints under Status.
      const message = "The node was low on resource: memory. Threshold quantity: 100Mi, available: 64Mi.";
      const { container } = renderFacts(
        pod(
          { containers: [APP_CONTAINER], nodeName: "node-a" },
          { phase: "Failed", reason: "Evicted", message },
          { name: "web-1", namespace: "default" },
        ),
      );
      expect(factLabels(container).slice(0, 3)).toEqual(["Reason", "Message", "Node"]);
      expect(factValue(container, "Reason")).toBe("Evicted");
      expect(screen.getByText(message)).toBeDefined();
    });

    it("shows a reason without a message, and a message without a reason", () => {
      const { container, unmount } = renderFacts(pod({}, { phase: "Failed", reason: "NodeLost" }, { name: "a" }));
      expect(factLabels(container)).toContain("Reason");
      expect(factLabels(container)).not.toContain("Message");
      unmount();
      const second = renderFacts(pod({}, { phase: "Pending", message: "Pod was rejected" }, { name: "b" }));
      expect(factLabels(second.container)).not.toContain("Reason");
      expect(factLabels(second.container)).toContain("Message");
    });

    it("omits Reason and Message on a healthy pod, where the API leaves them unset", () => {
      const { container } = renderFacts(FULL_POD);
      expect(factLabels(container)).not.toContain("Reason");
      expect(factLabels(container)).not.toContain("Message");
    });

    it("is one flat block, not a card", () => {
      // Whichever screen draws it: an unheaded run of pairs, divided from
      // what follows by a rule rather than boxed in a frame.
      const { container } = renderFacts(FULL_POD);
      expect(container.querySelector(".card")).toBeNull();
      expect(container.querySelectorAll("section.section")).toHaveLength(1);
      expect(container.querySelector("section.section > h3")).toBeNull();
    });
  });

  /**
   * Labels and Annotations are no longer this body's, and the pane they are
   * drawn on is where they are pinned now — `ResourceDetailView.test`'s "Labels
   * and Annotations, which the host places". They moved because the two hosts
   * lay them out differently (the peek stacks them, the full tab reads them
   * side by side) and a body that rendered its own could only ever produce
   * one of those. The Secret gate moved with them, whole.
   */
  describe("Labels and Annotations", () => {
    it("renders neither, so the host can place them", () => {
      render(<PodDetailsBody object={FULL_POD} />);
      expect(screen.queryByRole("heading", { name: "Labels" })).toBeNull();
      expect(screen.queryByRole("heading", { name: "Annotations" })).toBeNull();
    });
  });

  describe("Conditions", () => {
    it("shows the pod's own conditions in lifecycle order, through the one shared block", () => {
      const { container } = render(
        <PodDetailsBody
          object={pod(
            {},
            {
              conditions: [
                { type: "Ready", status: "True", lastTransitionTime: "2026-08-20T00:03:00Z" },
                { type: "PodScheduled", status: "True", lastTransitionTime: "2026-08-20T00:00:00Z" },
                { type: "ContainersReady", status: "True", lastTransitionTime: "2026-08-20T00:02:00Z" },
                { type: "Initialized", status: "True", lastTransitionTime: "2026-08-20T00:01:00Z" },
              ],
            },
          )}
        />,
      );
      const block = screen.getByRole("heading", { name: "Conditions" }).closest("section")!;
      expect([...block.querySelectorAll("li .status")].map((el) => el.textContent)).toEqual([
        "PodScheduled",
        "Initialized",
        "ContainersReady",
        "Ready",
      ]);
      // The shared block's form: the status, and its reason when it has one;
      // no last-transition column. A pod's lifecycle conditions carry no
      // reason, so each reads its status alone (#1070).
      expect(screen.getAllByText("True")).toHaveLength(4);
    });

    it("omits the Conditions block when the pod reports none", () => {
      const { container } = render(<PodDetailsBody object={pod({}, {})} />);
      expect(screen.queryByText("Conditions")).toBeNull();
      expect(container.querySelector(".card")).toBeNull();
    });
  });

  describe("Scheduling", () => {
    it("shows Scheduling facts when the pod has placement info", () => {
      const scheduled = pod(
        {
          nodeName: "node-b",
          nodeSelector: { disktype: "ssd" },
          affinity: { podAntiAffinity: { requiredDuringSchedulingIgnoredDuringExecution: [{}] } },
          tolerations: [{ key: "dedicated", operator: "Equal", value: "gpu", effect: "NoSchedule" }],
        },
        {},
        { name: "web-2" },
      );
      render(<PodDetailsBody object={scheduled} />);
      expect(screen.getByRole("heading", { name: "Scheduling" })).toBeDefined();
      // Not here at all: the node is one of the pod's own facts, which the
      // screen draws above this, and the pane said it twice (#1070).
      expect(screen.queryByText("node-b")).toBeNull();
      expect(podFacts({ kind: "Pod", object: scheduled }).map((f) => f.label)).toContain("Node");
      expect(screen.getByText("disktype=")).toBeDefined();
      expect(screen.getByText("ssd")).toBeDefined();
      expect(screen.getByText("Pod anti-affinity: 1 required")).toBeDefined();
      expect(screen.getByText("dedicated=gpu → NoSchedule")).toBeDefined();
      // With no cluster resolved there is nowhere to open the node, and the
      // name stays text — see `NodeLink`.
      expect(screen.queryByRole("button", { name: /^Open / })).toBeNull();
    });

    describe("on a resolved cluster (#822)", () => {
      const PROD: ClusterContext = {
        name: "prod-eu",
        stableId: "prod",
        key: "prod",
        cluster: "prod",
        server: "https://prod",
        isCurrent: true,
        sourceFile: "/home/dana/.kube/config",
        authKind: "client certificate",
      };
      const scheduled = () => pod({ nodeName: "node-b" }, {}, { name: "web-2" });

      beforeEach(() => {
        resetContexts();
        setContexts([PROD]);
        store.setState(defaultState([PROD]));
      });
      afterEach(() => {
        resetContexts();
        store.setState(defaultState([]));
      });

      it("makes the pod's Node fact the way to that node", async () => {
        const fact = podFacts({ kind: "Pod", object: scheduled() }).find((f) => f.label === "Node")!;
        render(<>{fact.value}</>);
        await userEvent.click(screen.getByRole("button", { name: "Open node node-b" }));

        const tab = store.currentWorkspace().tabs.find((t) => t.route === detailRoute("Node", null, "node-b", { cluster: "prod-eu" }));
        expect(tab).toBeDefined();
        expect(tab!.sub).toBe("prod-eu");
      });

      it("makes the pod's own Node fact the same link", () => {
        const fact = podFacts({ kind: "Pod", object: scheduled() }).find((f) => f.label === "Node")!;
        render(<>{fact.value}</>);
        expect(screen.getByRole("button", { name: "Open node node-b" }).textContent).toBe("node-b");
      });

      it("offers nothing to open for a pod that has not been scheduled", () => {
        const pending = pod(
          { tolerations: [{ key: "dedicated", operator: "Equal", value: "gpu", effect: "NoSchedule" }] },
          {},
          { name: "web-6" },
        );
        render(<PodDetailsBody object={pending} />);
        expect(screen.getByText("Not scheduled")).toBeDefined();
        expect(screen.queryByRole("button", { name: /^Open node/ })).toBeNull();
      });
    });

    it("omits the Scheduling block when the pod has no placement info", () => {
      render(<PodDetailsBody object={pod({}, {}, { name: "web-3" })} />);
      expect(screen.queryByText("Scheduling")).toBeNull();
    });

    it("keeps each toleration on one line and scrolls it, as machine text (#1070)", () => {
      // "node.kubernetes.io/not-ready exists → NoExecute for" / "300s" broke a
      // clause in half in the value column. AGENTS.md: long machine text does
      // not wrap. Every pod carries this pair, added by the API server.
      render(
        <PodDetailsBody
          object={pod(
            {
              tolerations: [
                { key: "node.kubernetes.io/not-ready", operator: "Exists", effect: "NoExecute", tolerationSeconds: 300 },
                { key: "node.kubernetes.io/unreachable", operator: "Exists", effect: "NoExecute", tolerationSeconds: 300 },
              ],
            },
            {},
            { name: "web-5" },
          )}
        />,
      );
      const value = screen.getByText("Tolerations").closest("dl")!.querySelector("dd")!;
      expect(value.className).toContain("whitespace-pre");
      expect(value.className).toContain("overflow-x-auto");
      expect(screen.getByText("node.kubernetes.io/not-ready exists → NoExecute for 300s")).toBeDefined();
    });

    it("omits the Scheduling block when its node is all it would say (#1070)", () => {
      // The node is a lead fact; a block repeating it and nothing else is the
      // duplicate this removed.
      render(<PodDetailsBody object={pod({ nodeName: "node-b" }, {}, { name: "web-4" })} />);
      expect(screen.queryByText("Scheduling")).toBeNull();
    });

    it("shows Not scheduled when the pod has placement info but no assigned node", () => {
      const pending = pod(
        { tolerations: [{ key: "dedicated", operator: "Equal", value: "gpu", effect: "NoSchedule" }] },
        {},
        { name: "web-6" },
      );
      render(<PodDetailsBody object={pending} />);
      expect(screen.getByRole("heading", { name: "Scheduling" })).toBeDefined();
      expect(screen.getByText("Not scheduled")).toBeDefined();
      expect(screen.getByText("Node")).toBeDefined();
    });
  });

  describe("Pod Volumes", () => {
    it("shows each volume's name, type and source", () => {
      const withVolumes = pod(
        {
          volumes: [
            { name: "data", persistentVolumeClaim: { claimName: "data-pvc" } },
            { name: "cache", emptyDir: {} },
            { name: "creds", secret: { secretName: "app-creds" } },
          ],
        },
        {},
        { name: "web-4" },
      );
      render(<PodDetailsBody object={withVolumes} />);
      expect(screen.getByRole("heading", { name: "Pod volumes" })).toBeDefined();
      expect(document.querySelector('section[data-padded="false"]')).toBeDefined();
      expect(screen.getByText("data")).toBeDefined();
      expect(screen.getByText("Persistent Volume Claim")).toBeDefined();
      expect(screen.getByText("PersistentVolumeClaim/data-pvc")).toBeDefined();
      expect(screen.getByText("cache")).toBeDefined();
      expect(screen.getByText("Empty Dir")).toBeDefined();
      expect(screen.getByText("Node temporary storage")).toBeDefined();
      expect(screen.getByText("creds")).toBeDefined();
      expect(screen.getByText("Secret/app-creds")).toBeDefined();
      expect(screen.queryByRole("button", { name: /^Open / })).toBeNull();
    });

    it("omits the Pod Volumes block when the pod has no volumes", () => {
      render(<PodDetailsBody object={pod({}, {}, { name: "web-5" })} />);
      expect(screen.queryByText("Pod volumes")).toBeNull();
    });
  });
});
