import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import React from "react";

const {
  deletePodMock,
  evictPodMock,
  deleteResourceMock,
  scaleResourceMock,
  rolloutRestartMock,
  cronjobSetSuspendMock,
  cronjobTriggerNowMock,
  debugPodMock,
} = vi.hoisted(() => ({
  deletePodMock: vi.fn(),
  evictPodMock: vi.fn(),
  deleteResourceMock: vi.fn(),
  scaleResourceMock: vi.fn(),
  rolloutRestartMock: vi.fn(),
  cronjobSetSuspendMock: vi.fn(),
  cronjobTriggerNowMock: vi.fn(),
  debugPodMock: vi.fn(),
}));
vi.mock("../lib/workloads", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../lib/workloads")>();
  return { ...actual, deletePod: deletePodMock, evictPod: evictPodMock };
});
vi.mock("../lib/actions", () => ({
  deleteResource: deleteResourceMock,
  scaleResource: scaleResourceMock,
  rolloutRestart: rolloutRestartMock,
  cronjobSetSuspend: cronjobSetSuspendMock,
  cronjobTriggerNow: cronjobTriggerNowMock,
  debugPod: debugPodMock,
}));
const { getObjectMock } = vi.hoisted(() => ({ getObjectMock: vi.fn() }));
vi.mock("../lib/manifest", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../lib/manifest")>();
  return { ...actual, getObject: getObjectMock };
});
const { notifyMock } = vi.hoisted(() => ({
  notifyMock: { success: vi.fn(), error: vi.fn(), info: vi.fn() },
}));
vi.mock("../lib/notify", () => ({ notify: notifyMock }));

vi.mock("../lib/access", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../lib/access")>();
  return { ...actual, useAccess: vi.fn() };
});
import { useAccess } from "../lib/access";
import { describeError } from "../lib/errors";

import {
  PodActions,
  ResourceActions,
  desiredReplicasFrom,
  desiredReplicasForDetail,
} from "./DetailActions";

const pod = {
  name: "web-1",
  namespace: "default",
  phase: "Running",
  ready: "1/1",
  restarts: 0,
  node: "node-a",
  age: "2d",
};

// This repo doesn't pull in @testing-library/jest-dom, so assert directly on
// DOM properties instead of `toBeDisabled`/`toHaveAttribute` sugar.
function isDisabled(el: HTMLElement): boolean {
  return (el as HTMLButtonElement).disabled;
}
function titleOf(el: HTMLElement): string | null {
  return el.getAttribute("title");
}

beforeEach(() => {
  deletePodMock.mockReset();
  evictPodMock.mockReset();
  deleteResourceMock.mockReset();
  scaleResourceMock.mockReset();
  rolloutRestartMock.mockReset();
  cronjobSetSuspendMock.mockReset();
  cronjobTriggerNowMock.mockReset();
  debugPodMock.mockReset();
  getObjectMock.mockReset();
  // Default: object fetches resolve to nothing useful — tests that care about
  // the replica seed set their own return value.
  getObjectMock.mockResolvedValue({});
  notifyMock.success.mockReset();
  notifyMock.error.mockReset();
  // Default: everything allowed, so pre-existing behavioural tests (written
  // before RBAC gating existed) keep exercising enabled controls.
  vi.mocked(useAccess).mockReturnValue({
    allowed: () => true,
    reason: () => "",
    known: () => true,
    loading: false,
  });
  // jsdom has no clipboard API; stub it fresh per test (issue #158).
  Object.defineProperty(navigator, "clipboard", {
    value: { writeText: vi.fn().mockResolvedValue(undefined) },
    configurable: true,
  });
});

describe("PodActions", () => {
  it("opens logs and shell via the header icons", () => {
    const onOpenLogs = vi.fn();
    const onOpenTerminal = vi.fn();
    render(
      <PodActions context="kind-dev" pod={pod} onOpenLogs={onOpenLogs} onOpenTerminal={onOpenTerminal} />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Logs" }));
    fireEvent.click(screen.getByRole("button", { name: "Shell" }));
    expect(onOpenLogs).toHaveBeenCalledWith({ context: "kind-dev", namespace: "default", pod: "web-1" });
    expect(onOpenTerminal).toHaveBeenCalledWith({ context: "kind-dev", namespace: "default", pod: "web-1" });
  });

  it("asks which container to open a shell into on a multi-container pod", async () => {
    getObjectMock.mockResolvedValue({
      object: {
        spec: { containers: [{ name: "istio-proxy" }, { name: "app" }] },
        status: {
          containerStatuses: [
            { name: "istio-proxy", state: { running: {} } },
            { name: "app", state: { running: {} } },
          ],
        },
      },
    });
    const onOpenTerminal = vi.fn();
    render(<PodActions context="kind-dev" pod={pod} onOpenTerminal={onOpenTerminal} />);
    await waitFor(() => expect(getObjectMock).toHaveBeenCalled());

    fireEvent.click(screen.getByRole("button", { name: "Shell" }));
    fireEvent.click(await screen.findByRole("menuitem", { name: /^app/ }));
    expect(onOpenTerminal).toHaveBeenCalledWith({
      context: "kind-dev",
      namespace: "default",
      pod: "web-1",
      container: "app",
    });
  });

  it("opens the annotated default container without asking on a single-container pod", async () => {
    // One container means there is no choice to make; the annotation still
    // decides, because the API server ignores it and would pick spec order.
    getObjectMock.mockResolvedValue({
      object: {
        metadata: { annotations: { "kubectl.kubernetes.io/default-container": "app" } },
        spec: { containers: [{ name: "app" }] },
        status: { containerStatuses: [{ name: "app", state: { running: {} } }] },
      },
    });
    const onOpenTerminal = vi.fn();
    render(<PodActions context="kind-dev" pod={pod} onOpenTerminal={onOpenTerminal} />);
    await waitFor(() => expect(getObjectMock).toHaveBeenCalled());

    fireEvent.click(screen.getByRole("button", { name: "Shell" }));
    expect(screen.queryByRole("menu")).toBeNull();
    await waitFor(() =>
      expect(onOpenTerminal).toHaveBeenCalledWith({
        context: "kind-dev",
        namespace: "default",
        pod: "web-1",
        container: "app",
      }),
    );
  });

  it("does not ask when the only other container is a finished init step", async () => {
    getObjectMock.mockResolvedValue({
      object: {
        spec: { containers: [{ name: "app" }], initContainers: [{ name: "migrate" }] },
        status: {
          containerStatuses: [{ name: "app", state: { running: {} } }],
          initContainerStatuses: [{ name: "migrate", state: { terminated: { exitCode: 0 } } }],
        },
      },
    });
    const onOpenTerminal = vi.fn();
    render(<PodActions context="kind-dev" pod={pod} onOpenTerminal={onOpenTerminal} />);
    await waitFor(() => expect(getObjectMock).toHaveBeenCalled());

    fireEvent.click(screen.getByRole("button", { name: "Shell" }));
    expect(screen.queryByRole("menu")).toBeNull();
    expect(onOpenTerminal).toHaveBeenCalledWith({
      context: "kind-dev",
      namespace: "default",
      pod: "web-1",
      container: "app",
    });
  });

  it("lists every container in the menu, including ones that aren't running", async () => {
    getObjectMock.mockResolvedValue({
      object: {
        spec: {
          containers: [{ name: "mongodb" }, { name: "metrics" }],
          initContainers: [{ name: "generate-tls-certs" }],
        },
        status: {
          containerStatuses: [
            { name: "mongodb", state: { running: {} } },
            { name: "metrics", state: { running: {} } },
          ],
          initContainerStatuses: [{ name: "generate-tls-certs", state: { terminated: {} } }],
        },
      },
    });
    render(<PodActions context="kind-dev" pod={pod} onOpenTerminal={vi.fn()} />);
    await waitFor(() => expect(getObjectMock).toHaveBeenCalled());

    fireEvent.click(screen.getByRole("button", { name: "Shell" }));
    const items = await screen.findAllByRole("menuitem");
    expect(items.map((i) => i.textContent)).toEqual(["mongodb", "metrics", "generate-tls-certs"]);
    // The finished init container is shown but marked, not silently dropped.
    expect(titleOf(items[2])).toBe("Shell into generate-tls-certs (not running)");
  });

  it("re-reads the containers when the menu opens", async () => {
    // A pod's containers aren't fixed for its lifetime: they restart, and
    // ephemeral debug containers get attached (by the button next to this one).
    // A snapshot from whenever the drawer opened goes stale.
    const twoContainers = {
      object: {
        spec: { containers: [{ name: "mongodb" }, { name: "metrics" }] },
        status: {
          containerStatuses: [
            { name: "mongodb", state: { running: {} } },
            { name: "metrics", state: { running: {} } },
          ],
        },
      },
    };
    getObjectMock.mockResolvedValueOnce(twoContainers).mockResolvedValue({
      object: {
        spec: {
          containers: [{ name: "mongodb" }, { name: "metrics" }],
          ephemeralContainers: [{ name: "debugger-abc12" }],
        },
        status: {
          containerStatuses: [
            { name: "mongodb", state: { running: {} } },
            { name: "metrics", state: { running: {} } },
          ],
          ephemeralContainerStatuses: [{ name: "debugger-abc12", state: { running: {} } }],
        },
      },
    });
    render(<PodActions context="kind-dev" pod={pod} onOpenTerminal={vi.fn()} />);
    await waitFor(() => expect(getObjectMock).toHaveBeenCalledTimes(1));

    fireEvent.click(screen.getByRole("button", { name: "Shell" }));
    await waitFor(async () =>
      expect((await screen.findAllByRole("menuitem")).map((i) => i.textContent)).toEqual([
        "mongodb",
        "metrics",
        "debugger-abc12",
      ]),
    );
  });

  it("falls back to the old behaviour when the pod can't be read", async () => {
    // An RBAC-restricted get must not disable the shell button: without a
    // container the API server picks, exactly as before the picker existed.
    getObjectMock.mockResolvedValue({ error: "forbidden" });
    const onOpenTerminal = vi.fn();
    render(<PodActions context="kind-dev" pod={pod} onOpenTerminal={onOpenTerminal} />);
    await waitFor(() => expect(getObjectMock).toHaveBeenCalled());

    fireEvent.click(screen.getByRole("button", { name: "Shell" }));
    expect(screen.queryByRole("menu")).toBeNull();
    expect(onOpenTerminal).toHaveBeenCalledWith({
      context: "kind-dev",
      namespace: "default",
      pod: "web-1",
    });
  });

  it("confirms and deletes the pod", async () => {
    deletePodMock.mockResolvedValue({ deleted: true });
    const onDeleted = vi.fn();
    render(<PodActions context="kind-dev" pod={pod} onDeleted={onDeleted} />);
    fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    expect(screen.getByRole("dialog")).toBeDefined();
    // The dialog marks outside content aria-hidden, so the only reachable
    // "Delete" is now the dialog's confirm button.
    fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(deletePodMock).toHaveBeenCalledWith("kind-dev", "default", "web-1"));
    await waitFor(() => expect(onDeleted).toHaveBeenCalled());
  });

  it("attaches an ephemeral debug container and opens a terminal into it", async () => {
    debugPodMock.mockResolvedValue({ container: "debugger-abc12" });
    const onOpenTerminal = vi.fn();
    render(<PodActions context="kind-dev" pod={pod} onOpenTerminal={onOpenTerminal} />);
    fireEvent.click(screen.getByRole("button", { name: "Debug" }));
    // Confirm the dialog (default image busybox).
    fireEvent.click(screen.getByRole("button", { name: "Attach & open shell" }));
    await waitFor(() =>
      expect(debugPodMock).toHaveBeenCalledWith("kind-dev", "default", "web-1", "busybox", null),
    );
    await waitFor(() =>
      expect(onOpenTerminal).toHaveBeenCalledWith({
        context: "kind-dev",
        namespace: "default",
        pod: "web-1",
        container: "debugger-abc12",
      }),
    );
  });

  it("evicts the pod behind a confirm", async () => {
    evictPodMock.mockResolvedValue({ ok: true });
    const onDeleted = vi.fn();
    render(<PodActions context="kind-dev" pod={pod} onDeleted={onDeleted} />);
    fireEvent.click(screen.getByRole("button", { name: "Evict" }));
    // dialog open → its Evict button is the only reachable one
    fireEvent.click(screen.getByRole("button", { name: "Evict" }));
    await waitFor(() => expect(evictPodMock).toHaveBeenCalledWith("kind-dev", "default", "web-1"));
    await waitFor(() => expect(onDeleted).toHaveBeenCalled());
  });

  it("copies the kubectl get command for the pod via the Copy as kubectl menu", async () => {
    render(<PodActions context="kind-dev" pod={pod} onDeleted={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: "Copy as kubectl" }));
    fireEvent.click(await screen.findByRole("button", { name: "Copy get" }));
    expect(navigator.clipboard.writeText).toHaveBeenCalledWith(
      "kubectl get pods web-1 -n default --context kind-dev -o yaml",
    );
  });

  it("copies the kubectl describe command for the pod via the Copy as kubectl menu", async () => {
    render(<PodActions context="kind-dev" pod={pod} onDeleted={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: "Copy as kubectl" }));
    fireEvent.click(await screen.findByRole("button", { name: "Copy describe" }));
    expect(navigator.clipboard.writeText).toHaveBeenCalledWith(
      "kubectl describe pods web-1 -n default --context kind-dev",
    );
  });

  it("shows the kubectl equivalent in the delete confirm dialog and copies it", () => {
    render(<PodActions context="kind-dev" pod={pod} onDeleted={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    expect(screen.getByText("kubectl delete pods web-1 -n default --context kind-dev")).toBeDefined();
    fireEvent.click(screen.getByRole("button", { name: "Copy kubectl command" }));
    expect(navigator.clipboard.writeText).toHaveBeenCalledWith(
      "kubectl delete pods web-1 -n default --context kind-dev",
    );
  });

  it("shows a note instead of a (misleading) command in the evict confirm dialog", () => {
    render(<PodActions context="kind-dev" pod={pod} onDeleted={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: "Evict" }));
    expect(screen.queryByText(/^kubectl /)).toBeNull();
    expect(screen.getByText(/No single-line kubectl equivalent/)).toBeDefined();
  });

  it("does not report success when the clipboard write fails", async () => {
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText: vi.fn().mockRejectedValue(new Error("denied")) },
      configurable: true,
    });
    render(<PodActions context="kind-dev" pod={pod} onDeleted={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: "Copy as kubectl" }));
    fireEvent.click(await screen.findByRole("button", { name: "Copy get" }));
    await waitFor(() => expect(navigator.clipboard.writeText).toHaveBeenCalled());
    expect(notifyMock.success).not.toHaveBeenCalledWith("Copied kubectl command");
  });
});

describe("desiredReplicasFrom", () => {
  it("derives the desired count from each scalable kind's row shape", () => {
    // ReplicaSet rows carry `desired` directly.
    expect(desiredReplicasFrom({ desired: 4, ready: 2 })).toBe(4);
    // Deployment / StatefulSet rows encode it as "ready/total".
    expect(desiredReplicasFrom({ ready: "1/3" })).toBe(3);
    expect(desiredReplicasFrom({ ready: "0/0" })).toBe(0);
  });

  it("returns undefined when the row can't say", () => {
    expect(desiredReplicasFrom(undefined)).toBeUndefined();
    expect(desiredReplicasFrom({})).toBeUndefined();
    expect(desiredReplicasFrom({ ready: "not-a-fraction" })).toBeUndefined();
    expect(desiredReplicasFrom({ ready: 3 })).toBeUndefined();
  });

  it("matches the detail's row by name AND namespace", () => {
    // All-namespaces view: two workloads sharing a name must not seed
    // each other's counts.
    const rows = [
      { name: "web", namespace: "team-a", ready: "1/3" },
      { name: "web", namespace: "team-b", ready: "2/5" },
    ];
    expect(desiredReplicasForDetail(rows, "web", "team-b")).toBe(5);
    expect(desiredReplicasForDetail(rows, "web", "team-a")).toBe(3);
    expect(desiredReplicasForDetail(rows, "web", "team-c")).toBeUndefined();
    expect(desiredReplicasForDetail(rows, "api", "team-a")).toBeUndefined();
  });
});

describe("ResourceActions", () => {
  it("scales a deployment through the scale dialog", async () => {
    scaleResourceMock.mockResolvedValue({ ok: true });
    render(
      <ResourceActions
        context="kind-dev"
        kind="Deployment"
        namespace="default"
        name="web"
        onDeleted={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Scale" }));
    fireEvent.change(screen.getByLabelText("Replicas"), { target: { value: "3" } });
    // Outside content is aria-hidden while the dialog is open → the dialog's
    // Scale button is the only reachable one.
    fireEvent.click(screen.getByRole("button", { name: "Scale" }));
    await waitFor(() =>
      expect(scaleResourceMock).toHaveBeenCalledWith("kind-dev", "Deployment", "default", "web", 3),
    );
    // A success toast confirms the operation.
    await waitFor(() => expect(notifyMock.success).toHaveBeenCalledWith(expect.stringMatching(/Scaled web to 3/)));
  });

  it("defaults the scale dialog to the current replica count, on the slider too", () => {
    render(
      <ResourceActions
        context="kind-dev"
        kind="Deployment"
        namespace="default"
        name="web"
        currentReplicas={2}
        onDeleted={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Scale" }));

    expect((screen.getByLabelText("Replicas") as HTMLInputElement).value).toBe("2");
    const slider = screen.getByRole("slider");
    expect(slider.getAttribute("aria-valuenow")).toBe("2");
    // Range: 0 to max(currentReplicas * 2, 100).
    expect(slider.getAttribute("aria-valuemin")).toBe("0");
    expect(slider.getAttribute("aria-valuemax")).toBe("100");
    // The row already knew the count — no object fetch needed.
    expect(getObjectMock).not.toHaveBeenCalled();
  });

  it("fetches the live count when the row can't provide one (ReplicaSets)", async () => {
    getObjectMock.mockResolvedValue({ object: { spec: { replicas: 4 } } });
    render(
      <ResourceActions
        context="kind-dev"
        kind="ReplicaSet"
        namespace="default"
        name="web-abc123"
        onDeleted={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Scale" }));

    expect(getObjectMock).toHaveBeenCalledWith("kind-dev", "ReplicaSet", "default", "web-abc123");
    await waitFor(() =>
      expect((screen.getByLabelText("Replicas") as HTMLInputElement).value).toBe("4"),
    );
    expect(screen.getByRole("slider").getAttribute("aria-valuenow")).toBe("4");
  });

  it("never lets a late fetch overwrite what the user typed", async () => {
    let resolveFetch!: (v: unknown) => void;
    getObjectMock.mockReturnValue(new Promise((r) => (resolveFetch = r)));
    render(
      <ResourceActions
        context="kind-dev"
        kind="ReplicaSet"
        namespace="default"
        name="web-abc123"
        onDeleted={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Scale" }));
    fireEvent.change(screen.getByLabelText("Replicas"), { target: { value: "7" } });

    resolveFetch({ object: { spec: { replicas: 4 } } });
    // Give the resolved promise a tick to (wrongly) apply itself.
    await new Promise((r) => setTimeout(r, 0));
    expect((screen.getByLabelText("Replicas") as HTMLInputElement).value).toBe("7");
  });

  it("an intentionally CLEARED input also survives a late fetch", async () => {
    // Clearing the field to type a fresh value is an edit: restoring the
    // fetched count into the empty window would make the next digit append
    // (clearing 4 to type 0 must not become 40).
    let resolveFetch!: (v: unknown) => void;
    getObjectMock.mockReturnValue(new Promise((r) => (resolveFetch = r)));
    render(
      <ResourceActions
        context="kind-dev"
        kind="ReplicaSet"
        namespace="default"
        name="web-abc123"
        onDeleted={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Scale" }));
    fireEvent.change(screen.getByLabelText("Replicas"), { target: { value: "7" } });
    fireEvent.change(screen.getByLabelText("Replicas"), { target: { value: "" } });

    resolveFetch({ object: { spec: { replicas: 4 } } });
    await new Promise((r) => setTimeout(r, 0));
    expect((screen.getByLabelText("Replicas") as HTMLInputElement).value).toBe("");
  });

  it("a pending fetch is invalidated by ANY later dialog open, not only fetching ones", async () => {
    // The same component instance survives the detail switching resources.
    // Open Scale on a ReplicaSet (fetch pending), switch to a Deployment
    // whose row KNOWS its count (no fetch), open Scale again: the old fetch
    // resolving now must not overwrite the Deployment's seeded count.
    let resolveFetch!: (v: unknown) => void;
    getObjectMock.mockReturnValue(new Promise((r) => (resolveFetch = r)));
    const { rerender } = render(
      <ResourceActions
        context="kind-dev"
        kind="ReplicaSet"
        namespace="default"
        name="web-abc123"
        onDeleted={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Scale" }));
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    rerender(
      <ResourceActions
        context="kind-dev"
        kind="Deployment"
        namespace="default"
        name="api"
        currentReplicas={3}
        onDeleted={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Scale" }));
    expect((screen.getByLabelText("Replicas") as HTMLInputElement).value).toBe("3");

    resolveFetch({ object: { spec: { replicas: 9 } } });
    await new Promise((r) => setTimeout(r, 0));
    expect((screen.getByLabelText("Replicas") as HTMLInputElement).value).toBe("3");
  });

  it("widens the slider range past 100 for larger workloads", () => {
    render(
      <ResourceActions
        context="kind-dev"
        kind="Deployment"
        namespace="default"
        name="web"
        currentReplicas={80}
        onDeleted={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Scale" }));
    // The slider must always be able to reach and double the current count.
    expect(screen.getByRole("slider").getAttribute("aria-valuemax")).toBe("160");
  });

  it("keeps the numeric input and the slider in sync both ways", () => {
    render(
      <ResourceActions
        context="kind-dev"
        kind="Deployment"
        namespace="default"
        name="web"
        currentReplicas={2}
        onDeleted={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Scale" }));

    // Typing updates the slider…
    fireEvent.change(screen.getByLabelText("Replicas"), { target: { value: "5" } });
    const slider = screen.getByRole("slider");
    expect(slider.getAttribute("aria-valuenow")).toBe("5");

    // …and moving the slider updates the input.
    fireEvent.keyDown(slider, { key: "ArrowRight" });
    expect((screen.getByLabelText("Replicas") as HTMLInputElement).value).toBe("6");
  });

  it("scales to the slider-chosen value on confirm", async () => {
    scaleResourceMock.mockResolvedValue({ ok: true });
    render(
      <ResourceActions
        context="kind-dev"
        kind="Deployment"
        namespace="default"
        name="web"
        currentReplicas={2}
        onDeleted={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Scale" }));
    fireEvent.keyDown(screen.getByRole("slider"), { key: "ArrowRight" });
    fireEvent.click(screen.getByRole("button", { name: "Scale" }));
    await waitFor(() =>
      expect(scaleResourceMock).toHaveBeenCalledWith("kind-dev", "Deployment", "default", "web", 3),
    );
  });

  it("still rejects a non-integer typed into the input", async () => {
    scaleResourceMock.mockResolvedValue({ ok: true });
    render(
      <ResourceActions
        context="kind-dev"
        kind="Deployment"
        namespace="default"
        name="web"
        currentReplicas={2}
        onDeleted={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Scale" }));
    fireEvent.change(screen.getByLabelText("Replicas"), { target: { value: "1.5" } });
    fireEvent.click(screen.getByRole("button", { name: "Scale" }));
    expect(await screen.findByText(/non-negative replica count/)).toBeDefined();
    expect(scaleResourceMock).not.toHaveBeenCalled();
  });

  it("shows an actionable error toast (mapped, not raw) when an operation fails", async () => {
    scaleResourceMock.mockResolvedValue({ error: "forbidden" });
    render(
      <ResourceActions context="kind-dev" kind="Deployment" namespace="default" name="web" onDeleted={() => {}} />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Scale" }));
    fireEvent.change(screen.getByLabelText("Replicas"), { target: { value: "3" } });
    fireEvent.click(screen.getByRole("button", { name: "Scale" }));
    // The toast carries the friendly detail from describeError, not the raw "forbidden".
    await waitFor(() =>
      expect(notifyMock.error).toHaveBeenCalledWith("Failed to scale web", describeError("forbidden").detail),
    );
    expect(notifyMock.error).not.toHaveBeenCalledWith(expect.anything(), "forbidden");
    expect(notifyMock.success).not.toHaveBeenCalled();
  });

  it("triggers a rollout restart only after confirmation", async () => {
    rolloutRestartMock.mockResolvedValue({ ok: true });
    render(
      <ResourceActions
        context="kind-dev"
        kind="Deployment"
        namespace="default"
        name="web"
        onDeleted={() => {}}
      />,
    );
    // Clicking Restart opens a confirm dialog; it must NOT fire the mutation yet.
    fireEvent.click(screen.getByRole("button", { name: "Restart" }));
    expect(rolloutRestartMock).not.toHaveBeenCalled();
    // Confirm in the dialog (a second "Restart" button appears).
    const buttons = await screen.findAllByRole("button", { name: "Restart" });
    fireEvent.click(buttons[buttons.length - 1]);
    await waitFor(() =>
      expect(rolloutRestartMock).toHaveBeenCalledWith("kind-dev", "Deployment", "default", "web"),
    );
  });

  it("does not offer Scale/Restart for a non-workload kind", () => {
    render(
      <ResourceActions
        context="kind-dev"
        kind="ConfigMap"
        namespace="default"
        name="cm"
        onDeleted={() => {}}
      />,
    );
    expect(screen.queryByRole("button", { name: "Scale" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Restart" })).toBeNull();
    expect(screen.getByRole("button", { name: "Delete" })).toBeDefined();
  });

  it("triggers a CronJob run now with confirmation", async () => {
    cronjobTriggerNowMock.mockResolvedValue({ jobName: "nightly-123" });
    render(
      <ResourceActions
        context="kind-dev"
        kind="CronJob"
        namespace="ops"
        name="nightly"
        onDeleted={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Run now" }));
    fireEvent.click(await screen.findByRole("button", { name: "Run" }));
    await waitFor(() =>
      expect(cronjobTriggerNowMock).toHaveBeenCalledWith("kind-dev", "ops", "nightly"),
    );
  });

  it("shows Resume for a suspended CronJob and calls setSuspend(false)", async () => {
    cronjobSetSuspendMock.mockResolvedValue({ ok: true });
    render(
      <ResourceActions
        context="kind-dev"
        kind="CronJob"
        namespace="ops"
        name="nightly"
        cronjobSuspended
        onDeleted={() => {}}
      />,
    );
    expect(screen.queryByRole("button", { name: "Suspend" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Resume" }));
    // Confirm in the dialog (a second "Resume" button appears).
    const buttons = await screen.findAllByRole("button", { name: "Resume" });
    fireEvent.click(buttons[buttons.length - 1]);
    await waitFor(() =>
      expect(cronjobSetSuspendMock).toHaveBeenCalledWith("kind-dev", "ops", "nightly", false),
    );
  });

  it("shows Suspend for an active CronJob and calls setSuspend(true)", async () => {
    cronjobSetSuspendMock.mockResolvedValue({ ok: true });
    render(
      <ResourceActions
        context="kind-dev"
        kind="CronJob"
        namespace="ops"
        name="nightly"
        cronjobSuspended={false}
        onDeleted={() => {}}
      />,
    );
    // Clicking Suspend opens a confirm dialog; it must NOT fire the mutation yet.
    fireEvent.click(screen.getByRole("button", { name: "Suspend" }));
    expect(cronjobSetSuspendMock).not.toHaveBeenCalled();
    const buttons = await screen.findAllByRole("button", { name: "Suspend" });
    fireEvent.click(buttons[buttons.length - 1]);
    await waitFor(() =>
      expect(cronjobSetSuspendMock).toHaveBeenCalledWith("kind-dev", "ops", "nightly", true),
    );
  });

  it("copies the kubectl get command for the resource via the Copy as kubectl menu", async () => {
    render(
      <ResourceActions context="kind-dev" kind="Deployment" namespace="default" name="web" onDeleted={() => {}} />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Copy as kubectl" }));
    fireEvent.click(await screen.findByRole("button", { name: "Copy get" }));
    expect(navigator.clipboard.writeText).toHaveBeenCalledWith(
      "kubectl get deployments web -n default --context kind-dev -o yaml",
    );
  });

  it("copies the kubectl describe command for the resource via the Copy as kubectl menu", async () => {
    render(
      <ResourceActions context="kind-dev" kind="Deployment" namespace="default" name="web" onDeleted={() => {}} />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Copy as kubectl" }));
    fireEvent.click(await screen.findByRole("button", { name: "Copy describe" }));
    expect(navigator.clipboard.writeText).toHaveBeenCalledWith(
      "kubectl describe deployments web -n default --context kind-dev",
    );
  });

  it("shows the kubectl equivalent in the restart confirm dialog", () => {
    render(
      <ResourceActions context="kind-dev" kind="Deployment" namespace="default" name="web" onDeleted={() => {}} />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Restart" }));
    expect(
      screen.getByText("kubectl rollout restart deployments/web -n default --context kind-dev"),
    ).toBeDefined();
  });

  it("shows the kubectl equivalent in the delete confirm dialog and copies it", () => {
    render(
      <ResourceActions context="kind-dev" kind="Deployment" namespace="default" name="web" onDeleted={() => {}} />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    expect(screen.getByText("kubectl delete deployments web -n default --context kind-dev")).toBeDefined();
    fireEvent.click(screen.getByRole("button", { name: "Copy kubectl command" }));
    expect(navigator.clipboard.writeText).toHaveBeenCalledWith(
      "kubectl delete deployments web -n default --context kind-dev",
    );
  });

  it("shows the kubectl equivalent in the scale confirm dialog only once a valid replica count is entered", () => {
    render(
      <ResourceActions context="kind-dev" kind="Deployment" namespace="default" name="web" onDeleted={() => {}} />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Scale" }));
    expect(screen.queryByText(/kubectl scale/)).toBeNull();
    fireEvent.change(screen.getByLabelText("Replicas"), { target: { value: "5" } });
    expect(
      screen.getByText("kubectl scale deployments/web --replicas=5 -n default --context kind-dev"),
    ).toBeDefined();
    // An invalid (non-integer) entry hides the preview again rather than showing a bogus command.
    fireEvent.change(screen.getByLabelText("Replicas"), { target: { value: "abc" } });
    expect(screen.queryByText(/kubectl scale/)).toBeNull();
  });

  it("shows the kubectl equivalent in the cronjob trigger confirm dialog, timestamp-suffixed so a re-run doesn't collide", () => {
    render(
      <ResourceActions context="kind-dev" kind="CronJob" namespace="ops" name="nightly" onDeleted={() => {}} />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Run now" }));
    expect(
      screen.getByText(
        "kubectl create job --from=cronjob/nightly nightly-manual-$(date +%s) -n ops --context kind-dev",
      ),
    ).toBeDefined();
  });

  it("shows the kubectl equivalent in the cronjob suspend confirm dialog, quoted for cmd.exe too", () => {
    render(
      <ResourceActions
        context="kind-dev"
        kind="CronJob"
        namespace="ops"
        name="nightly"
        cronjobSuspended={false}
        onDeleted={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Suspend" }));
    expect(
      screen.getByText('kubectl patch cronjob nightly -p "{\\"spec\\":{\\"suspend\\":true}}" -n ops --context kind-dev'),
    ).toBeDefined();
  });

  it("shows the kubectl equivalent in the cronjob resume confirm dialog, quoted for cmd.exe too", () => {
    render(
      <ResourceActions
        context="kind-dev"
        kind="CronJob"
        namespace="ops"
        name="nightly"
        cronjobSuspended
        onDeleted={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Resume" }));
    expect(
      screen.getByText('kubectl patch cronjob nightly -p "{\\"spec\\":{\\"suspend\\":false}}" -n ops --context kind-dev'),
    ).toBeDefined();
  });

  it("does not report success when the clipboard write fails", async () => {
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText: vi.fn().mockRejectedValue(new Error("denied")) },
      configurable: true,
    });
    render(
      <ResourceActions context="kind-dev" kind="Deployment" namespace="default" name="web" onDeleted={() => {}} />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Copy as kubectl" }));
    fireEvent.click(await screen.findByRole("button", { name: "Copy get" }));
    await waitFor(() => expect(navigator.clipboard.writeText).toHaveBeenCalled());
    expect(notifyMock.success).not.toHaveBeenCalledWith("Copied kubectl command");
  });
});

describe("PodActions RBAC gating", () => {
  const prodPod = { ...pod, namespace: "prod" };

  it("disables the Delete control and explains why when the user can't delete", () => {
    vi.mocked(useAccess).mockReturnValue({
      allowed: () => false,
      reason: () => "RBAC: no rule",
      known: () => true,
      loading: false,
    });
    render(<PodActions context="kind-dev" pod={prodPod} onDeleted={() => {}} />);
    const del = screen.getByRole("button", { name: "Delete" });
    expect(isDisabled(del)).toBe(true);
    expect(titleOf(del)).toEqual(expect.stringContaining("permission to delete pods in prod"));
  });

  it("enables Delete when allowed", () => {
    vi.mocked(useAccess).mockReturnValue({
      allowed: () => true,
      reason: () => "",
      known: () => true,
      loading: false,
    });
    render(<PodActions context="kind-dev" pod={prodPod} onDeleted={() => {}} />);
    expect(isDisabled(screen.getByRole("button", { name: "Delete" }))).toBe(false);
  });

  it("disables Evict and explains why when the user can't evict", () => {
    vi.mocked(useAccess).mockReturnValue({
      allowed: () => false,
      reason: () => "RBAC: no rule",
      known: () => true,
      loading: false,
    });
    render(<PodActions context="kind-dev" pod={prodPod} onDeleted={() => {}} />);
    const evict = screen.getByRole("button", { name: "Evict" });
    expect(isDisabled(evict)).toBe(true);
    expect(titleOf(evict)).toEqual(expect.stringContaining("permission to create pods/eviction in prod"));
  });

  it("disables the pod Edit control and explains why when the user can't patch pods", () => {
    vi.mocked(useAccess).mockReturnValue({
      allowed: () => false,
      reason: () => "RBAC: no rule",
      known: () => true,
      loading: false,
    });
    render(<PodActions context="kind-dev" pod={prodPod} onDeleted={() => {}} onEdit={() => {}} />);
    const edit = screen.getByRole("button", { name: "Edit" });
    expect(isDisabled(edit)).toBe(true);
    expect(titleOf(edit)).toEqual(expect.stringContaining("permission to patch pods in prod"));
  });

  it("enables the pod Edit control when allowed", () => {
    vi.mocked(useAccess).mockReturnValue({
      allowed: () => true,
      reason: () => "",
      known: () => true,
      loading: false,
    });
    render(<PodActions context="kind-dev" pod={prodPod} onDeleted={() => {}} onEdit={() => {}} />);
    expect(isDisabled(screen.getByRole("button", { name: "Edit" }))).toBe(false);
  });
});

describe("ResourceActions RBAC gating", () => {
  it("disables the Delete control and explains why when the user can't delete", () => {
    vi.mocked(useAccess).mockReturnValue({
      allowed: () => false,
      reason: () => "RBAC: no rule",
      known: () => true,
      loading: false,
    });
    render(
      <ResourceActions
        context="kind-dev"
        kind="Deployment"
        namespace="prod"
        name="web"
        onDeleted={() => {}}
      />,
    );
    const del = screen.getByRole("button", { name: "Delete" });
    expect(isDisabled(del)).toBe(true);
    expect(titleOf(del)).toEqual(expect.stringContaining("permission to delete deployments in prod"));
  });

  it("enables Delete when allowed", () => {
    vi.mocked(useAccess).mockReturnValue({
      allowed: () => true,
      reason: () => "",
      known: () => true,
      loading: false,
    });
    render(
      <ResourceActions
        context="kind-dev"
        kind="Deployment"
        namespace="prod"
        name="web"
        onDeleted={() => {}}
      />,
    );
    expect(isDisabled(screen.getByRole("button", { name: "Delete" }))).toBe(false);
  });

  it("disables Scale when the user can't patch the scale subresource", () => {
    vi.mocked(useAccess).mockReturnValue({
      allowed: () => false,
      reason: () => "RBAC: no rule",
      known: () => true,
      loading: false,
    });
    render(
      <ResourceActions
        context="kind-dev"
        kind="Deployment"
        namespace="prod"
        name="web"
        onDeleted={() => {}}
      />,
    );
    const scale = screen.getByRole("button", { name: "Scale" });
    expect(isDisabled(scale)).toBe(true);
    expect(titleOf(scale)).toEqual(expect.stringContaining("permission to patch deployments/scale in prod"));
  });

  it("disables Restart when the user can't patch the workload", () => {
    vi.mocked(useAccess).mockReturnValue({
      allowed: () => false,
      reason: () => "RBAC: no rule",
      known: () => true,
      loading: false,
    });
    render(
      <ResourceActions
        context="kind-dev"
        kind="Deployment"
        namespace="prod"
        name="web"
        onDeleted={() => {}}
      />,
    );
    expect(isDisabled(screen.getByRole("button", { name: "Restart" }))).toBe(true);
  });

  it("disables Edit when the user can't patch the resource", () => {
    vi.mocked(useAccess).mockReturnValue({
      allowed: () => false,
      reason: () => "RBAC: no rule",
      known: () => true,
      loading: false,
    });
    render(
      <ResourceActions
        context="kind-dev"
        kind="Deployment"
        namespace="prod"
        name="web"
        onDeleted={() => {}}
        onEdit={() => {}}
      />,
    );
    expect(isDisabled(screen.getByRole("button", { name: "Edit" }))).toBe(true);
  });

  it("disables cronjob Run now / Suspend when denied", () => {
    vi.mocked(useAccess).mockReturnValue({
      allowed: () => false,
      reason: () => "RBAC: no rule",
      known: () => true,
      loading: false,
    });
    render(
      <ResourceActions
        context="kind-dev"
        kind="CronJob"
        namespace="ops"
        name="nightly"
        onDeleted={() => {}}
      />,
    );
    expect(isDisabled(screen.getByRole("button", { name: "Run now" }))).toBe(true);
    expect(isDisabled(screen.getByRole("button", { name: "Suspend" }))).toBe(true);
  });

  it("leaves controls enabled for an unknown/CRD kind, regardless of RBAC checks", () => {
    vi.mocked(useAccess).mockReturnValue({
      allowed: () => false,
      reason: () => "RBAC: no rule",
      known: () => true,
      loading: false,
    });
    render(
      <ResourceActions
        context="kind-dev"
        kind="Widget"
        namespace="prod"
        name="thing"
        onDeleted={() => {}}
      />,
    );
    expect(isDisabled(screen.getByRole("button", { name: "Delete" }))).toBe(false);
  });
});
