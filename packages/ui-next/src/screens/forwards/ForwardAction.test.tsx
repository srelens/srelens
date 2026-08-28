import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

/**
 * `forwardAddress` and `toKubectl` stay REAL, the same arrangement
 * `NewForwardDialog.test.tsx` uses: this file drives the dialog through the
 * door rather than replacing it, because the door's whole job is what the
 * dialog is told.
 */
const platform = vi.hoisted(() => ({ isTauri: vi.fn(() => true) }));
vi.mock("@srelens/core/platform", async (orig) => ({
  ...(await orig<typeof import("@srelens/core/platform")>()),
  isTauri: platform.isTauri,
}));

const store = vi.hoisted(() => ({
  list: [] as unknown[],
  listeners: new Set<() => void>(),
}));
const core = vi.hoisted(() => ({
  startPortForward: vi.fn(),
  openExternal: vi.fn(),
  listNamespaces: vi.fn(),
  listPods: vi.fn(),
  listServices: vi.fn(),
  notify: { success: vi.fn(), error: vi.fn(), info: vi.fn() },
}));
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  getForwards: () => store.list,
  subscribeForwards: (l: () => void) => {
    store.listeners.add(l);
    return () => store.listeners.delete(l);
  },
  ...core,
}));

import { ForwardAction } from "./ForwardAction";

const CONTEXT = "prod-eu";
const ELSEWHERE = "stage-eu";
const NAMESPACE = "checkout";

beforeEach(() => {
  vi.clearAllMocks();
  platform.isTauri.mockReturnValue(true);
  core.listNamespaces.mockResolvedValue({ namespaces: [NAMESPACE] });
  core.listServices.mockResolvedValue({ services: [{ name: "checkout-api", namespace: NAMESPACE }] });
  core.listPods.mockResolvedValue({ pods: [] });
  core.startPortForward.mockResolvedValue({ id: 7, localPort: 9090 });
  core.openExternal.mockResolvedValue(undefined);
  store.list = [];
  store.listeners.clear();
});

function draw(context: string) {
  return render(
    <ForwardAction
      context={context}
      namespace={NAMESPACE}
      kind="Service"
      name="checkout-api"
      remotePort={8080}
      label="Forward port 8080"
    >
      Forward
    </ForwardAction>,
  );
}

describe("ForwardAction", () => {
  it("opens §A.4's dialog on the port beside it", async () => {
    draw(CONTEXT);
    await userEvent.click(screen.getByRole("button", { name: "Forward port 8080" }));

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("New port forward")).toBeTruthy();
    await waitFor(() => expect(core.listNamespaces).toHaveBeenCalledWith(CONTEXT));
  });

  /**
   * **The cluster is pinned at the GESTURE**, as it is at every other door into
   * this dialog (`Forwards`' header action, `ResourceMenu`'s row menu). This one
   * passed its `context` prop straight through, and that prop follows the
   * cluster rail: the detail screen it is drawn in reads `useActiveContext`.
   *
   * Today the misfire is hidden by `useObject`'s render-time gate — it returns
   * no object the instant the context changes, so the Ports table, this control
   * and its dialog are all destroyed, and the reader loses a half-filled dialog
   * rather than getting a forward on the wrong cluster. The rerender below is
   * that same prop change WITHOUT the unmount, which is what any future "keep
   * the last object while refetching" would produce.
   */
  it("keeps the dialog on the cluster it was opened against when the prop moves", async () => {
    const { rerender } = draw(CONTEXT);
    await userEvent.click(screen.getByRole("button", { name: "Forward port 8080" }));
    await screen.findByRole("dialog");
    await waitFor(() => expect(core.listNamespaces).toHaveBeenCalledWith(CONTEXT));

    rerender(
      <ForwardAction
        context={ELSEWHERE}
        namespace={NAMESPACE}
        kind="Service"
        name="checkout-api"
        remotePort={8080}
        label="Forward port 8080"
      >
        Forward
      </ForwardAction>,
    );

    // The listings stay with the pinned cluster: another cluster's namespaces
    // under a port read off this one is the whole of the defect.
    expect(core.listNamespaces).toHaveBeenCalledTimes(1);
    expect(core.listNamespaces).not.toHaveBeenCalledWith(ELSEWHERE);
    // And the divergence is stated, in `lib/clusterMoved`'s own words.
    expect(screen.getByText(`This still runs against ${CONTEXT}, not ${ELSEWHERE}`)).toBeTruthy();
  });

  it("refuses the start until the reader confirms the cluster, then forwards on that one", async () => {
    const { rerender } = draw(CONTEXT);
    await userEvent.click(screen.getByRole("button", { name: "Forward port 8080" }));
    await screen.findByRole("dialog");
    await waitFor(() => expect(core.listNamespaces).toHaveBeenCalledWith(CONTEXT));
    await userEvent.selectOptions(screen.getByLabelText("Namespace"), NAMESPACE);
    await waitFor(() =>
      expect(
        within(screen.getByLabelText("Target")).queryByRole("option", { name: "svc/checkout-api" }),
      ).toBeTruthy(),
    );

    rerender(
      <ForwardAction
        context={ELSEWHERE}
        namespace={NAMESPACE}
        kind="Service"
        name="checkout-api"
        remotePort={8080}
        label="Forward port 8080"
      >
        Forward
      </ForwardAction>,
    );

    await userEvent.click(screen.getByRole("button", { name: "Start forward" }));
    expect(core.startPortForward).not.toHaveBeenCalled();

    await userEvent.click(
      screen.getByRole("checkbox", { name: `Yes, still forward on ${CONTEXT}.` }),
    );
    await userEvent.click(screen.getByRole("button", { name: "Start forward" }));

    await waitFor(() => expect(core.startPortForward).toHaveBeenCalledTimes(1));
    expect(core.startPortForward).toHaveBeenCalledWith(
      expect.objectContaining({ context: CONTEXT, namespace: NAMESPACE, name: "checkout-api" }),
    );
  });

  it("says nothing about a cluster that has not moved", async () => {
    draw(CONTEXT);
    await userEvent.click(screen.getByRole("button", { name: "Forward port 8080" }));
    await screen.findByRole("dialog");
    await waitFor(() => expect(core.listNamespaces).toHaveBeenCalledWith(CONTEXT));

    expect(screen.queryByText(/This still runs against/)).toBeNull();
    expect(screen.queryByRole("checkbox", { name: /Yes, still/ })).toBeNull();
  });

  it("asks the cluster again each time the control is opened", async () => {
    // The acknowledgement belongs to one gesture. A reader who ticked it,
    // cancelled, and opened the control again is answering a new question.
    const { rerender } = draw(CONTEXT);
    await userEvent.click(screen.getByRole("button", { name: "Forward port 8080" }));
    await screen.findByRole("dialog");
    rerender(
      <ForwardAction
        context={ELSEWHERE}
        namespace={NAMESPACE}
        kind="Service"
        name="checkout-api"
        remotePort={8080}
        label="Forward port 8080"
      >
        Forward
      </ForwardAction>,
    );
    await userEvent.click(
      screen.getByRole("checkbox", { name: `Yes, still forward on ${CONTEXT}.` }),
    );
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());

    // Opened again, now on `stage-eu`: nothing carried over, and nothing to
    // carry over to — the pin and the rail agree again.
    await userEvent.click(screen.getByRole("button", { name: "Forward port 8080" }));
    await screen.findByRole("dialog");
    expect(screen.queryByText(/This still runs against/)).toBeNull();
    await waitFor(() => expect(core.listNamespaces).toHaveBeenCalledWith(ELSEWHERE));
  });
});
