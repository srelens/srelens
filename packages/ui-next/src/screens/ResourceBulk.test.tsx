import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

// Everything this screen reaches into core for. Mocked so a test can control
// what a write "does" without a real cluster, and can make one fail on demand
// — the partial-failure report is the whole point of one of these tests.
type ActionResult = { ok?: boolean; error?: string };

const { deleteResource, evictPod, rolloutRestart } = vi.hoisted(() => ({
  deleteResource: vi.fn(async (): Promise<ActionResult> => ({ ok: true })),
  evictPod: vi.fn(async (): Promise<ActionResult> => ({ ok: true })),
  rolloutRestart: vi.fn(async (): Promise<ActionResult> => ({ ok: true })),
}));

vi.mock("@srelens/core", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@srelens/core")>()),
  deleteResource,
  evictPod,
  rolloutRestart,
}));

import { ResourceBulk } from "./ResourceBulk";
import type { KindDescriptor, ListRow } from "../lib/kinds/types";

const PODS: ListRow[] = [
  { name: "web-0", namespace: "kube-system" },
  { name: "web-0", namespace: "prod" },
  { name: "api-1", namespace: "prod" },
];

const POD_DESCRIPTOR: KindDescriptor = {
  k8sKind: "Pod",
  columns: [],
  source: "watch",
  scope: "namespaced",
  actions: { evict: true },
};

const DEPLOY_DESCRIPTOR: KindDescriptor = {
  k8sKind: "Deployment",
  columns: [],
  source: "watch",
  scope: "namespaced",
  actions: { restart: true },
};

function keyOf(row: ListRow): string {
  return `${row.namespace ?? ""}/${row.name}`;
}

const ALL_SELECTED = new Set(PODS.map(keyOf));

beforeEach(() => {
  vi.clearAllMocks();
  // A prior test's `mockImplementation` (the partial-failure case) would
  // otherwise leak into the next one — `clearAllMocks` resets call history,
  // not a swapped-in implementation.
  deleteResource.mockResolvedValue({ ok: true });
  evictPod.mockResolvedValue({ ok: true });
  rolloutRestart.mockResolvedValue({ ok: true });
});

describe("ResourceBulk", () => {
  it("stays out of the way until something is selected", () => {
    const { container } = render(
      <ResourceBulk
        selected={new Set()}
        kind="pods"
        descriptor={POD_DESCRIPTOR}
        context="prod"
        rows={PODS}
        onDone={() => {}}
      />,
    );
    expect(container.textContent).toBe("");
  });

  // Whole-branch review (FIX 2): `Table` never prunes `selection.selected`
  // when its data changes — select rows, then filter the list down to none
  // of them, and the stale keys are still in `selected`. The bar must count
  // (and act on) only the selection that resolves against the rows it was
  // actually handed, not the raw key count, or Delete opens a confirm for
  // rows that no longer exist and a mismatched count.
  it("counts and acts on only the selected keys still present in rows, not stale ones", () => {
    const selected = new Set([...PODS.map(keyOf), "ghost-ns/ghost-pod"]);
    render(
      <ResourceBulk
        selected={selected}
        kind="pods"
        descriptor={POD_DESCRIPTOR}
        context="prod"
        rows={PODS}
        onDone={() => {}}
      />,
    );
    // Not "4 selected" — the ghost key was never in `rows`.
    expect(screen.getByText("3 selected")).toBeDefined();
  });

  it("stays out of the way when every selected key has fallen out of the rows", () => {
    const { container } = render(
      <ResourceBulk
        selected={new Set(["ghost-ns/ghost-pod"])}
        kind="pods"
        descriptor={POD_DESCRIPTOR}
        context="prod"
        rows={PODS}
        onDone={() => {}}
      />,
    );
    expect(container.textContent).toBe("");
  });

  it("counts what is selected, in words", () => {
    render(
      <ResourceBulk
        selected={ALL_SELECTED}
        kind="pods"
        descriptor={POD_DESCRIPTOR}
        context="prod"
        rows={PODS}
        onDone={() => {}}
      />,
    );
    expect(screen.getByText("3 selected")).toBeDefined();
  });

  it("asks once for the whole selection, naming how many", async () => {
    render(
      <ResourceBulk
        selected={ALL_SELECTED}
        kind="pods"
        descriptor={POD_DESCRIPTOR}
        context="prod"
        rows={PODS}
        onDone={() => {}}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Delete" }));

    // Exactly one dialog, not one per row.
    expect(screen.getAllByRole("dialog")).toHaveLength(1);
    expect(screen.getByText("Delete 3 pods?")).toBeDefined();
    expect(deleteResource).not.toHaveBeenCalled();
  });

  it("acts on every selected row, by namespace and name", async () => {
    render(
      <ResourceBulk
        selected={ALL_SELECTED}
        kind="pods"
        descriptor={POD_DESCRIPTOR}
        context="prod"
        rows={PODS}
        onDone={() => {}}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Delete" }));
    const dialog = within(screen.getByRole("dialog"));
    await userEvent.click(dialog.getByRole("button", { name: "Delete" }));

    await waitFor(() => expect(deleteResource).toHaveBeenCalledTimes(3));
    expect(deleteResource).toHaveBeenCalledWith("prod", "Pod", "kube-system", "web-0");
    // The other same-named pod, in a different namespace, is a distinct call.
    expect(deleteResource).toHaveBeenCalledWith("prod", "Pod", "prod", "web-0");
    expect(deleteResource).toHaveBeenCalledWith("prod", "Pod", "prod", "api-1");
  });

  it("says which succeeded and which did not, rather than 'some failed'", async () => {
    // `runBulk`'s workers pull items off the front of the list synchronously
    // before their first await, so with concurrency >= item count the calls
    // land in `PODS` order — the first is `kube-system/web-0`.
    deleteResource.mockResolvedValueOnce({ error: "forbidden" }).mockResolvedValue({ ok: true });
    render(
      <ResourceBulk
        selected={ALL_SELECTED}
        kind="pods"
        descriptor={POD_DESCRIPTOR}
        context="prod"
        rows={PODS}
        onDone={() => {}}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Delete" }));
    const dialog = within(screen.getByRole("dialog"));
    await userEvent.click(dialog.getByRole("button", { name: "Delete" }));

    await waitFor(() => expect(deleteResource).toHaveBeenCalledTimes(3));
    // Names the row that failed and why, and the ones that did not.
    expect(await screen.findByText(/kube-system\/web-0/)).toBeDefined();
    expect(screen.getByText(/forbidden/)).toBeDefined();
    expect(screen.getByText(/prod\/web-0/)).toBeDefined();
    expect(screen.getByText(/prod\/api-1/)).toBeDefined();
    expect(screen.queryByText(/some failed/i)).toBeNull();
  });

  // Whole-branch review (FIX 7): the report dialog set both confirmLabel and
  // cancelLabel to "Close" — two controls a screen reader cannot tell apart,
  // since an accessible name is what distinguishes them, not which side of
  // the dialog they render on.
  it("gives the report dialog's two buttons distinct accessible names", async () => {
    deleteResource.mockResolvedValueOnce({ error: "forbidden" }).mockResolvedValue({ ok: true });
    render(
      <ResourceBulk
        selected={ALL_SELECTED}
        kind="pods"
        descriptor={POD_DESCRIPTOR}
        context="prod"
        rows={PODS}
        onDone={() => {}}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Delete" }));
    const confirmDialog = within(screen.getByRole("dialog"));
    await userEvent.click(confirmDialog.getByRole("button", { name: "Delete" }));

    const report = within(await screen.findByRole("dialog"));
    const names = report.getAllByRole("button").map((button) => button.textContent);
    expect(new Set(names).size).toBe(names.length);
  });

  // Whole-branch review (FIX 3): same reason as the row menu's own gate — a
  // custom resource's Delete always fails against the backend's kind→GVR
  // resolution, so the bulk bar must not offer it either.
  it("withholds Delete when the kind's actions say so", () => {
    const noDeleteDescriptor: KindDescriptor = {
      k8sKind: "Widget",
      columns: [],
      source: "poll",
      scope: "namespaced",
      actions: { delete: false },
    };
    render(
      <ResourceBulk
        selected={ALL_SELECTED}
        kind="widgets"
        descriptor={noDeleteDescriptor}
        context="prod"
        rows={PODS}
        onDone={() => {}}
      />,
    );
    expect(screen.queryByRole("button", { name: "Delete" })).toBeNull();
  });

  it("offers evict only where the kind has it", () => {
    const pods = render(
      <ResourceBulk
        selected={ALL_SELECTED}
        kind="pods"
        descriptor={POD_DESCRIPTOR}
        context="prod"
        rows={PODS}
        onDone={() => {}}
      />,
    );
    expect(screen.getByRole("button", { name: "Evict" })).toBeDefined();
    pods.unmount();

    render(
      <ResourceBulk
        selected={ALL_SELECTED}
        kind="deployments"
        descriptor={DEPLOY_DESCRIPTOR}
        context="prod"
        rows={PODS}
        onDone={() => {}}
      />,
    );
    expect(screen.queryByRole("button", { name: "Evict" })).toBeNull();
    // Deployments have restart instead.
    expect(screen.getByRole("button", { name: "Restart rollout" })).toBeDefined();
  });

  it("clears the selection once the action is done", async () => {
    const onDone = vi.fn();
    render(
      <ResourceBulk
        selected={ALL_SELECTED}
        kind="pods"
        descriptor={POD_DESCRIPTOR}
        context="prod"
        rows={PODS}
        onDone={onDone}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Delete" }));
    const dialog = within(screen.getByRole("dialog"));
    await userEvent.click(dialog.getByRole("button", { name: "Delete" }));

    await waitFor(() => expect(onDone).toHaveBeenCalled());
    // A clean run closes the dialog rather than leaving a report open.
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("evicts by namespace and name, not delete, when Evict is picked", async () => {
    render(
      <ResourceBulk
        selected={ALL_SELECTED}
        kind="pods"
        descriptor={POD_DESCRIPTOR}
        context="prod"
        rows={PODS}
        onDone={() => {}}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Evict" }));
    const dialog = within(screen.getByRole("dialog"));
    await userEvent.click(dialog.getByRole("button", { name: "Evict" }));

    await waitFor(() => expect(evictPod).toHaveBeenCalledTimes(3));
    expect(evictPod).toHaveBeenCalledWith("prod", "kube-system", "web-0");
    expect(deleteResource).not.toHaveBeenCalled();
  });

  it("restarts the rollout of every selected row when the kind offers it", async () => {
    render(
      <ResourceBulk
        selected={ALL_SELECTED}
        kind="deployments"
        descriptor={DEPLOY_DESCRIPTOR}
        context="prod"
        rows={PODS}
        onDone={() => {}}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Restart rollout" }));
    const dialog = within(screen.getByRole("dialog"));
    await userEvent.click(dialog.getByRole("button", { name: "Restart" }));

    await waitFor(() => expect(rolloutRestart).toHaveBeenCalledTimes(3));
    expect(rolloutRestart).toHaveBeenCalledWith("prod", "Deployment", "prod", "api-1");
  });
});

/**
 * **A batch runs on the cluster it was opened on.**
 *
 * `Pending` was always a snapshot of the ROWS, so a selection change under an
 * open dialog could not retarget it. It was never a snapshot of the CLUSTER,
 * and since #357 the cluster rail is live behind a dialog: `setActiveCluster`
 * switches the active cluster in place, nothing here remounts, and
 * `Resources.tsx` keeps this bar mounted across the switch whenever the
 * cluster being moved TO already has that kind in the row cache — which is the
 * ordinary case for a reader moving between two clusters they have both looked
 * at.
 *
 * Established by execution against the real `Resources` screen before this
 * suite existed: the dialog stayed open across `setActiveCluster`, and the
 * confirmed delete went out as `[ 'stage-eu', 'Pod', 'default', 'web-1' ]` —
 * production's row name, staging's cluster. The `showRows` flip that was
 * assumed to save this saves nothing once the target view is cached, and it
 * was never designed to save it.
 *
 * The rule is `lib/clusterMoved`'s: pin at open, run against the pinned
 * cluster, state the divergence, and re-arm the confirmation. This dialog has
 * no typed input at all, so the tick costs nothing — and it is the one confirm
 * here where a silent retarget takes out forty objects rather than one.
 */
describe("ResourceBulk — the cluster the batch was opened on", () => {
  const PINNED = "prod-eu";
  const MOVED = "stage-eu";
  const ROWS: ListRow[] = [
    { name: "web-0", namespace: "kube-system" },
    { name: "api-1", namespace: "prod" },
  ];
  const SELECTED = new Set(ROWS.map(keyOf));

  const bar = (context: string) => (
    <ResourceBulk
      selected={SELECTED}
      kind="pods"
      descriptor={POD_DESCRIPTOR}
      context={context}
      rows={ROWS}
      onDone={() => {}}
    />
  );

  const box = () => within(screen.getByRole("dialog"));
  const tick = (verb: string) =>
    screen.getByRole("checkbox", { name: `Yes, still ${verb} on ${PINNED}.` }) as HTMLInputElement;
  const REFUSAL = `This runs on ${PINNED}, not ${MOVED}. Confirm the cluster above, or cancel.`;

  /** The cluster every call in a batch went to. The mocks above declare no
   *  parameters, so the argument list is read structurally. */
  const targetsOf = (calls: unknown[][]) => calls.map((call) => call[0]);

  /** Open a batch on `prod-eu`, then move the rail to `stage-eu` under it. */
  async function openThenMove(label: string) {
    const view = render(bar(PINNED));
    await userEvent.click(screen.getByRole("button", { name: label }));
    await screen.findByRole("dialog");
    view.rerender(bar(MOVED));
    return view;
  }

  it("deletes on the cluster the batch was opened on, not the one the rail moved to", async () => {
    await openThenMove("Delete");
    await userEvent.click(tick("delete"));
    await userEvent.click(box().getByRole("button", { name: "Delete" }));

    await waitFor(() => expect(deleteResource).toHaveBeenCalledTimes(2));
    for (const row of ROWS) {
      expect(deleteResource).toHaveBeenCalledWith(PINNED, "Pod", row.namespace, row.name);
    }
    expect(targetsOf(deleteResource.mock.calls)).toEqual([PINNED, PINNED]);
  });

  it("refuses the run until the reader confirms which cluster, and says why", async () => {
    await openThenMove("Delete");
    await userEvent.click(box().getByRole("button", { name: "Delete" }));

    // Not one row went out, and the dialog is still up with the reason in it.
    expect(deleteResource).not.toHaveBeenCalled();
    expect(box().getByText(REFUSAL)).toBeTruthy();
    expect(screen.getByRole("dialog")).toBeTruthy();

    await userEvent.click(tick("delete"));
    await userEvent.click(box().getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(deleteResource).toHaveBeenCalledTimes(2));
  });

  it("evicts on the cluster the batch was opened on", async () => {
    // A second core call behind the same confirm: a fix applied to Delete
    // alone would still retarget this one.
    await openThenMove("Evict");
    await userEvent.click(tick("evict"));
    await userEvent.click(box().getByRole("button", { name: "Evict" }));
    await waitFor(() => expect(evictPod).toHaveBeenCalledTimes(2));
    expect(targetsOf(evictPod.mock.calls)).toEqual([PINNED, PINNED]);
  });

  it("says the rail moved, first, above the list of rows", async () => {
    await openThenMove("Delete");
    const alert = screen.getByText(`This still runs against ${PINNED}, not ${MOVED}`).closest("[data-tone]");
    expect(alert).toBeTruthy();
    expect(alert?.getAttribute("data-tone")).toBe("warn");
    expect(alert?.getAttribute("role")).toBe("status");
    expect(alert?.textContent?.replace(/\s+/g, " ")).toContain(
      `the same names on ${MOVED} are different objects`,
    );

    // Above `This will delete 2 pods`: the alert changes what every name in
    // the list under it refers to.
    const message = alert?.parentElement;
    expect(message?.firstElementChild).toBe(alert);
    expect(message?.textContent).toContain("This will delete 2 pods");
  });

  it("says nothing, and asks nothing, while the rail has not moved", async () => {
    render(bar(PINNED));
    await userEvent.click(screen.getByRole("button", { name: "Delete" }));
    await screen.findByRole("dialog");

    expect(document.querySelector('[role="dialog"] [data-tone]')).toBeNull();
    expect(screen.queryByRole("checkbox")).toBeNull();
    await userEvent.click(box().getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(deleteResource).toHaveBeenCalledTimes(2));
    expect(targetsOf(deleteResource.mock.calls)).toEqual([PINNED, PINNED]);
  });

  /**
   * Caught by mutation testing: making the gate's `reset` a no-op left every
   * assertion above green. An acknowledgement is about ONE open question.
   */
  it("forgets an acknowledgement when the dialog it was given for closes", async () => {
    const view = await openThenMove("Delete");
    await userEvent.click(tick("delete"));
    await userEvent.click(box().getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).toBeNull();

    view.rerender(bar(PINNED));
    await userEvent.click(screen.getByRole("button", { name: "Delete" }));
    await screen.findByRole("dialog");
    view.rerender(bar(MOVED));

    expect(tick("delete").checked).toBe(false);
    await userEvent.click(box().getByRole("button", { name: "Delete" }));
    expect(deleteResource).not.toHaveBeenCalled();
    expect(box().getByText(REFUSAL)).toBeTruthy();
  });

  it("re-arms when the rail moves on again, rather than carrying the tick over", async () => {
    const view = await openThenMove("Delete");
    await userEvent.click(tick("delete"));
    expect(tick("delete").checked).toBe(true);

    view.rerender(bar("dev-eu"));
    expect(tick("delete").checked).toBe(false);
    await userEvent.click(box().getByRole("button", { name: "Delete" }));
    expect(deleteResource).not.toHaveBeenCalled();
  });
});
