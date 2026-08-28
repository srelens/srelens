import { describe, it, expect, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { K8sObject } from "@srelens/core";
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
    it("leads with what the design's own Pod frame leads with, and heads it with nothing", () => {
      // Status first, Created ninth — not classic's Created/Name/Namespace
      // opening. The extras srelens shows beyond the design frame sit beside
      // their own kin (Pod IPs after Pod IP, Last restart after Restarts)
      // rather than at the end.
      const { container } = renderFacts(FULL_POD);
      expect(factLabels(container)).toEqual([
        "Status",
        "Node",
        "Pod IP",
        "Pod IPs",
        "QoS class",
        "Service account",
        "Priority class",
        "Runtime class",
        "Image pull secrets",
        "Containers ready",
        "Restarts",
        "Controlled by",
        "Namespace",
        "Created",
        "Image",
      ]);
      // No heading over the first block: the pane's header has already said
      // which pod this is.
      expect(screen.queryByRole("heading", { name: "Properties" })).toBeNull();
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

    it("counts the containers that are ready", () => {
      renderFacts(FULL_POD);
      expect(screen.getByText("Containers ready")).toBeDefined();
      expect(screen.getByText("1 of 1")).toBeDefined();
    });

    it("omits the ready count while the kubelet has reported no container statuses", () => {
      // "0 of 0" would read as a fact where there is only an absence.
      renderFacts(pod({ containers: [APP_CONTAINER] }, { phase: "Pending" }));
      expect(screen.queryByText("Containers ready")).toBeNull();
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

    it("takes the status word from core's one reading, so the header cannot contradict it", () => {
      // A pod whose container is in CrashLoopBackOff still reports phase
      // "Running"; `resourceStatusLine` is what the header reads too.
      renderFacts(
        pod(
          { containers: [APP_CONTAINER] },
          {
            phase: "Running",
            containerStatuses: [{ name: "app", ready: false, restartCount: 7, state: { waiting: { reason: "CrashLoopBackOff" } } }],
          },
        ),
      );
      expect(screen.getByText("CrashLoopBackOff")).toBeDefined();
      expect(screen.queryByText("Running")).toBeNull();
    });

    it("keeps the header's word off the phase between restarts, when there is no reason to show", () => {
      // The same pod a moment later: the container is genuinely running, so
      // there is no waiting reason on the object at all — only `ready: false`
      // and a restart count. The header used to read a plain "Running" here
      // while the pod was still failing every few seconds.
      renderFacts(
        pod(
          { containers: [APP_CONTAINER] },
          {
            phase: "Running",
            containerStatuses: [
              {
                name: "app",
                ready: false,
                restartCount: 7,
                state: { running: { startedAt: "2026-08-24T13:28:18Z" } },
                lastState: { terminated: { exitCode: 1, reason: "Error" } },
              },
            ],
          },
        ),
      );
      expect(screen.getByText("NotReady")).toBeDefined();
      expect(screen.queryByText("Running")).toBeNull();
    });

    it("still reads a pod that has simply not become ready yet as Running", () => {
      // The carve-out, at the header: no restarts, so nothing says this is
      // failure rather than a container that started two seconds ago.
      renderFacts(
        pod(
          { containers: [APP_CONTAINER] },
          {
            phase: "Running",
            containerStatuses: [
              { name: "app", ready: false, restartCount: 0, state: { running: { startedAt: "2026-08-24T13:28:18Z" } } },
            ],
          },
        ),
      );
      expect(screen.getByText("Running")).toBeDefined();
      expect(screen.queryByText("NotReady")).toBeNull();
    });

    it("shows the remaining facts as plain text, with nothing that navigates", () => {
      renderFacts(FULL_POD);
      expect(screen.getByText("default")).toBeDefined();
      expect(screen.getByText("ReplicaSet/web-abc123")).toBeDefined();
      expect(screen.getAllByText("node-a").length).toBeGreaterThan(0);
      expect(screen.getByText("fd00::5")).toBeDefined();
      expect(screen.getByText("web-sa")).toBeDefined();
      expect(screen.getByText("high")).toBeDefined();
      expect(screen.getByText("gvisor")).toBeDefined();
      expect(screen.getByText("Secret/registry-creds")).toBeDefined();
      expect(screen.getByText("Burstable")).toBeDefined();
      // Namespace, Node, Service account, Priority class, Runtime class and
      // Controlled by are `ResourceLink`s in classic; nothing here can
      // navigate (see the task report).
      expect(screen.queryByRole("button", { name: /^Open / })).toBeNull();
    });

    it("omits absent facts rather than showing them empty", () => {
      const bare = pod({}, {}, { name: "bare-1", namespace: "default" });
      const { container } = renderFacts(bare);
      expect(factLabels(container)).toEqual(["Status", "Namespace"]);
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
      expect(factLabels(container, "Conditions")).toEqual([
        "PodScheduled",
        "Initialized",
        "ContainersReady",
        "Ready",
      ]);
      // The shared block's form: status and reason as one value, no
      // last-transition column.
      expect(screen.getAllByText("True · —")).toHaveLength(4);
    });

    it("omits the Conditions block when the pod reports none", () => {
      const { container } = render(<PodDetailsBody object={pod({}, {})} />);
      expect(screen.queryByText("Conditions")).toBeNull();
      // And the pane is not empty on its account: the facts are the screen's
      // to draw and are still derived — see the fact cases above.
      expect(container.querySelector(".card")).toBeNull();
      expect(podFacts({ kind: "Pod", object: pod({}, {}) }).map((f) => f.label)).toContain("Status");
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
      // Once here. The node is also one of the pod's own facts, which the
      // screen draws above this — the same duplication classic has, now
      // asserted where each half lives.
      expect(screen.getAllByText("node-b")).toHaveLength(1);
      expect(podFacts({ kind: "Pod", object: scheduled }).map((f) => f.label)).toContain("Node");
      expect(screen.getByText("disktype=")).toBeDefined();
      expect(screen.getByText("ssd")).toBeDefined();
      expect(screen.getByText("Pod anti-affinity: 1 required")).toBeDefined();
      expect(screen.getByText("dedicated=gpu → NoSchedule")).toBeDefined();
      expect(screen.queryByRole("button", { name: /^Open / })).toBeNull();
    });

    it("omits the Scheduling block when the pod has no placement info", () => {
      render(<PodDetailsBody object={pod({}, {}, { name: "web-3" })} />);
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
      expect(screen.getByRole("heading", { name: "Pod Volumes" })).toBeDefined();
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
      expect(screen.queryByText("Pod Volumes")).toBeNull();
    });
  });
});
