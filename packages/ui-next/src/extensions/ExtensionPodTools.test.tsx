import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@srelens/core", async (original) => ({
  ...(await original<typeof import("@srelens/core")>()),
  isTauri: vi.fn(() => true),
  listContexts: vi.fn(),
  extensionPods: vi.fn(),
  openExtensionView: vi.fn(),
}));
vi.mock("./inventoryStore", async (original) => ({
  ...(await original<typeof import("./inventoryStore")>()),
  useExtensions: vi.fn(),
}));

import {
  extensionPods,
  isTauri,
  listContexts,
  openExtensionView,
  type ExtensionStreamHandlers,
  type ExtensionStreamRequest,
  type ExtensionView,
  type InstalledExtension,
} from "@srelens/core";
import { useExtensions } from "./inventoryStore";
import { ExtensionPodSlot, podBindingsFor } from "./ExtensionPodTools";

const COMMAND = ["cmctl", "status", "certificate", "--all-namespaces"];

/** A cert-manager app reaching the pods of the Deployments it reads (#567). */
const plugin = {
  manifest: {
    id: "org.example.certmanager",
    name: "cert-manager",
    version: "0.1.0",
    srelensApiVersion: "^0.5",
    kind: "declarative",
    permissions: [
      "k8s.listDeployments",
      { capability: "k8s.streamLogs", namespaces: ["cert-manager"] },
      "k8s.exec",
      "k8s.portForward",
    ],
    capabilities: [
      { name: "controllers", title: "Controllers", target: "k8s.listDeployments", arguments: {}, inputs: ["context", "namespace"] },
      { name: "controllerLogs", title: "Controller logs", target: "k8s.streamLogs", arguments: { resource: "controllers" }, inputs: [] },
      { name: "namespaceLogs", title: "Namespace logs", target: "k8s.streamLogs", arguments: {}, inputs: [] },
      {
        name: "status", title: "cmctl status", target: "k8s.exec", inputs: [],
        arguments: { resource: "controllers", container: "controller", command: COMMAND },
      },
      { name: "metrics", title: "Metrics", target: "k8s.portForward", arguments: { resource: "controllers", port: 9402 }, inputs: [] },
    ],
    contributions: { pages: [], detailTabs: [], detailLinks: [] },
  },
  enabled: true,
  revision: 3,
  grants: ["k8s.listDeployments", "k8s.streamLogs", "k8s.exec", "k8s.portForward"],
  source: "local",
} as unknown as InstalledExtension;

const deployment = {
  apiVersion: "apps/v1",
  kind: "Deployment",
  metadata: { name: "web", namespace: "team" },
};

/** A scripted app stream view: records every open and hands each its handlers. */
function fakeView() {
  const opened: Array<{ request: ExtensionStreamRequest; handlers: ExtensionStreamHandlers; cancel: ReturnType<typeof vi.fn> }> = [];
  const close = vi.fn(async () => {});
  const view: ExtensionView = {
    view: "org.example.certmanager/pods#1",
    open: vi.fn(async (request: ExtensionStreamRequest, handlers: ExtensionStreamHandlers) => {
      const cancel = vi.fn(async () => {});
      opened.push({ request, handlers: handlers as ExtensionStreamHandlers, cancel });
      return { stream: `s-${opened.length}`, cancel };
    }) as ExtensionView["open"],
    close,
  };
  return { view, opened, close };
}

let stream: ReturnType<typeof fakeView>;

beforeEach(() => {
  stream = fakeView();
  vi.mocked(openExtensionView).mockReset().mockReturnValue(stream.view);
  vi.mocked(isTauri).mockReturnValue(true);
  vi.mocked(listContexts).mockReset().mockResolvedValue({ contexts: [] } as never);
  vi.mocked(useExtensions).mockReturnValue({ status: "ready", data: { plugins: [plugin] }, reload: vi.fn() } as never);
  vi.mocked(extensionPods).mockReset().mockResolvedValue({
    pods: [
      { name: "web-1", namespace: "team", containers: ["controller", "sidecar"], phase: "Running", ready: true },
      { name: "web-2", namespace: "team", containers: ["controller", "sidecar"], phase: "Pending", ready: false },
    ],
    scope: "pods selected by Deployment web",
  });
});

afterEach(cleanup);

async function openTools(resource: unknown = deployment) {
  render(<ExtensionPodSlot context="kind-dev" resource={resource} />);
  return screen.findByRole("region", { name: "cert-manager pod tools" });
}

describe("which bindings a resource offers", () => {
  it("offers a binding scoped by a reader of this kind, and a namespace grant only on its pods", () => {
    const names = (kind: string, namespace: string) =>
      podBindingsFor(plugin.manifest, kind, namespace, "x").map(({ binding }) => binding.name);
    expect(names("apps/Deployment", "team")).toEqual(["controllerLogs", "status", "metrics"]);
    expect(names("apps/StatefulSet", "team")).toEqual([]);
    expect(names("/Pod", "cert-manager")).toEqual(["namespaceLogs"]);
    expect(names("/Pod", "team")).toEqual([]);
  });
});

describe("the pod tools on a Deployment (#567)", () => {
  it("lists the pods the host says the binding may reach, in its words", async () => {
    const tools = await openTools();
    await within(tools).findByRole("list", { name: "Pods Controller logs may reach" });
    expect(extensionPods).toHaveBeenCalledWith({
      id: "org.example.certmanager", revision: 3, capability: "controllerLogs",
      context: "kind-dev", namespace: "team", name: "web",
    });
    expect(tools.textContent).toContain("Pods selected by Deployment web.");
    const row = within(tools).getByRole("listitem", { name: "Pod web-2" });
    expect(row.textContent).toContain("Pending · not ready");
    expect(within(row).getByRole("button", { name: "Logs · Controller logs (web-2)" })).toBeTruthy();
  });

  it("says what failed when the pods cannot be listed, and retries", async () => {
    vi.mocked(extensionPods).mockRejectedValueOnce(new Error("pods is forbidden"));
    const tools = await openTools();
    const alert = await within(tools).findByRole("alert");
    expect(alert.textContent).toContain("Could not list the pods this app may reach");
    fireEvent.click(within(alert).getByRole("button", { name: "Retry" }));
    await within(tools).findByRole("list", { name: "Pods Controller logs may reach" });
  });

  it("follows a container's logs through the app's logs source and draws its lines", async () => {
    const tools = await openTools();
    fireEvent.click(await within(tools).findByRole("button", { name: "Logs · Controller logs (web-1)" }));
    await waitFor(() => expect(stream.opened).toHaveLength(1));
    expect(stream.opened[0].request).toMatchObject({
      id: "org.example.certmanager", revision: 3, context: "kind-dev", namespace: "team",
      source: { kind: "logs", capability: "controllerLogs", name: "web", pod: "web-1", container: "controller", tailLines: 200 },
    });
    act(() => {
      stream.opened[0].handlers.onData({ event: "status", source: "web-1/controller", status: "live" }, 1);
      stream.opened[0].handlers.onData({ event: "lines", lines: [{ source: "web-1/controller", line: "certificate issued" }] }, 2);
    });
    const log = await within(tools).findByRole("log", { name: "Logs of web-1/controller" });
    await waitFor(() => expect(log.textContent).toContain("certificate issued"));
    // It scrolls both ways, so a keyboard must be able to reach it.
    expect(log.getAttribute("tabindex")).toBe("0");
    expect(within(tools).getByText("Following")).toBeTruthy();
    // A failure reads as one.
    act(() => stream.opened[0].handlers.onEnd?.({ type: "error", code: "source", message: "pods \"web-1\" is forbidden" }));
    expect(await within(tools).findByText(/The stream failed: pods "web-1" is forbidden/)).toBeTruthy();
  });

  it("runs nothing until the host confirmation naming the pod, container and exact command is approved", async () => {
    const tools = await openTools();
    fireEvent.click(await within(tools).findByRole("button", { name: "Run · cmctl status (web-1)" }));
    const review = within(tools).getByRole("region", { name: "Review command" });
    expect(within(review).getByTestId("host-confirm-question").textContent).toBe(
      "Run this app's command in Pod team/web-1 in cluster kind-dev?",
    );
    expect(within(review).getByTestId("host-confirm-target").textContent).toBe("team/web-1");
    expect(within(review).getByTestId("host-confirm-container").textContent).toBe("controller");
    expect(within(review).getByTestId("host-confirm-command").textContent).toBe(COMMAND.join(" "));
    expect(within(review).getByTestId("host-confirm-impact").textContent).toBe("High impact");
    expect(within(review).getByTestId("host-confirm-app-name").textContent).toBe("cert-manager");
    fireEvent.click(within(review).getByRole("button", { name: "Cancel" }));
    expect(within(tools).queryByRole("region", { name: "Review command" })).toBeNull();
    expect(stream.opened).toHaveLength(0);
    // Focus goes back where it came from, not to the page.
    expect(document.activeElement).toBe(within(tools).getByRole("button", { name: "Run · cmctl status (web-1)" }));

    fireEvent.click(within(tools).getByRole("button", { name: "Run · cmctl status (web-1)" }));
    fireEvent.click(within(tools).getByRole("button", { name: "Run command" }));
    await waitFor(() => expect(stream.opened).toHaveLength(1));
    // What was confirmed is what is sent, field for field.
    expect(stream.opened[0].request.source).toEqual({
      kind: "exec", capability: "status", name: "web", pod: "web-1", container: "controller",
      confirmed: { pod: "web-1", container: "controller", command: COMMAND },
    });
    act(() => {
      stream.opened[0].handlers.onData({ event: "output", chunks: [{ stream: "stdout", text: "Ready: False\n" }, { stream: "stderr", text: "expired\n" }] }, 1);
      stream.opened[0].handlers.onData({ event: "exit", code: 3 }, 2);
    });
    const output = await within(tools).findByLabelText("Command output");
    expect(output.textContent).toBe("Ready: False\nstderr expired\n");
    expect(output.getAttribute("tabindex")).toBe("0");
    expect(within(tools).getByText("The command exited with code 3.")).toBeTruthy();
  });

  it("says why the host refused a command, rather than showing an empty run", async () => {
    const tools = await openTools();
    vi.mocked(stream.view.open).mockRejectedValueOnce(new Error("Pod web-1 is not selected by Deployment web in team"));
    fireEvent.click(await within(tools).findByRole("button", { name: "Run · cmctl status (web-1)" }));
    fireEvent.click(within(tools).getByRole("button", { name: "Run command" }));
    expect(await within(tools).findByText(/The host refused the command: Pod web-1 is not selected/)).toBeTruthy();
  });

  it("forwards through a port the host picks, and stops it", async () => {
    const tools = await openTools();
    fireEvent.click(await within(tools).findByRole("button", { name: "Forward · Metrics (web-1)" }));
    await waitFor(() => expect(stream.opened).toHaveLength(1));
    expect(stream.opened[0].request.source).toEqual({ kind: "portForward", capability: "metrics", name: "web", pod: "web-1" });
    expect(within(tools).getByText("Opening a local port…")).toBeTruthy();
    act(() => stream.opened[0].handlers.onData({ event: "ready", localPort: 54321, pod: "web-1", port: 9402 }, 1));
    const status = await within(tools).findByText(/Forwarding/);
    expect(status.textContent).toContain("127.0.0.1:54321");
    expect(status.textContent).toContain("web-1 port 9402");
    // A connection the cluster refused is said, beside where the forward still listens.
    act(() =>
      stream.opened[0].handlers.onData({ event: "connectionFailed", count: 2, message: "pods \"web-1\" is forbidden" }, 2),
    );
    expect(
      (await within(tools).findByText(/2 connections through it failed/)).textContent,
    ).toContain('the last because: pods "web-1" is forbidden');
    expect(within(tools).getByText(/Forwarding/).textContent).toContain("127.0.0.1:54321");
    fireEvent.click(within(tools).getByRole("button", { name: "Stop forwarding" }));
    await waitFor(() => expect(stream.opened[0].cancel).toHaveBeenCalled());
  });

  // Found by looking: a group whose pods were listed through its logs binding said
  // "No Service sends to a pod this app may reach" — a claim about the cluster made
  // from an answer that never carries Services.
  it("lists the Services a forward through a Service may use, from an answer that carries them", async () => {
    const withService = {
      ...plugin,
      manifest: {
        ...plugin.manifest,
        capabilities: [
          ...plugin.manifest.capabilities,
          { name: "webhook", title: "Webhook", target: "k8s.portForward", inputs: [], arguments: { resource: "controllers", port: 443, service: true } },
        ],
      },
    } as unknown as InstalledExtension;
    vi.mocked(useExtensions).mockReturnValue({ status: "ready", data: { plugins: [withService] }, reload: vi.fn() } as never);
    vi.mocked(extensionPods).mockResolvedValue({
      pods: [{ name: "web-1", namespace: "team", containers: ["controller"], phase: "Running", ready: true }],
      services: [{ name: "web", port: 443 }],
      scope: "pods selected by Deployment web",
    });
    const tools = await openTools();
    const services = await within(tools).findByRole("list", { name: "Services to forward through" });
    expect(extensionPods).toHaveBeenCalledWith(expect.objectContaining({ capability: "webhook", name: "web" }));
    expect(services.textContent).toContain("web port 443");
    fireEvent.click(within(services).getByRole("button", { name: "Forward · Webhook" }));
    await waitFor(() => expect(stream.opened).toHaveLength(1));
    expect(stream.opened[0].request.source).toEqual({ kind: "portForward", capability: "webhook", name: "web", service: "web" });
    // The pods are the same scope's, and still offered.
    expect(within(tools).getByRole("listitem", { name: "Pod web-1" })).toBeTruthy();
  });

  // The Inspector reuses its slot as the reader moves between resources: a review
  // or a session opened on one must never show under the next.
  it("starts over, with a new view, when the resource it is on changes", async () => {
    const view = render(<ExtensionPodSlot context="kind-dev" resource={deployment} />);
    const tools = await screen.findByRole("region", { name: "cert-manager pod tools" });
    fireEvent.click(await within(tools).findByRole("button", { name: "Run · cmctl status (web-1)" }));
    expect(within(tools).getByRole("region", { name: "Review command" })).toBeTruthy();
    view.rerender(
      <ExtensionPodSlot context="kind-dev" resource={{ ...deployment, metadata: { name: "api", namespace: "team" } }} />,
    );
    await screen.findByRole("region", { name: "cert-manager pod tools" });
    expect(screen.queryByRole("region", { name: "Review command" })).toBeNull();
    expect(stream.close).toHaveBeenCalledOnce();
    expect(openExtensionView).toHaveBeenCalledTimes(2);
    await waitFor(() =>
      expect(extensionPods).toHaveBeenLastCalledWith(expect.objectContaining({ name: "api" })),
    );
  });

  it("starts over, with a new view, when the same resource is shown on another cluster", async () => {
    const view = render(<ExtensionPodSlot context="kind-dev" resource={deployment} />);
    const tools = await screen.findByRole("region", { name: "cert-manager pod tools" });
    fireEvent.click(await within(tools).findByRole("button", { name: "Forward · Metrics (web-1)" }));
    await waitFor(() => expect(stream.opened).toHaveLength(1));
    act(() => stream.opened[0].handlers.onData({ event: "ready", localPort: 54321, pod: "web-1", port: 9402 }, 1));
    await within(tools).findByText(/Forwarding/);
    view.rerender(<ExtensionPodSlot context="kind-prod" resource={deployment} />);
    await screen.findByRole("region", { name: "cert-manager pod tools" });
    // Nothing of the first cluster's forward is left under the second.
    expect(screen.queryByText(/Forwarding/)).toBeNull();
    expect(stream.close).toHaveBeenCalledOnce();
    expect(openExtensionView).toHaveBeenCalledTimes(2);
    await waitFor(() =>
      expect(extensionPods).toHaveBeenLastCalledWith(expect.objectContaining({ context: "kind-prod", name: "web" })),
    );
  });

  it("closes its view when it goes away, which ends every stream and forward it opened", async () => {
    const view = render(<ExtensionPodSlot context="kind-dev" resource={deployment} />);
    await screen.findByRole("region", { name: "cert-manager pod tools" });
    view.unmount();
    expect(stream.close).toHaveBeenCalledOnce();
  });

  it("says on the web that app streams run in the desktop app, and lists nothing", async () => {
    vi.mocked(isTauri).mockReturnValue(false);
    const tools = await openTools();
    expect(tools.textContent).toContain("The web app does not run app streams yet");
    expect(extensionPods).not.toHaveBeenCalled();
  });
});

describe("the pod tools on a pod in a granted namespace", () => {
  const pod = (namespace: string) => ({ apiVersion: "v1", kind: "Pod", metadata: { name: "webhook-1", namespace } });

  it("offers the namespace-scoped bindings for that pod only", async () => {
    vi.mocked(extensionPods).mockResolvedValue({
      pods: [
        { name: "webhook-1", namespace: "cert-manager", containers: ["webhook"], phase: "Running", ready: true },
        { name: "other-1", namespace: "cert-manager", containers: ["x"], phase: "Running", ready: true },
      ],
      scope: "pods in namespace cert-manager",
    });
    const tools = await openTools(pod("cert-manager"));
    await within(tools).findByRole("list", { name: "Pods Namespace logs may reach" });
    expect(extensionPods).toHaveBeenCalledWith(expect.objectContaining({ capability: "namespaceLogs", namespace: "cert-manager" }));
    expect(vi.mocked(extensionPods).mock.calls[0][0].name).toBeUndefined();
    expect(within(tools).queryByRole("listitem", { name: "Pod other-1" })).toBeNull();
    expect(within(tools).getByRole("listitem", { name: "Pod webhook-1" })).toBeTruthy();
  });

  it("offers nothing on a pod outside the grant", () => {
    render(<ExtensionPodSlot context="kind-dev" resource={pod("team")} />);
    expect(screen.queryByRole("region", { name: "cert-manager pod tools" })).toBeNull();
  });
});

describe("the pod tools when a listing fails", () => {
  it("says the installed apps could not be listed, with a retry, rather than showing none", async () => {
    const reload = vi.fn();
    vi.mocked(useExtensions).mockReturnValue({ status: "error", error: "extensions.json is unreadable", reload } as never);
    render(<ExtensionPodSlot context="kind-dev" resource={deployment} />);
    const tools = await screen.findByRole("region", { name: "App pod tools" });
    const alert = within(tools).getByRole("alert");
    expect(alert.textContent).toContain("Could not list the installed apps");
    fireEvent.click(within(alert).getByRole("button", { name: "Retry" }));
    expect(reload).toHaveBeenCalledOnce();
  });

  it("says the clusters could not be listed when an app limited to some has pod tools here", async () => {
    vi.mocked(useExtensions).mockReturnValue({
      status: "ready",
      data: { plugins: [{ ...plugin, contexts: ["/kube/config#kind-dev"] }] },
      reload: vi.fn(),
    } as never);
    vi.mocked(listContexts).mockResolvedValue({ error: "kubeconfig is unreadable" } as never);
    render(<ExtensionPodSlot context="kind-dev" resource={deployment} />);
    const tools = await screen.findByRole("region", { name: "App pod tools" });
    expect(within(tools).getByRole("alert").textContent).toContain("Could not list the clusters");
    expect(screen.queryByRole("region", { name: "cert-manager pod tools" })).toBeNull();
    // Once the clusters list, and this is one the app is enabled for, its tools are there.
    vi.mocked(listContexts).mockResolvedValue({ contexts: [{ name: "kind-dev", key: "/kube/config#kind-dev" }] } as never);
    fireEvent.click(within(tools).getByRole("button", { name: "Retry" }));
    await screen.findByRole("region", { name: "cert-manager pod tools" });
    expect(screen.queryByRole("region", { name: "App pod tools" })).toBeNull();
  });
});
