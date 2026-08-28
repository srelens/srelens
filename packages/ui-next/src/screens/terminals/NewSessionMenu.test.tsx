import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

/**
 * `isTauri` moved per-test the way `AppLog.test.tsx` does: it is a function
 * this component calls directly, so flipping it on the mocked `@srelens/core`
 * module between renders is enough — no second mocking scheme is needed for
 * one function.
 *
 * `podContainerChoices`, `execCandidates` and `defaultContainer` stay REAL:
 * "execCandidates decides which containers are offered" is a property of
 * THOSE functions, and a test that stubbed them would only prove the menu
 * calls a stub. Only the network-shaped calls are doubled.
 */
const core = vi.hoisted(() => ({
  isTauri: vi.fn(() => true),
  listNamespaces: vi.fn(),
  listPods: vi.fn(),
  listNodes: vi.fn(),
  getObject: vi.fn(),
  createNodeDebugPod: vi.fn(),
}));
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  ...core,
}));

/** The session store is `../../lib/sessions`, not core — see `sessions.ts`'s
 *  own note on why it lives in `ui-next`. Doubled wholesale: this file's job
 *  is proving the menu calls it with the right request, not what the store
 *  does with one. */
const sessions = vi.hoisted(() => ({
  startPodSession: vi.fn(),
  startLocalSession: vi.fn(),
}));
vi.mock("../../lib/sessions", () => sessions);

import { NewSessionMenu } from "./NewSessionMenu";

const CONTEXT = "prod-eu";
const NAMESPACE = "checkout";
const POD = "checkout-api-5c8b7f2d9-mk3wl";

/** An app container and a container-shaped finished init container — the
 *  fixture `podContainers.test.ts` uses to prove `execCandidates` drops the
 *  one that already exited. */
const POD_OBJECT = {
  metadata: { name: POD, namespace: NAMESPACE },
  spec: {
    containers: [{ name: "api" }, { name: "proxy" }],
    initContainers: [{ name: "migrate" }],
  },
  status: {
    containerStatuses: [
      { name: "api", state: { running: {} } },
      { name: "proxy", state: { running: {} } },
    ],
    initContainerStatuses: [{ name: "migrate", state: { terminated: {} } }],
  },
};

let onStarted: ReturnType<typeof vi.fn>;
let onClose: ReturnType<typeof vi.fn>;

beforeEach(() => {
  vi.clearAllMocks();
  core.isTauri.mockReturnValue(true);
  core.listNamespaces.mockResolvedValue({ namespaces: [NAMESPACE, "payments"] });
  core.listPods.mockResolvedValue({ pods: [{ name: POD, namespace: NAMESPACE }] });
  core.listNodes.mockResolvedValue({ nodes: [{ name: "eu-w4-c3-standard-a1" }] });
  core.getObject.mockResolvedValue({ object: POD_OBJECT });
  core.createNodeDebugPod.mockResolvedValue({ namespace: "kube-system", pod: "node-debug-abc12" });
  sessions.startPodSession.mockResolvedValue(9);
  sessions.startLocalSession.mockResolvedValue(11);
  onStarted = vi.fn();
  onClose = vi.fn();
});

/** The namespace select starts prefilled from the `namespace` prop, so this
 *  only has to wait for it to load and drive the pod and container selects
 *  underneath it. */
async function pickPodAndContainer(user: ReturnType<typeof userEvent.setup>) {
  await waitFor(() => expect((screen.getByLabelText("Pod") as HTMLSelectElement).disabled).toBe(false));
  await user.selectOptions(screen.getByLabelText("Pod"), POD);
  await waitFor(() => expect((screen.getByLabelText("Container") as HTMLSelectElement).disabled).toBe(false));
}

describe("NewSessionMenu", () => {
  it("offers a local shell on the desktop", () => {
    core.isTauri.mockReturnValue(true);
    render(
      <NewSessionMenu context={CONTEXT} namespace={NAMESPACE} onStarted={onStarted} onClose={onClose} />,
    );

    expect(screen.getByRole("button", { name: "Local shell" })).not.toBeNull();
  });

  it("does not offer a local shell in the browser, and says once why", () => {
    core.isTauri.mockReturnValue(false);
    render(
      <NewSessionMenu context={CONTEXT} namespace={NAMESPACE} onStarted={onStarted} onClose={onClose} />,
    );

    expect(screen.queryByRole("button", { name: "Local shell" })).toBeNull();
    // Said once for the whole menu — not a sentence that would have to repeat
    // itself for Pod and again for Node.
    expect(
      screen.getAllByText(/does not offer|only session kind the browser build does not offer/).length,
    ).toBe(1);
  });

  it("offers only the containers execCandidates keeps, dropping the finished init container", async () => {
    const user = userEvent.setup();
    render(
      <NewSessionMenu context={CONTEXT} namespace={NAMESPACE} onStarted={onStarted} onClose={onClose} />,
    );

    await pickPodAndContainer(user);

    const containerSelect = screen.getByLabelText("Container") as HTMLSelectElement;
    const optionLabels = within(containerSelect)
      .getAllByRole("option")
      .map((o) => o.textContent);
    expect(optionLabels).toContain("api");
    expect(optionLabels).toContain("proxy");
    expect(optionLabels).not.toContain("migrate");
  });

  it("starting a pod session sends the chosen container", async () => {
    const user = userEvent.setup();
    render(
      <NewSessionMenu context={CONTEXT} namespace={NAMESPACE} onStarted={onStarted} onClose={onClose} />,
    );

    await pickPodAndContainer(user);
    await user.selectOptions(screen.getByLabelText("Container"), "proxy");
    await user.click(screen.getByRole("button", { name: "Start session" }));

    await waitFor(() => expect(sessions.startPodSession).toHaveBeenCalledTimes(1));
    expect(sessions.startPodSession).toHaveBeenCalledWith(
      expect.objectContaining({
        context: CONTEXT,
        namespace: NAMESPACE,
        pod: POD,
        container: "proxy",
      }),
    );
    expect(onStarted).toHaveBeenCalledWith(9);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("starting a node session creates its debug pod, then attaches a shell tagged node", async () => {
    const user = userEvent.setup();
    render(
      <NewSessionMenu context={CONTEXT} namespace={NAMESPACE} onStarted={onStarted} onClose={onClose} />,
    );

    await user.click(screen.getByRole("button", { name: "Node" }));
    await waitFor(() => expect((screen.getByLabelText("Node") as HTMLSelectElement).disabled).toBe(false));
    await user.selectOptions(screen.getByLabelText("Node"), "eu-w4-c3-standard-a1");
    await user.click(screen.getByRole("button", { name: "Start session" }));

    await waitFor(() => expect(sessions.startPodSession).toHaveBeenCalledTimes(1));
    expect(core.createNodeDebugPod).toHaveBeenCalledWith(CONTEXT, "eu-w4-c3-standard-a1");
    expect(sessions.startPodSession).toHaveBeenCalledWith(
      expect.objectContaining({
        context: CONTEXT,
        namespace: "kube-system",
        pod: "node-debug-abc12",
        kind: "node",
      }),
    );
    expect(onStarted).toHaveBeenCalledWith(9);
  });

  /**
   * A local shell is the one kind whose readiness needs no cluster listing to
   * come back, so it was the one kind that could start with no cluster at all —
   * `startLocalSession({ context: "" })`, under a hint promising a KUBECONFIG
   * "scoped to the active cluster". Nothing scopes it, and the shell that comes
   * up talks to whatever the reader's own kubeconfig points at.
   */
  it("will not start a local shell with no cluster to scope it to", async () => {
    const user = userEvent.setup();
    render(<NewSessionMenu context="" onStarted={onStarted} onClose={onClose} />);

    await user.click(screen.getByRole("button", { name: "Local shell" }));
    const start = screen.getByRole("button", { name: "Start session" });
    expect(start.hasAttribute("disabled")).toBe(true);

    await user.click(start);
    expect(sessions.startLocalSession).not.toHaveBeenCalled();
    // And it does not claim a scoping it cannot perform.
    expect(screen.queryByText(/scoped to the active cluster/)).toBeNull();
    expect(screen.getByText(/no cluster in focus/i)).toBeTruthy();
  });

  it("starting a local session on the desktop scopes it to the context", async () => {
    const user = userEvent.setup();
    render(
      <NewSessionMenu context={CONTEXT} namespace={NAMESPACE} onStarted={onStarted} onClose={onClose} />,
    );

    await user.click(screen.getByRole("button", { name: "Local shell" }));
    await user.click(screen.getByRole("button", { name: "Start session" }));

    await waitFor(() => expect(sessions.startLocalSession).toHaveBeenCalledTimes(1));
    expect(sessions.startLocalSession).toHaveBeenCalledWith(
      expect.objectContaining({ context: CONTEXT }),
    );
    expect(onStarted).toHaveBeenCalledWith(11);
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
