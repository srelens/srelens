import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

// Everything this hook reaches into core for. Mocked so a test can control
// what a write "does" without a real cluster, and can make one fail on
// demand — the dialog's error path is the whole point of half these tests.
type ActionResult = { ok?: boolean; error?: string };

const { deleteResource, scaleResource, rolloutRestart, evictPod, cronjobSetSuspend, cronjobTriggerNow } = vi.hoisted(
  () => ({
    deleteResource: vi.fn(async (): Promise<ActionResult> => ({ ok: true })),
    scaleResource: vi.fn(async (): Promise<ActionResult> => ({ ok: true })),
    rolloutRestart: vi.fn(async (): Promise<ActionResult> => ({ ok: true })),
    evictPod: vi.fn(async (): Promise<ActionResult> => ({ ok: true })),
    cronjobSetSuspend: vi.fn(async (): Promise<ActionResult> => ({ ok: true })),
    cronjobTriggerNow: vi.fn(async (): Promise<{ jobName?: string; error?: string }> => ({
      jobName: "nightly-manual-1",
    })),
  }),
);

// Direct references, not `(...a) => fn(...a)` wrappers: each mock above is
// typed by its own implementation (zero declared params), and TypeScript
// refuses to spread a variable-length `unknown[]` into a call with no rest
// parameter to receive it. A bare reference is structurally assignable to
// the real (many-parameter) core signature — a mock that ignores its
// arguments is still a valid stand-in for a function that takes some.
// What §A.4's forward dialog reaches for, once `Port forward` opens it rather
// than minting a tab. `toKubectl` and `kindToForwardTarget` stay real.
const forwardCore = vi.hoisted(() => ({
  listNamespaces: vi.fn(async () => ({ namespaces: ["kube-system", "default"] })),
  listServices: vi.fn(async () => ({ services: [] })),
  listPods: vi.fn(async () => ({ pods: [{ name: "web-0", namespace: "kube-system" }] })),
  startPortForward: vi.fn(async () => ({ id: 1, localPort: 9090 })),
}));

vi.mock("@srelens/core", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@srelens/core")>()),
  deleteResource,
  scaleResource,
  rolloutRestart,
  evictPod,
  cronjobSetSuspend,
  cronjobTriggerNow,
  ...forwardCore,
}));

import { useRowMenu, type UseRowMenuArgs } from "./ResourceMenu";
import type { ContextMenuItem } from "@srelens/ui-kit";
import type { ListRow } from "../lib/kinds/types";
import * as store from "../lib/tabsStore";
import { defaultState } from "../lib/tabs";

/**
 * Renders the menu's items as plain buttons (a right-click menu is Table's
 * concern, tested in `Table.test.tsx`; this hook is tested on its own
 * contract) plus the dialog it opens. Separators are skipped — nothing to
 * click, nothing to assert on.
 */
function Harness({ args, row }: { args: UseRowMenuArgs; row: ListRow }) {
  const { items, dialog } = useRowMenu(args);
  return (
    <div>
      {items(row).map((item, i) =>
        item.kind === "sep" ? null : (
          <button key={i} onClick={item.onPick}>
            {item.label}
          </button>
        ),
      )}
      {dialog}
    </div>
  );
}

function menuItems(args: UseRowMenuArgs, row: ListRow): ContextMenuItem[] {
  let captured: ContextMenuItem[] = [];
  function Capture() {
    captured = useRowMenu(args).items(row);
    return null;
  }
  render(<Capture />);
  return captured;
}

const POD_ROW: ListRow = { name: "web-0", namespace: "kube-system" };
const CM_ROW: ListRow = { name: "app-config", namespace: "default" };
const DEPLOY_ROW: ListRow = { name: "web", namespace: "default" };
const CRON_ROW: ListRow & { suspended: boolean } = { name: "nightly", namespace: "ops", suspended: false };
const NODE_ROW: ListRow = { name: "worker-1" };

const POD_ARGS: UseRowMenuArgs = {
  context: "prod",
  kind: "Pod",
  actions: { logs: true, shell: true, forward: true, evict: true },
};
const CM_ARGS: UseRowMenuArgs = { context: "prod", kind: "ConfigMap", actions: {} };
const DEPLOY_ARGS: UseRowMenuArgs = { context: "prod", kind: "Deployment", actions: { scale: true, restart: true } };
const CRON_ARGS: UseRowMenuArgs = { context: "prod", kind: "CronJob", actions: { suspend: true, trigger: true } };
const NODE_ARGS: UseRowMenuArgs = { context: "prod", kind: "Node", actions: {} };

beforeEach(() => {
  vi.clearAllMocks();
  store.setState(defaultState([]));
});

describe("useRowMenu", () => {
  it("offers logs, a shell and a forward on a pod, and none of them on a ConfigMap", () => {
    const podLabels = menuItems(POD_ARGS, POD_ROW).map((i) => (i.kind === "sep" ? "—" : i.label));
    expect(podLabels).toEqual(
      expect.arrayContaining(["Open in new tab", "Follow logs", "Open shell", "Port forward", "Edit", "Copy as kubectl", "Evict", "Delete"]),
    );

    const cmLabels = menuItems(CM_ARGS, CM_ROW).map((i) => (i.kind === "sep" ? "—" : i.label));
    expect(cmLabels).not.toContain("Follow logs");
    expect(cmLabels).not.toContain("Open shell");
    expect(cmLabels).not.toContain("Port forward");
    expect(cmLabels).not.toContain("Evict");
    // Edit and Open in new tab are not gated by KindActions at all — every
    // kind gets them. Delete defaults to offered the same way (every one of
    // the 34 built-in kinds passes `actions: {}` and still gets it) — but,
    // unlike every other entry in `KindActions`, it can be turned off, for
    // the one family that needs to: see the `actions.delete === false` test
    // below.
    expect(cmLabels).toEqual(expect.arrayContaining(["Open in new tab", "Edit", "Copy as kubectl", "Delete"]));
  });

  // Whole-branch review (FIX 3): a custom resource's `k8sKind` is the CRD's
  // own kind, and the backend resolves kind→GVR through a closed match with
  // no CRD path — Delete on a custom resource's row always fails, with a
  // confirm dialog (and a kubectl preview that lies, by falling back to
  // lowercasing) that makes it look like a real operation right up until it
  // isn't. `actions.delete === false` is the one way to turn Delete off;
  // every other `KindActions` field turns something on.
  it("withholds Delete when the kind's actions say so, without touching anything else", () => {
    const labels = menuItems({ context: "prod", kind: "Widget", actions: { delete: false } }, CM_ROW).map((i) =>
      i.kind === "sep" ? "—" : i.label,
    );
    expect(labels).not.toContain("Delete");
    expect(labels).toEqual(expect.arrayContaining(["Open in new tab", "Edit", "Copy as kubectl"]));
  });

  it("opens the resource in a tab, at its /k/<kind>/<namespace>/<name> route", async () => {
    render(<Harness args={POD_ARGS} row={POD_ROW} />);
    await userEvent.click(screen.getByRole("button", { name: "Open in new tab" }));
    expect(store.currentWorkspace().tabs.some((t) => t.route === "/k/Pod/kube-system/web-0")).toBe(true);
  });

  it("opens Follow logs at the Logs screen's route, not the placeholder one", async () => {
    // The row menu is the screen's front door — the bare `/logs` empty state
    // tells the reader to come through it. It used to mint
    // `/resources/<name>/logs`, which carries a name and no kind or namespace,
    // so it could not name a subject and rendered a Placeholder: an empty pane
    // where the logs should be.
    render(<Harness args={POD_ARGS} row={POD_ROW} />);
    await userEvent.click(screen.getByRole("button", { name: "Follow logs" }));
    expect(store.currentWorkspace().tabs.some((t) => t.route === "/logs/Pod/kube-system/web-0")).toBe(true);
  });

  /**
   * `Port forward` used to mint `/resources/<name>/forward` — a route no
   * screen is registered for, so the menu's own entry opened a Placeholder.
   * That is the mistake the Logs entry above shipped with and had fixed later
   * (#346); this is the same front door, on the same menu.
   *
   * The dialog goes through the hook's `dialog` slot, which every write action
   * here already uses — so the peek's and the tab's action bars, which render
   * that slot too, get the door for free rather than growing a second one.
   */
  it("opens §A.4's forward dialog on Port forward, rather than a placeholder tab", async () => {
    render(<Harness args={POD_ARGS} row={POD_ROW} />);
    // Nothing is mounted until the entry is picked: a dialog that were always
    // there would pass every assertion below.
    expect(screen.queryByRole("dialog")).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Port forward" }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("New port forward")).toBeDefined();
    expect(store.currentWorkspace().tabs.some((t) => t.route.endsWith("/forward"))).toBe(false);
  });

  it("hands the dialog the row it was picked on — the kind, the name, the namespace", async () => {
    render(<Harness args={POD_ARGS} row={POD_ROW} />);
    await userEvent.click(screen.getByRole("button", { name: "Port forward" }));
    await screen.findByRole("dialog");
    // `pod/`, from the list's KIND through `kindToForwardTarget`.
    await waitFor(() =>
      expect((screen.getByLabelText("Target") as HTMLSelectElement).value).toBe("pod/web-0"),
    );
    await waitFor(() =>
      expect((screen.getByLabelText("Namespace") as HTMLSelectElement).value).toBe("kube-system"),
    );
    // A list row knows no ports, so BOTH are the reader's to name — and the
    // dialog is where they name them.
    expect((screen.getByLabelText("Remote port") as HTMLInputElement).value).toBe("");
    expect((screen.getByLabelText("Local port") as HTMLInputElement).value).toBe("");
  });

  it("starts nothing on the pick, and closes on Cancel", async () => {
    render(<Harness args={POD_ARGS} row={POD_ROW} />);
    await userEvent.click(screen.getByRole("button", { name: "Port forward" }));
    await screen.findByRole("dialog");
    expect(forwardCore.startPortForward).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("still opens a shell in a tab — only the forward entry changed", async () => {
    render(<Harness args={POD_ARGS} row={POD_ROW} />);
    await userEvent.click(screen.getByRole("button", { name: "Open shell" }));
    expect(
      store.currentWorkspace().tabs.some((t) => t.route === "/resources/web-0/shell"),
    ).toBe(true);
  });

  it("opens a cluster-scoped resource with the placeholder namespace segment", async () => {
    render(<Harness args={NODE_ARGS} row={NODE_ROW} />);
    await userEvent.click(screen.getByRole("button", { name: "Open in new tab" }));
    expect(store.currentWorkspace().tabs.some((t) => t.route === "/k/Node/-/worker-1")).toBe(true);
  });

  it("asks before deleting, and calls nothing until the confirm is taken", async () => {
    render(<Harness args={POD_ARGS} row={POD_ROW} />);
    await userEvent.click(screen.getByRole("button", { name: "Delete" }));
    expect(await screen.findByRole("dialog")).toBeDefined();
    expect(deleteResource).not.toHaveBeenCalled();

    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(deleteResource).not.toHaveBeenCalled();
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("deletes the row that was right-clicked, namespace and all", async () => {
    render(<Harness args={POD_ARGS} row={POD_ROW} />);
    await userEvent.click(screen.getByRole("button", { name: "Delete" }));
    const dialog = within(screen.getByRole("dialog"));
    await userEvent.click(dialog.getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(deleteResource).toHaveBeenCalledWith("prod", "Pod", "kube-system", "web-0"));
  });

  it("shows the kubectl the action is equivalent to, so the reader can check it first", async () => {
    render(<Harness args={POD_ARGS} row={POD_ROW} />);
    await userEvent.click(screen.getByRole("button", { name: "Delete" }));
    expect(screen.getByText("kubectl delete pods web-0 -n kube-system --context prod")).toBeDefined();
  });

  it("keeps the dialog open with the message when the action fails", async () => {
    deleteResource.mockResolvedValueOnce({ error: "forbidden" });
    render(<Harness args={POD_ARGS} row={POD_ROW} />);
    await userEvent.click(screen.getByRole("button", { name: "Delete" }));
    const dialog = within(screen.getByRole("dialog"));
    await userEvent.click(dialog.getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(screen.getByText(/forbidden/)).toBeDefined());
    // Still up: the dialog did not close as though the delete had happened.
    expect(screen.getByRole("dialog")).toBeDefined();
  });

  it("takes a replica count before scaling, and refuses a negative one", async () => {
    render(<Harness args={DEPLOY_ARGS} row={DEPLOY_ROW} />);
    await userEvent.click(screen.getByRole("button", { name: "Scale" }));
    const dialog = within(screen.getByRole("dialog"));
    const input = dialog.getByRole("textbox", { name: "Replica count" });

    await userEvent.type(input, "-1");
    await userEvent.click(dialog.getByRole("button", { name: "Scale" }));
    expect(scaleResource).not.toHaveBeenCalled();
    expect(screen.getByText(/non-negative/i)).toBeDefined();

    await userEvent.clear(input);
    await userEvent.type(input, "3");
    await userEvent.click(dialog.getByRole("button", { name: "Scale" }));
    await waitFor(() => expect(scaleResource).toHaveBeenCalledWith("prod", "Deployment", "default", "web", 3));
  });

  it("marks every destructive entry as danger, not just delete", () => {
    // #335 fixed the tab menu's version of this bug; this is the row menu's.
    const podItems = menuItems(POD_ARGS, POD_ROW);
    const deployItems = menuItems(DEPLOY_ARGS, DEPLOY_ROW);
    const destructiveLabels = ["Scale", "Restart rollout", "Evict", "Delete"];
    for (const item of [...podItems, ...deployItems]) {
      if (item.kind !== "sep" && destructiveLabels.includes(item.label)) {
        expect(item.danger).toBe(true);
      }
    }
    // Sanity: every destructive label above was actually exercised at least
    // once, so the assertion isn't vacuously true.
    const seen = [...podItems, ...deployItems]
      .filter((i): i is Extract<ContextMenuItem, { label: string }> => i.kind !== "sep")
      .map((i) => i.label);
    for (const label of destructiveLabels) expect(seen).toContain(label);
  });

  it("offers Suspend/Resume and Run now for a CronJob only, labelled from the row's state", () => {
    const active = menuItems(CRON_ARGS, CRON_ROW);
    expect(active.some((i) => i.kind !== "sep" && i.label === "Suspend")).toBe(true);
    expect(active.some((i) => i.kind !== "sep" && i.label === "Run now")).toBe(true);

    const suspendedRow: ListRow & { suspended: boolean } = { ...CRON_ROW, suspended: true };
    const suspended = menuItems(CRON_ARGS, suspendedRow);
    expect(suspended.some((i) => i.kind !== "sep" && i.label === "Resume")).toBe(true);
    expect(suspended.some((i) => i.kind !== "sep" && i.label === "Suspend")).toBe(false);

    // No other kind offers either.
    const podLabels = menuItems(POD_ARGS, POD_ROW).map((i) => (i.kind === "sep" ? "—" : i.label));
    const deployLabels = menuItems(DEPLOY_ARGS, DEPLOY_ROW).map((i) => (i.kind === "sep" ? "—" : i.label));
    expect(podLabels).not.toContain("Suspend");
    expect(podLabels).not.toContain("Run now");
    expect(deployLabels).not.toContain("Suspend");
    expect(deployLabels).not.toContain("Run now");
  });

  it("suspends the row that was picked", async () => {
    render(<Harness args={CRON_ARGS} row={CRON_ROW} />);
    await userEvent.click(screen.getByRole("button", { name: "Suspend" }));
    const dialog = within(screen.getByRole("dialog"));
    await userEvent.click(dialog.getByRole("button", { name: "Suspend" }));
    await waitFor(() => expect(cronjobSetSuspend).toHaveBeenCalledWith("prod", "ops", "nightly", true));
  });

  it("resumes the row when it is already suspended, inverting the direction rather than repeating it", async () => {
    // The other half of the pair above: a regression that broke the
    // inversion specifically on the resume side (e.g. always passing `true`)
    // would still pass a test that only ever exercised `suspended: false`.
    const suspendedRow: ListRow & { suspended: boolean } = { ...CRON_ROW, suspended: true };
    render(<Harness args={CRON_ARGS} row={suspendedRow} />);
    await userEvent.click(screen.getByRole("button", { name: "Resume" }));
    const dialog = within(screen.getByRole("dialog"));
    await userEvent.click(dialog.getByRole("button", { name: "Resume" }));
    await waitFor(() => expect(cronjobSetSuspend).toHaveBeenCalledWith("prod", "ops", "nightly", false));
  });

  it("runs a CronJob now with no confirm at all", async () => {
    render(<Harness args={CRON_ARGS} row={CRON_ROW} />);
    await userEvent.click(screen.getByRole("button", { name: "Run now" }));
    await waitFor(() => expect(cronjobTriggerNow).toHaveBeenCalledWith("prod", "ops", "nightly"));
    // No dialog appeared for it — Cancel is not on screen.
    expect(screen.queryByRole("button", { name: "Cancel" })).toBeNull();
  });
});
