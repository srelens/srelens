import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { Status } from "./Status";
import { ConsoleProvider, useConsole } from "../console";
import { activeRoute, setState } from "../lib/tabsStore";
import { defaultState } from "../lib/tabs";
import { probeCluster, resetProbes } from "../lib/probe";
import { resetView } from "../lib/workspace";
import { lockWorkspace, resetLock } from "./LockGate";

// The forwards store is core's, module-level and driven by the backend, so the
// count is faked at the boundary rather than by starting a real forward. The
// getter hands back the same array until it is swapped, which is what
// `useSyncExternalStore` requires of it.
const forwards = vi.hoisted(() => ({ list: [] as unknown[], notify: new Set<() => void>() }));
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  getForwards: () => forwards.list,
  subscribeForwards: (l: () => void) => {
    forwards.notify.add(l);
    return () => forwards.notify.delete(l);
  },
}));

// The session store is ui-next's own (it holds the xterm instance core may
// not depend on), but it is read the same way the forwards store is: a
// module-level snapshot faked at the boundary rather than a real session
// stood up through xterm.
const sessions = vi.hoisted(() => ({ list: [] as unknown[], notify: new Set<() => void>() }));
vi.mock("../lib/sessions", () => ({
  getSessions: () => sessions.list,
  subscribeSessions: (l: () => void) => {
    sessions.notify.add(l);
    return () => sessions.notify.delete(l);
  },
}));

// Helm operations are ui-next's own store too, and read the same way: a
// module-level snapshot faked at the boundary rather than a real `helm
// upgrade` stood up through the backend. The getter hands back the same array
// until it is swapped, which is what `useSyncExternalStore` requires — and
// what a component that filtered inside its snapshot would break.
const helmOps = vi.hoisted(() => ({ list: [] as unknown[], notify: new Set<() => void>() }));
vi.mock("../lib/helmOps", () => ({
  getHelmOps: () => helmOps.list,
  subscribeHelmOps: (l: () => void) => {
    helmOps.notify.add(l);
    return () => helmOps.notify.delete(l);
  },
}));

const ctx = {
  name: "prod-eu", stableId: "prod", cluster: "c", server: "", isCurrent: false,
  sourceFile: "/home/dana/.kube/config", authKind: "client certificate",
};

beforeEach(() => {
  forwards.list = [];
  sessions.list = [];
  helmOps.list = [];
  resetView();
  resetProbes();
  resetLock();
});

/** Reads the console's open flag from outside the bar, the way the dock does. */
function Peek() {
  const { open } = useConsole();
  return <span data-testid="console-open">{String(open)}</span>;
}

function mount(node: ReactNode) {
  return render(
    <ConsoleProvider>
      {node}
      <Peek />
    </ConsoleProvider>,
  );
}

/** Connect the cluster for real through the probe, so link state is derived. */
async function connect(version: string | null) {
  const connectCluster = vi.fn().mockResolvedValue({ context: ctx.name, reachable: true, version });
  await act(async () => {
    await probeCluster(ctx, connectCluster as never);
  });
}

describe("Status", () => {
  it("says so when no cluster is active", () => {
    setState(defaultState([]));
    mount(<Status contexts={[]} />);
    expect(screen.getByText("No cluster")).toBeDefined();
    // Nothing to say about a version nobody asked for.
    expect(screen.queryByText("version unknown")).toBeNull();
  });

  it("names the active cluster, its version and its link", async () => {
    setState(defaultState([ctx]));
    mount(<Status contexts={[ctx]} />);
    await connect("v1.29.0");
    expect(screen.getByText("prod-eu")).toBeDefined();
    expect(screen.getByText("v1.29.0")).toBeDefined();
    expect(screen.getByText("Connected")).toBeDefined();
  });

  it("counts the port-forwards, in the plural the number calls for", () => {
    setState(defaultState([ctx]));
    forwards.list = [{ id: 1 }, { id: 2 }];
    mount(<Status contexts={[ctx]} />);
    expect(screen.getByRole("button", { name: "2 port-forwards" })).toBeDefined();
  });

  it("counts only the tunnels that are still alive", () => {
    setState(defaultState([ctx]));
    // A tunnel that gave up is not a tunnel in use. Two rows, one of them
    // dead — and the singular the remaining one calls for, which a count over
    // both would get wrong twice.
    forwards.list = [
      { id: 1, status: "active" },
      { id: 2, status: "failed" },
    ];
    mount(<Status contexts={[ctx]} />);
    expect(screen.getByRole("button", { name: "1 port-forward" })).toBeDefined();
  });

  it("says so on the strip when a tunnel has died", () => {
    setState(defaultState([ctx]));
    // The readout the reader actually had when a fifteen-minute-old forward
    // died: `0 PORT-FORWARDS` and nothing else. The count alone still says
    // nothing about the tunnel they are depending on.
    forwards.list = [{ id: 1, status: "failed" }];
    mount(<Status contexts={[ctx]} />);
    expect(screen.getByRole("button", { name: "0 port-forwards" })).toBeDefined();
    expect(screen.getByRole("button", { name: "1 forward failed" })).toBeDefined();
  });

  it("says nothing about failures when nothing has failed", () => {
    setState(defaultState([ctx]));
    forwards.list = [{ id: 1, status: "active" }];
    mount(<Status contexts={[ctx]} />);
    expect(screen.queryByRole("button", { name: /failed/i })).toBeNull();
  });

  it("counts idle sessions as live, alongside attached ones", () => {
    setState(defaultState([ctx]));
    // One of each running state. A count that only recognised `attached`
    // would say "1 shell" here — the same number a bug that forgot `idle`
    // would print — so this is the fixture that tells the two apart.
    sessions.list = [
      { id: 1, state: "attached" },
      { id: 2, state: "idle" },
    ];
    mount(<Status contexts={[ctx]} />);
    expect(screen.getByRole("button", { name: "2 shells" })).toBeDefined();
  });

  it("does not count a session that has closed", () => {
    setState(defaultState([ctx]));
    // Three rows, two live. The closed one stays out of the count but (unlike
    // a dead forward, which this file has its own segment for) does not get
    // one of its own here — the rail is where a reader goes to see why it died.
    sessions.list = [
      { id: 1, state: "attached" },
      { id: 2, state: "idle" },
      { id: 3, state: "closed" },
    ];
    mount(<Status contexts={[ctx]} />);
    expect(screen.getByRole("button", { name: "2 shells" })).toBeDefined();
  });

  it("says nothing about shells when none are live", () => {
    setState(defaultState([ctx]));
    sessions.list = [{ id: 1, state: "closed" }];
    mount(<Status contexts={[ctx]} />);
    expect(screen.queryByRole("button", { name: /shell/i })).toBeNull();
  });

  it("counts the helm operations still changing the cluster", () => {
    setState(defaultState([ctx]));
    helmOps.list = [
      { id: 1, state: "running", context: ctx.name },
      { id: 2, state: "running", context: ctx.name },
    ];
    mount(<Status contexts={[ctx]} />);
    expect(screen.getByRole("button", { name: "2 helm operations" })).toBeDefined();
  });

  it("counts only the operations still in flight", () => {
    setState(defaultState([ctx]));
    // A `done` upgrade has finished changing the cluster and a `failed` one
    // has stopped trying; neither is in flight. Three rows, one running — and
    // the singular that one calls for, which a count over all three would get
    // wrong twice.
    helmOps.list = [
      { id: 1, state: "running", context: ctx.name },
      { id: 2, state: "done", context: ctx.name },
      { id: 3, state: "failed", context: ctx.name },
    ];
    mount(<Status contexts={[ctx]} />);
    expect(screen.getByRole("button", { name: "1 helm operation" })).toBeDefined();
    // And the `done` one is not folded into the failure either: "finished" and
    // "failed" are two states, and a segment toned `sev` may only count one.
    expect(screen.getByRole("button", { name: "1 helm operation failed" })).toBeDefined();
  });

  it("says nothing about helm when nothing is in flight and nothing failed", () => {
    setState(defaultState([ctx]));
    helmOps.list = [{ id: 1, state: "done", context: ctx.name }];
    mount(<Status contexts={[ctx]} />);
    expect(screen.queryByRole("button", { name: /helm/i })).toBeNull();
    // Named as well as counted: `0 helm operations` is the exact readout the
    // absent-at-zero rule exists to keep off a strip already carrying five.
    expect(screen.queryByText(/0 helm operation/i)).toBeNull();
  });

  it("says so on the strip when a helm operation has failed", () => {
    setState(defaultState([ctx]));
    // The reader closed the dialog and the upgrade failed after it had gone.
    // Nothing else on screen reports that, and the count cannot: an operation
    // that fails leaves nothing in flight, which is what a reader who started
    // no operation at all sees.
    helmOps.list = [{ id: 1, state: "failed", context: ctx.name }];
    mount(<Status contexts={[ctx]} />);
    expect(screen.getByRole("button", { name: "1 helm operation failed" })).toBeDefined();
    expect(screen.queryByRole("button", { name: "0 helm operations" })).toBeNull();
  });

  it("tells a failed operation apart from the ones still running", () => {
    setState(defaultState([ctx]));
    // Both segments at once, and both saying "1": the fixture that catches an
    // assertion which would pass for either one on its own.
    helmOps.list = [
      { id: 1, state: "running", context: ctx.name },
      { id: 2, state: "failed", context: ctx.name },
    ];
    mount(<Status contexts={[ctx]} />);
    expect(screen.getByRole("button", { name: "1 helm operation" })).toBeDefined();
    expect(screen.getByRole("button", { name: "1 helm operation failed" })).toBeDefined();
  });

  it("opens the helm screen from either helm readout", async () => {
    setState(defaultState([ctx]));
    helmOps.list = [{ id: 1, state: "running", context: ctx.name }];
    mount(<Status contexts={[ctx]} />);
    await userEvent.click(screen.getByRole("button", { name: "1 helm operation" }));
    expect(activeRoute()).toBe("/helm");
  });

  it("opens the helm screen from the failed readout", async () => {
    setState(defaultState([ctx]));
    helmOps.list = [{ id: 1, state: "failed", context: ctx.name }];
    mount(<Status contexts={[ctx]} />);
    await userEvent.click(screen.getByRole("button", { name: "1 helm operation failed" }));
    expect(activeRoute()).toBe("/helm");
  });

  it("counts only the helm operations on the cluster the reader is looking at", () => {
    setState(defaultState([ctx]));
    // Two upgrades in flight, one of them on a cluster this window is not
    // showing. The strip names `prod-eu` and the segment beside it opens the
    // Helm screen, which lists `prod-eu` and nothing else — so a count of two
    // would send the reader somewhere the second one cannot be seen.
    helmOps.list = [
      { id: 1, state: "running", context: ctx.name },
      { id: 2, state: "running", context: "staging-us" },
    ];
    mount(<Status contexts={[ctx]} />);
    expect(screen.getByRole("button", { name: "1 helm operation" })).toBeDefined();
    expect(screen.queryByRole("button", { name: "2 helm operations" })).toBeNull();
  });

  it("does not report a failure from a cluster this window is not looking at", () => {
    setState(defaultState([ctx]));
    // The `sev` tone is a summons, and this one would lead nowhere: the Helm
    // screen the segment opens lists `prod-eu`, so a `staging-us` failure
    // shown here is an alarm with no page behind it. It comes back the moment
    // the reader switches to that cluster; the row stays in the store until
    // it is dismissed.
    helmOps.list = [{ id: 1, state: "failed", context: "staging-us" }];
    mount(<Status contexts={[ctx]} />);
    expect(screen.queryByRole("button", { name: /helm/i })).toBeNull();
  });

  it("says nothing about helm when no cluster is active", () => {
    setState(defaultState([]));
    // No cluster means no Helm screen to open — the screen asks for a context
    // before it lists anything — so there is nothing to count towards.
    helmOps.list = [
      { id: 1, state: "running", context: ctx.name },
      { id: 2, state: "failed", context: ctx.name },
    ];
    mount(<Status contexts={[]} />);
    expect(screen.queryByRole("button", { name: /helm/i })).toBeNull();
  });

  it("still counts forwards and shells from every cluster, the way their screens list them", () => {
    setState(defaultState([ctx]));
    // The scoping above is not a rule about the strip, it is a rule about
    // each segment agreeing with where it sends the reader. `/forwards` and
    // `/terminals` list every cluster's rows, so these two counts stay whole
    // — narrowing them to `prod-eu` would hide a tunnel the reader is
    // depending on from the only surface that reports it.
    forwards.list = [{ id: 1, status: "active", context: "staging-us" }];
    sessions.list = [{ id: 1, state: "attached", context: "staging-us" }];
    mount(<Status contexts={[ctx]} />);
    expect(screen.getByRole("button", { name: "1 port-forward" })).toBeDefined();
    expect(screen.getByRole("button", { name: "1 shell" })).toBeDefined();
  });

  it("opens the console from Ask", async () => {
    setState(defaultState([ctx]));
    mount(<Status contexts={[ctx]} />);
    expect(screen.getByTestId("console-open").textContent).toBe("false");
    await userEvent.click(screen.getByRole("button", { name: "Ask" }));
    expect(screen.getByTestId("console-open").textContent).toBe("true");
  });

  /**
   * Spec decision 5, at this component's own boundary. Eight `onSelect`s here
   * called `openTab`, and behind a raised cover every one of them was a way
   * into the workspace from a window that looked sealed. The handlers are
   * dropped in one place, so a segment added later is covered too — which is
   * what this asserts rather than naming the eight.
   */
  it("is readouts only while the vault is sealed", () => {
    setState(defaultState([ctx]));
    forwards.list = [{ id: 1, status: "active", context: ctx.name }];
    sessions.list = [{ id: 1, state: "attached", context: ctx.name }];
    lockWorkspace();
    mount(<Status contexts={[ctx]} />);
    const strip = screen.getByRole("group", { name: "Status" });
    expect(strip.querySelectorAll("button")).toHaveLength(0);
    // The readouts stay: a kubeconfig context name and counts of work this
    // window started are not things the vault ever sealed.
    expect(strip.textContent ?? "").toContain(ctx.name);
    expect(strip.textContent ?? "").toContain("1 port-forward");
  });

  it("gives the segments their handlers back when the vault opens", () => {
    setState(defaultState([ctx]));
    lockWorkspace();
    mount(<Status contexts={[ctx]} />);
    expect(screen.getByRole("group", { name: "Status" }).querySelectorAll("button")).toHaveLength(0);
    act(() => resetLock());
    expect(screen.getByRole("button", { name: "Ask" })).toBeDefined();
  });
});
