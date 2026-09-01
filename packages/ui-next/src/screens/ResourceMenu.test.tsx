import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

// Everything this hook reaches into core for. Mocked so a test can control
// what a write "does" without a real cluster, and can make one fail on
// demand — the dialog's error path is the whole point of half these tests.
type ActionResult = { ok?: boolean; error?: string };

const { deleteResource, scaleResource, rolloutRestart, evictPod, cronjobSetSuspend, cronjobTriggerNow, getObject } =
  vi.hoisted(() => ({
    deleteResource: vi.fn(async (): Promise<ActionResult> => ({ ok: true })),
    scaleResource: vi.fn(async (): Promise<ActionResult> => ({ ok: true })),
    rolloutRestart: vi.fn(async (): Promise<ActionResult> => ({ ok: true })),
    evictPod: vi.fn(async (): Promise<ActionResult> => ({ ok: true })),
    cronjobSetSuspend: vi.fn(async (): Promise<ActionResult> => ({ ok: true })),
    cronjobTriggerNow: vi.fn(async (): Promise<{ jobName?: string; error?: string }> => ({
      jobName: "nightly-manual-1",
    })),
    // `Open shell` looks the pod up fresh, off the live cluster, to find out
    // which containers are worth asking about — a list row carries no
    // container names. Real by default (one running "app" container); tests
    // that care about the shape override it per-call.
    getObject: vi.fn(
      async (): Promise<{ object?: { spec: { containers: { name: string }[] } }; error?: string }> => ({
        object: { spec: { containers: [{ name: "app" }] } },
      }),
    ),
  }));

// `Open shell` starts a session in the module-level store rather than
// minting a route — mocked so a test can see exactly what it was asked to
// start, without a real xterm instance or a real PTY behind it.
const startPodSession = vi.hoisted(() => vi.fn(async () => 1));
vi.mock("../lib/sessions", () => ({ startPodSession }));

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
  getObject,
  ...forwardCore,
}));

import { useRowMenu, type UseRowMenuArgs } from "./ResourceMenu";
import { toKubectl } from "@srelens/core";
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
  Object.defineProperty(navigator, "clipboard", {
    value: { writeText: vi.fn().mockResolvedValue(undefined) },
    configurable: true,
  });
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

  it("confirms a kubectl copy in place and announces it without a toast host", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText },
      configurable: true,
    });
    render(<Harness args={POD_ARGS} row={POD_ROW} />);

    await userEvent.click(screen.getByRole("button", { name: "Copy as kubectl" }));

    expect(writeText).toHaveBeenCalledWith(
      toKubectl({
        action: "get",
        kind: "Pod",
        name: "web-0",
        namespace: "kube-system",
        context: "prod",
        output: "yaml",
      }),
    );
    expect(await screen.findByRole("button", { name: "Copied" })).toBeDefined();
    expect(screen.getByRole("status").textContent).toBe("Copied to clipboard");
  });

  it("shows and announces a refused kubectl copy without claiming success", async () => {
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText: vi.fn().mockRejectedValue(new Error("denied")) },
      configurable: true,
    });
    render(<Harness args={POD_ARGS} row={POD_ROW} />);

    await userEvent.click(screen.getByRole("button", { name: "Copy as kubectl" }));

    expect(await screen.findByRole("button", { name: "Copy failed" })).toBeDefined();
    expect(screen.queryByRole("button", { name: "Copied" })).toBeNull();
    expect(screen.getByRole("status").textContent).toBe("Could not copy to clipboard");
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

  /**
   * `Edit` used to mint `/edit/<name>` — the NAME alone, while `Open in new
   * tab` and `Follow logs` beside it both carry kind, namespace and name.
   * `openTab` dedupes by route string, so `Edit` on `default/api` and on
   * `staging/api` collapsed into ONE tab titled "Edit api", and the second
   * click focused the FIRST resource's editor. A Pod `api` and a Deployment
   * `api` collapsed the same way.
   */
  it("opens Edit at a route carrying the kind, the namespace and the name", async () => {
    render(<Harness args={POD_ARGS} row={POD_ROW} />);
    await userEvent.click(screen.getByRole("button", { name: "Edit" }));
    expect(store.currentWorkspace().tabs.some((t) => t.route === "/edit/Pod/kube-system/web-0")).toBe(true);
  });

  it("gives Edit on two namespaces' same-named resources two tabs, not one", async () => {
    const args: UseRowMenuArgs = { context: "prod", kind: "Deployment", actions: {} };
    const { unmount } = render(<Harness args={args} row={{ name: "api", namespace: "default" }} />);
    await userEvent.click(screen.getByRole("button", { name: "Edit" }));
    unmount();
    render(<Harness args={args} row={{ name: "api", namespace: "staging" }} />);
    await userEvent.click(screen.getByRole("button", { name: "Edit" }));
    const edits = store.currentWorkspace().tabs.filter((t) => t.route.startsWith("/edit/"));
    expect(edits.map((t) => t.route)).toEqual([
      "/edit/Deployment/default/api",
      "/edit/Deployment/staging/api",
    ]);
  });

  it("gives Edit on a cluster-scoped kind the same arity, with the placeholder segment", async () => {
    render(<Harness args={NODE_ARGS} row={NODE_ROW} />);
    await userEvent.click(screen.getByRole("button", { name: "Edit" }));
    expect(store.currentWorkspace().tabs.some((t) => t.route === "/edit/Node/-/worker-1")).toBe(true);
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

  /**
   * `Open shell` used to mint `/resources/<name>/shell` — a route no screen
   * is registered for, the same dead end Follow logs and Port forward shipped
   * with (#346, #349). Sessions live in a module-level store, so the fix is
   * to start one there and open the screen that shows it, not to design the
   * route the other two needed.
   *
   * Both effects are asserted, deliberately: a test that checked only the
   * `/terminals` tab would still pass with the session start deleted, and a
   * test that checked only `startPodSession` would still pass with the
   * navigation deleted. Neither promise implies the other.
   */
  it("starts a session for the pod and opens /terminals on it — a one-container pod is not interrogated", async () => {
    render(<Harness args={POD_ARGS} row={POD_ROW} />);
    expect(screen.queryByRole("dialog")).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Open shell" }));

    await waitFor(() =>
      expect(startPodSession).toHaveBeenCalledWith({
        context: "prod",
        namespace: "kube-system",
        pod: "web-0",
        container: "app",
      }),
    );
    expect(store.currentWorkspace().tabs.some((t) => t.route === "/terminals")).toBe(true);
    // The pod has one candidate container — nothing was asked.
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("asks which container on a pod with more than one, and starts nothing until the pick is confirmed", async () => {
    getObject.mockResolvedValueOnce({
      object: { spec: { containers: [{ name: "app" }, { name: "proxy" }] } },
    });
    render(<Harness args={POD_ARGS} row={POD_ROW} />);
    await userEvent.click(screen.getByRole("button", { name: "Open shell" }));

    const dialog = within(await screen.findByRole("dialog"));
    expect(startPodSession).not.toHaveBeenCalled();
    // No annotation and neither container reported running: the first app
    // container is the default offered.
    await waitFor(() => expect((dialog.getByLabelText("Container") as HTMLSelectElement).value).toBe("app"));

    await userEvent.selectOptions(dialog.getByLabelText("Container"), "proxy");
    await userEvent.click(dialog.getByRole("button", { name: "Open" }));

    await waitFor(() =>
      expect(startPodSession).toHaveBeenCalledWith({
        context: "prod",
        namespace: "kube-system",
        pod: "web-0",
        container: "proxy",
      }),
    );
    expect(store.currentWorkspace().tabs.some((t) => t.route === "/terminals")).toBe(true);
  });

  it("reports the pod lookup's failure through describeError, and starts nothing", async () => {
    getObject.mockResolvedValueOnce({ error: "pods \"web-0\" is forbidden" });
    render(<Harness args={POD_ARGS} row={POD_ROW} />);
    await userEvent.click(screen.getByRole("button", { name: "Open shell" }));

    await waitFor(() => expect(getObject).toHaveBeenCalled());
    expect(startPodSession).not.toHaveBeenCalled();
    expect(store.currentWorkspace().tabs.some((t) => t.route === "/terminals")).toBe(false);
    expect(screen.queryByRole("dialog")).toBeNull();
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

/**
 * **Every write here runs on the cluster the row was picked on.**
 *
 * Until #357 a dialog was window-modal: its overlay covered the cluster rail,
 * so the reader could not switch clusters while one was open, and reading the
 * live `context` prop inside `confirm` was accidentally right. It is not any
 * more — `setActiveCluster` switches the active cluster in place, globally,
 * and no screen carries `key={name}`, so nothing here remounts and `pending`
 * survives the switch with the other cluster's row still in it.
 *
 * Reproduced by execution before this suite existed: Delete opened on
 * `prod-eu`'s `checkout`, the rail moved to `stage-eu`, Delete confirmed, and
 * `deleteResource` was called with `[ 'stage-eu', 'Deployment', 'default',
 * 'checkout' ]`. Two clicks and staging's Deployment is gone, with the dialog
 * still naming production's row.
 *
 * The rule, from `lib/clusterMoved`: the cluster is pinned when the entry is
 * picked, the write runs against the pinned one, the divergence is stated —
 * and, unlike Helm's dialog, the confirmation is re-armed. These confirms take
 * one click (Scale takes one number, and it is kept), so asking again costs
 * the reader a tick.
 */
describe("useRowMenu — the cluster the row was picked on", () => {
  const PINNED = "prod-eu";
  const MOVED = "stage-eu";
  const ROW: ListRow = { name: "checkout", namespace: "default" };

  const argsOn = (context: string): UseRowMenuArgs => ({
    context,
    kind: "Deployment",
    actions: { scale: true, restart: true },
  });

  const box = () => within(screen.getByRole("dialog"));
  const tickFor = (verb: string) => screen.getByRole("checkbox", { name: `Yes, still ${verb} on ${PINNED}.` });
  const REFUSAL = `This runs on ${PINNED}, not ${MOVED}. Confirm the cluster above, or cancel.`;

  /** Pick an entry on `prod-eu`, then move the rail to `stage-eu` under it. */
  async function pickThenMove(label: string, args: (context: string) => UseRowMenuArgs = argsOn) {
    const view = render(<Harness args={args(PINNED)} row={ROW} />);
    await userEvent.click(screen.getByRole("button", { name: label }));
    await screen.findByRole("dialog");
    view.rerender(<Harness args={args(MOVED)} row={ROW} />);
    return view;
  }

  it("deletes on the cluster the row was picked on, not the one the rail moved to", async () => {
    await pickThenMove("Delete");
    await userEvent.click(tickFor("delete"));
    await userEvent.click(box().getByRole("button", { name: "Delete" }));

    await waitFor(() => expect(deleteResource).toHaveBeenCalledTimes(1));
    expect(deleteResource).toHaveBeenCalledWith(PINNED, "Deployment", "default", "checkout");
  });

  it("refuses the write until the reader confirms which cluster, and says why", async () => {
    await pickThenMove("Delete");
    await userEvent.click(box().getByRole("button", { name: "Delete" }));

    // Nothing went out, and the dialog is still up with the reason in it —
    // the same path a validation message this hook wrote itself takes.
    expect(deleteResource).not.toHaveBeenCalled();
    expect(box().getByText(REFUSAL)).toBeTruthy();
    expect(screen.getByRole("dialog")).toBeTruthy();

    // And the tick is all it takes: the question is which cluster, not
    // whether to ask the whole confirm again.
    await userEvent.click(tickFor("delete"));
    await userEvent.click(box().getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(deleteResource).toHaveBeenCalledTimes(1));
    expect(deleteResource).toHaveBeenCalledWith(PINNED, "Deployment", "default", "checkout");
  });

  it("says the rail moved, first, above the row's own name", async () => {
    await pickThenMove("Delete");
    const alert = screen.getByText(`This still runs against ${PINNED}, not ${MOVED}`).closest("[data-tone]");
    expect(alert).toBeTruthy();
    expect(alert?.getAttribute("data-tone")).toBe("warn");
    // Toned `warn`, so the kit gives it `role="status"` — a polite live
    // region, which is what announces a fact that appears while the dialog is
    // already open.
    expect(alert?.getAttribute("role")).toBe("status");
    expect(alert?.textContent?.replace(/\s+/g, " ")).toContain(
      `the same names on ${MOVED} are different objects`,
    );

    // First in the message, above `Delete checkout in default?`: it changes
    // what that name refers to.
    const message = alert?.parentElement;
    expect(message?.firstElementChild).toBe(alert);
    expect(message?.textContent).toContain("This cannot be undone.");
  });

  it("keeps naming the cluster the write will reach in the kubectl preview", async () => {
    await pickThenMove("Delete");
    expect(box().getByText(new RegExp(`^kubectl delete .* --context ${PINNED}$`))).toBeTruthy();
    expect(box().queryByText(new RegExp(`--context ${MOVED}`))).toBeNull();
  });

  it("says nothing, and asks nothing, while the rail has not moved", async () => {
    render(<Harness args={argsOn(PINNED)} row={ROW} />);
    await userEvent.click(screen.getByRole("button", { name: "Delete" }));
    await screen.findByRole("dialog");

    expect(document.querySelector('[role="dialog"] [data-tone]')).toBeNull();
    expect(screen.queryByRole("checkbox")).toBeNull();
    // One click, exactly as before this fix existed.
    await userEvent.click(box().getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(deleteResource).toHaveBeenCalledTimes(1));
    expect(deleteResource).toHaveBeenCalledWith(PINNED, "Deployment", "default", "checkout");
  });

  it("re-arms when the rail moves on again, rather than carrying the tick over", async () => {
    const view = await pickThenMove("Delete");
    await userEvent.click(tickFor("delete"));
    expect((tickFor("delete") as HTMLInputElement).checked).toBe(true);

    // A third cluster. The reader confirmed a divergence that no longer
    // exists, and was never asked about this one.
    view.rerender(<Harness args={argsOn("dev-eu")} row={ROW} />);
    const again = screen.getByRole("checkbox", { name: `Yes, still delete on ${PINNED}.` }) as HTMLInputElement;
    expect(again.checked).toBe(false);
    await userEvent.click(box().getByRole("button", { name: "Delete" }));
    expect(deleteResource).not.toHaveBeenCalled();
    expect(box().getByText(`This runs on ${PINNED}, not dev-eu. Confirm the cluster above, or cancel.`)).toBeTruthy();
  });

  /**
   * Caught by mutation testing: making the gate's `reset` a no-op left every
   * assertion above green. An acknowledgement is about ONE open question, and
   * a dialog that cancelled and a dialog that opened next are two.
   */
  it("forgets an acknowledgement when the dialog it was given for closes", async () => {
    const view = await pickThenMove("Delete");
    await userEvent.click(tickFor("delete"));
    await userEvent.click(box().getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).toBeNull();

    // Back on the cluster the row belongs to, and a fresh Delete on it.
    view.rerender(<Harness args={argsOn(PINNED)} row={ROW} />);
    await userEvent.click(screen.getByRole("button", { name: "Delete" }));
    await screen.findByRole("dialog");
    view.rerender(<Harness args={argsOn(MOVED)} row={ROW} />);

    expect((tickFor("delete") as HTMLInputElement).checked).toBe(false);
    await userEvent.click(box().getByRole("button", { name: "Delete" }));
    expect(deleteResource).not.toHaveBeenCalled();
    expect(box().getByText(REFUSAL)).toBeTruthy();
  });

  it("scales the cluster it was picked on, and keeps the replica count typed for it", async () => {
    const view = render(<Harness args={argsOn(PINNED)} row={ROW} />);
    await userEvent.click(screen.getByRole("button", { name: "Scale" }));
    await screen.findByRole("dialog");
    await userEvent.type(box().getByLabelText("Replica count"), "3");
    view.rerender(<Harness args={argsOn(MOVED)} row={ROW} />);

    // The number survives the switch — the gate re-arms the confirmation, it
    // does not reset the dialog.
    expect((box().getByLabelText("Replica count") as HTMLInputElement).value).toBe("3");
    await userEvent.click(tickFor("scale"));
    await userEvent.click(box().getByRole("button", { name: "Scale" }));
    await waitFor(() => expect(scaleResource).toHaveBeenCalledTimes(1));
    expect(scaleResource).toHaveBeenCalledWith(PINNED, "Deployment", "default", "checkout", 3);
  });

  it("restarts the cluster it was picked on", async () => {
    // The other destructive entries share one `confirm`, but each names its
    // own core call: a fix applied to Delete alone would still retarget these.
    await pickThenMove("Restart rollout");
    await userEvent.click(tickFor("restart"));
    await userEvent.click(box().getByRole("button", { name: "Restart" }));
    await waitFor(() => expect(rolloutRestart).toHaveBeenCalledTimes(1));
    expect(rolloutRestart).toHaveBeenCalledWith(PINNED, "Deployment", "default", "checkout");
  });

  it("evicts on the cluster the pod was picked on", async () => {
    const podArgs = (context: string): UseRowMenuArgs => ({ context, kind: "Pod", actions: { evict: true } });
    await pickThenMove("Evict", podArgs);
    await userEvent.click(tickFor("evict"));
    await userEvent.click(box().getByRole("button", { name: "Evict" }));
    await waitFor(() => expect(evictPod).toHaveBeenCalledTimes(1));
    expect(evictPod).toHaveBeenCalledWith(PINNED, "default", "checkout");
  });

  it("suspends on the cluster the CronJob was picked on, and says which way in the tick", async () => {
    const cronArgs = (context: string): UseRowMenuArgs => ({ context, kind: "CronJob", actions: { suspend: true } });
    await pickThenMove("Suspend", cronArgs);
    // The acknowledgement carries the button's own word, so a Resume can
    // never be confirmed with a sentence that says "suspend".
    await userEvent.click(tickFor("suspend"));
    await userEvent.click(box().getByRole("button", { name: "Suspend" }));
    await waitFor(() => expect(cronjobSetSuspend).toHaveBeenCalledTimes(1));
    expect(cronjobSetSuspend).toHaveBeenCalledWith(PINNED, "default", "checkout", true);
  });

  /**
   * `Open shell` is the one entry with a round trip between the click and the
   * dialog — the exact case Helm's own fix singles out. It writes nothing, so
   * it states the divergence and asks for no tick; what was wrong before is
   * which cluster's pod the terminal attached to.
   */
  it("attaches a shell to the pod on the cluster it was picked on", async () => {
    getObject.mockResolvedValue({ object: { spec: { containers: [{ name: "app" }, { name: "sidecar" }] } } });
    const podArgs = (context: string): UseRowMenuArgs => ({ context, kind: "Pod", actions: { shell: true } });
    await pickThenMove("Open shell", podArgs);

    // Stated, with no acknowledgement to give: nothing is written by opening
    // a terminal, and the terminal itself is labelled with its cluster.
    expect(screen.getByText(`This still runs against ${PINNED}, not ${MOVED}`)).toBeTruthy();
    expect(screen.queryByRole("checkbox")).toBeNull();

    await userEvent.click(box().getByRole("button", { name: "Open" }));
    await waitFor(() => expect(startPodSession).toHaveBeenCalledTimes(1));
    expect(startPodSession).toHaveBeenCalledWith({
      context: PINNED,
      namespace: "default",
      pod: "checkout",
      container: "app",
    });
    // The tab the session opens is labelled with the same cluster: a
    // terminal captioned `stage-eu` over a `prod-eu` pod is the same lie one
    // layer up.
    expect(store.currentWorkspace().tabs.some((t) => t.route === "/terminals" && t.sub === PINNED)).toBe(true);
  });

  /**
   * `Port forward` is the one entry whose dialog is not a confirm — it is
   * §A.4's own form, with its own fields, its own equivalent command and its
   * own submit. So the pin has to travel INTO it: the cluster its namespace
   * list comes from, the cluster the equivalent command names, and the cluster
   * `startPortForward` reaches must all be the one the row was picked on.
   *
   * Following the rail here does not delete anything — it opens a tunnel to a
   * DIFFERENT cluster's workload of the same name, on the reader's own
   * loopback, while the dialog still names the row they picked. A forward is a
   * write with a consequence, so it gets the tick, not just the banner.
   */
  const forwardArgs = (context: string): UseRowMenuArgs => ({
    context,
    kind: "Pod",
    actions: { forward: true },
  });

  /** Open the forward dialog on `prod-eu`, move the rail, then name a port. */
  async function pickForwardThenMove() {
    const view = await pickThenMove("Port forward", forwardArgs);
    await waitFor(() =>
      expect((screen.getByLabelText("Target") as HTMLSelectElement).value).toBe("pod/checkout"),
    );
    await userEvent.type(screen.getByLabelText("Remote port"), "8080");
    return view;
  }

  it("forwards the workload on the cluster the row was picked on, not the one the rail moved to", async () => {
    await pickForwardThenMove();
    await userEvent.click(tickFor("forward"));
    await userEvent.click(box().getByRole("button", { name: "Start forward" }));

    await waitFor(() => expect(forwardCore.startPortForward).toHaveBeenCalledTimes(1));
    expect(forwardCore.startPortForward).toHaveBeenCalledWith({
      context: PINNED,
      namespace: "default",
      kind: "Pod",
      name: "checkout",
      remotePort: 8080,
    });
  });

  it("refuses to start the forward until the reader confirms which cluster, and says why", async () => {
    await pickForwardThenMove();
    await userEvent.click(box().getByRole("button", { name: "Start forward" }));

    expect(forwardCore.startPortForward).not.toHaveBeenCalled();
    expect(box().getByText(REFUSAL)).toBeTruthy();

    // And the tick is all it takes — the question is which cluster, not
    // whether to forward at all.
    await userEvent.click(tickFor("forward"));
    await userEvent.click(box().getByRole("button", { name: "Start forward" }));
    await waitFor(() => expect(forwardCore.startPortForward).toHaveBeenCalledTimes(1));
  });

  it("says the rail moved, and keeps naming the pinned cluster in the equivalent command", async () => {
    await pickForwardThenMove();
    expect(box().getByText(`This still runs against ${PINNED}, not ${MOVED}`)).toBeTruthy();
    // The command under the fields is the one that would be run: same cluster
    // as the banner names, not the one in focus.
    await waitFor(() =>
      expect(box().getByText(new RegExp(`--context ${PINNED}\\b`))).toBeTruthy(),
    );
    expect(box().queryByText(new RegExp(`--context ${MOVED}\\b`))).toBeNull();
  });

  it("lists the pinned cluster's namespaces, and does not re-list when the rail moves", async () => {
    // The dialog's own listings are the other half of the same promise: a
    // namespace select that followed the rail would offer another cluster's
    // namespaces under a prefilled row from this one.
    await pickForwardThenMove();
    expect(forwardCore.listNamespaces).toHaveBeenCalledTimes(1);
    expect(forwardCore.listNamespaces).toHaveBeenCalledWith(PINNED);
    expect(forwardCore.listPods).toHaveBeenCalledWith(PINNED, "default");
  });

  it("says nothing, and asks nothing, while the rail has not moved under the forward", async () => {
    render(<Harness args={forwardArgs(PINNED)} row={ROW} />);
    await userEvent.click(screen.getByRole("button", { name: "Port forward" }));
    await screen.findByRole("dialog");
    expect(screen.queryByText(new RegExp(`This still runs against`))).toBeNull();
    expect(box().queryByRole("checkbox", { name: /Yes, still/ })).toBeNull();
  });
});
