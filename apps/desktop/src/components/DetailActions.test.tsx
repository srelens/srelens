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

import { PodActions, ResourceActions } from "./DetailActions";

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
