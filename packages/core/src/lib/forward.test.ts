import { afterEach, describe, it, expect, vi, beforeEach } from "vitest";

const { invokeCommandMock, onMock } = vi.hoisted(() => ({
  invokeCommandMock: vi.fn(),
  onMock: vi.fn(),
}));
vi.mock("../transport/transport", () => ({
  invokeCommand: invokeCommandMock,
  on: onMock,
}));

import {
  startPortForward,
  stopPortForward,
  rehydrateForwards,
  getForwards,
  subscribeForwards,
  forwardUrl,
  forwardAddress,
  isForwardEnded,
  __resetForwardStoreForTests,
} from "./forward";
import { describeError } from "./errors";

vi.mock("./notify", () => ({ notify: { error: vi.fn(), success: vi.fn(), info: vi.fn() } }));
import { notify } from "./notify";

/** What `list_forwards` hands back, as the Rust `ForwardEntry` serialises it. */
interface BackendEntry {
  id: number;
  context: string;
  namespace: string;
  kind: string;
  name: string;
  remotePort: number;
  localPort: number;
  startedAt: number;
  bytes: number;
  /** True once the manager's own task has given up on this tunnel. */
  ended: boolean;
  /** Why it gave up, when the loop had a reason. */
  error: string | null;
}

// Deliberately in the past: a `startedAt` sourced from the local clock would
// land in the present day and fail every assertion that names this constant.
const BACKEND_EPOCH = 1_700_000_000_000;

// Every `forward:*` handler the store registers, so tests can fire the events.
const handlers = new Map<string, (payload?: unknown) => void>();

// A minimal stand-in for `ForwardManager`: `start_port_forward` registers an
// entry, `list_forwards` reports what it holds, `stop_port_forward` drops one.
// Tests reach into `backend` directly to set up the cases that matter — a
// tunnel this client never started, or one the manager still holds after the
// store gave up on it.
let backend: BackendEntry[] = [];
let nextId = 1;
let listFails: unknown = null;

function fire(channel: string, payload?: unknown) {
  const handler = handlers.get(channel);
  if (!handler) throw new Error(`nothing is listening on ${channel}`);
  handler(payload);
}

const traffic = (id: number, bytes: number) => fire(`forward:traffic:${id}`, { bytes });
/** `forward:closed:<id>` carries the loop's final error, or null for a clean end. */
const closed = (id: number, reason?: string) => fire(`forward:closed:${id}`, reason ?? null);
const statusEvent = (id: number, state: string, error?: string) =>
  fire(`forward:status:${id}`, { state, attempt: 5, error: error ?? null });

beforeEach(() => {
  invokeCommandMock.mockReset();
  onMock.mockReset();
  handlers.clear();
  onMock.mockImplementation((channel: string, handler: (payload?: unknown) => void) => {
    handlers.set(channel, handler);
    return () => handlers.delete(channel);
  });

  backend = [];
  nextId = 1;
  listFails = null;
  invokeCommandMock.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
    if (command === "start_port_forward") {
      const id = nextId++;
      backend.push({
        id,
        context: String(args?.context),
        namespace: String(args?.namespace),
        kind: String(args?.kind),
        name: String(args?.name),
        remotePort: Number(args?.remotePort),
        localPort: (args?.localPort as number | null) ?? 50000 + id,
        startedAt: BACKEND_EPOCH + id,
        bytes: 0,
        ended: false,
        error: null,
      });
      const started = backend[backend.length - 1];
      return { id, localPort: started.localPort, startedAt: started.startedAt };
    }
    if (command === "list_forwards") {
      if (listFails) throw new Error(String(listFails));
      return { forwards: backend.map((e) => ({ ...e })) };
    }
    if (command === "stop_port_forward") {
      backend = backend.filter((e) => e.id !== args?.id);
      return undefined;
    }
    throw new Error(`unexpected command ${command}`);
  });

  vi.mocked(notify.error).mockClear();
  __resetForwardStoreForTests();
});

const req = {
  context: "kind-dev",
  namespace: "default",
  kind: "Pod",
  name: "web-1",
  remotePort: 80,
};

const otherReq = { ...req, name: "api-2", remotePort: 8080 };

function entry(over: Partial<BackendEntry> & { id: number }): BackendEntry {
  return {
    context: "kind-dev",
    namespace: "default",
    kind: "Service",
    name: "redis",
    remotePort: 6379,
    localPort: 61000 + over.id,
    startedAt: BACKEND_EPOCH + over.id,
    bytes: 0,
    ended: false,
    error: null,
    ...over,
  };
}

describe("forward store", () => {
  it("adds a forward and notifies subscribers", async () => {
    const changed = vi.fn();
    const unsub = subscribeForwards(changed);

    const fwd = await startPortForward(req);

    expect(fwd).toMatchObject({ id: 1, localPort: 50001, name: "web-1", remotePort: 80 });
    expect(getForwards()).toHaveLength(1);
    expect(changed).toHaveBeenCalled();
    expect(invokeCommandMock).toHaveBeenCalledWith("start_port_forward", {
      context: "kind-dev",
      namespace: "default",
      kind: "Pod",
      name: "web-1",
      remotePort: 80,
      localPort: null,
    });
    unsub();
  });

  it("stops a forward and removes it from the store", async () => {
    const fwd = await startPortForward(req);
    expect(getForwards()).toHaveLength(1);

    await stopPortForward(fwd.id);

    expect(getForwards()).toHaveLength(0);
    expect(invokeCommandMock).toHaveBeenCalledWith("stop_port_forward", { id: fwd.id });
  });

  it("keeps a forward the backend closed, marked failed rather than deleted", async () => {
    // The defect this file used to assert. A tunnel that dies underneath the
    // reader is NEWS: deleting the row left a fifteen-minute-old forward
    // gone from the screen with no trace, and the reader assuming a tunnel
    // they depend on is fine when it is dead.
    const fwd = await startPortForward(req);
    expect(getForwards()).toHaveLength(1);

    // Simulate the backend serve-loop ending.
    closed(fwd.id);

    expect(getForwards()).toHaveLength(1);
    expect(getForwards()[0].status).toBe("failed");
  });

  it("defaults a new forward's status to active", async () => {
    const fwd = await startPortForward(req);
    expect(fwd.status).toBe("active");
    expect(getForwards()[0].status).toBe("active");
  });

  it("flips status to reconnecting when the backend emits forward:status", async () => {
    const changed = vi.fn();
    const fwd = await startPortForward(req);
    const unsub = subscribeForwards(changed);

    statusEvent(fwd.id, "reconnecting");

    expect(getForwards()[0].status).toBe("reconnecting");
    expect(changed).toHaveBeenCalled();
    unsub();
  });

  it("passes a preferred local port through", async () => {
    const fwd = await startPortForward({ ...req, localPort: 8080 });
    expect(fwd.localPort).toBe(8080);
    expect(invokeCommandMock).toHaveBeenCalledWith(
      "start_port_forward",
      expect.objectContaining({ localPort: 8080 }),
    );
  });
});

describe("a forward's age", () => {
  it("dates a forward this session started from the backend, not the local clock", async () => {
    const fwd = await startPortForward(req);

    expect(fwd.startedAt).toBe(BACKEND_EPOCH + 1);
    expect(getForwards()[0].startedAt).toBe(BACKEND_EPOCH + 1);
  });

  it("starts a new forward's traffic at what the backend has counted", async () => {
    const fwd = await startPortForward(req);
    expect(fwd.bytesMoved).toBe(0);
    expect(getForwards()[0].bytesMoved).toBe(0);
  });
});

describe("traffic", () => {
  it("updates one forward's bytesMoved and leaves the others untouched", async () => {
    const a = await startPortForward(req);
    const b = await startPortForward(otherReq);
    const before = getForwards();
    const bBefore = before.find((f) => f.id === b.id);

    traffic(a.id, 2048);

    const after = getForwards();
    expect(after).not.toBe(before);
    expect(after.find((f) => f.id === a.id)?.bytesMoved).toBe(2048);
    // The untouched row keeps its identity, so a memoised row does not re-render.
    expect(after.find((f) => f.id === b.id)).toBe(bBefore);
  });

  it("takes the event's total as the total, not as a delta", async () => {
    const fwd = await startPortForward(req);

    traffic(fwd.id, 1024);
    traffic(fwd.id, 4096);

    expect(getForwards()[0].bytesMoved).toBe(4096);
  });

  it("emits nothing and keeps the snapshot when the total has not changed", async () => {
    const fwd = await startPortForward(req);
    traffic(fwd.id, 4096);
    const snapshot = getForwards();
    const changed = vi.fn();
    const unsub = subscribeForwards(changed);

    // The same running total the store already holds — the backend only fires
    // on a change, but a repeat must not wake `useSyncExternalStore`.
    traffic(fwd.id, 4096);

    expect(changed).not.toHaveBeenCalled();
    expect(getForwards()).toBe(snapshot);
    unsub();
  });

  it("ignores a traffic event with no usable byte count", async () => {
    const fwd = await startPortForward(req);
    traffic(fwd.id, 512);
    const snapshot = getForwards();

    fire(`forward:traffic:${fwd.id}`, { bytes: "lots" });
    fire(`forward:traffic:${fwd.id}`, null);

    expect(getForwards()).toBe(snapshot);
    expect(getForwards()[0].bytesMoved).toBe(512);
  });
});

describe("rehydrateForwards", () => {
  it("populates the store from the backend", async () => {
    backend = [entry({ id: 9, name: "redis", bytes: 8192 }), entry({ id: 10, name: "pg" })];
    const changed = vi.fn();
    const unsub = subscribeForwards(changed);

    await rehydrateForwards();

    expect(getForwards()).toHaveLength(2);
    expect(getForwards()[0]).toMatchObject({
      id: 9,
      kind: "Service",
      name: "redis",
      namespace: "default",
      remotePort: 6379,
      localPort: 61009,
      startedAt: BACKEND_EPOCH + 9,
      bytesMoved: 8192,
      status: "active",
    });
    expect(changed).toHaveBeenCalled();
    unsub();
  });

  it("does not duplicate a forward this session already knows", async () => {
    const mine = await startPortForward(req);
    backend.push(entry({ id: 42 }));
    const before = getForwards().find((f) => f.id === mine.id);

    await rehydrateForwards();

    expect(getForwards().map((f) => f.id).sort()).toEqual([mine.id, 42].sort());
    expect(getForwards().filter((f) => f.id === mine.id)).toHaveLength(1);
    // The row this session owns keeps its identity rather than being replaced.
    expect(getForwards().find((f) => f.id === mine.id)).toBe(before);
  });

  it("does not duplicate a tunnel it already knows has died", async () => {
    const fwd = await startPortForward(req);
    // A forward that exhausts its retries emits `forward:closed` but stays in
    // the manager's map until `stop` is called, so `list` still reports it —
    // now saying, itself, that it has ended.
    statusEvent(fwd.id, "failed", "pod web-1 was deleted");
    closed(fwd.id);
    backend = [entry({ id: fwd.id, kind: "Pod", name: "web-1", ended: true, error: "pod web-1 was deleted" })];

    await rehydrateForwards();

    expect(getForwards()).toHaveLength(1);
    expect(getForwards()[0].status).toBe("failed");
    expect(getForwards()[0].error).toBe("pod web-1 was deleted");
  });

  it("adopts a tunnel that died before this page existed as dead, not as active", async () => {
    // The case no bookkeeping in this module can reach. `dropped` is
    // module-level JavaScript that a browser reload wipes, and the
    // `forward:closed` event fired before the page existed — so a store with
    // nothing dropped meets a manager still holding a tunnel that gave up.
    // It used to adopt it as `active`: a green row for a tunnel that cannot
    // carry a byte, and a `/pf/<id>/` URL that will never answer.
    backend = [entry({ id: 9, name: "redis", ended: true, error: "connection refused" })];

    await rehydrateForwards();

    expect(getForwards()).toHaveLength(1);
    expect(getForwards()[0].status).toBe("failed");
    expect(getForwards()[0].error).toBe("connection refused");
    expect(isForwardEnded(getForwards()[0])).toBe(true);
  });

  it("does not resurrect a stopped forward from a list call already in flight", async () => {
    const fwd = await startPortForward(req);
    await stopPortForward(fwd.id);
    // The snapshot a concurrent `list_forwards` took before the stop landed.
    backend = [entry({ id: fwd.id, kind: "Pod", name: "web-1" })];

    await rehydrateForwards();

    expect(getForwards()).toHaveLength(0);
  });

  it("keeps the snapshot when the backend reports only what the store holds", async () => {
    await startPortForward(req);
    const snapshot = getForwards();
    const changed = vi.fn();
    const unsub = subscribeForwards(changed);

    await rehydrateForwards();

    expect(getForwards()).toBe(snapshot);
    expect(changed).not.toHaveBeenCalled();
    unsub();
  });

  it("watches a rehydrated forward's traffic and closure", async () => {
    backend = [entry({ id: 9 })];
    await rehydrateForwards();

    traffic(9, 65536);
    expect(getForwards()[0].bytesMoved).toBe(65536);

    closed(9, "the pod went away");
    expect(getForwards()).toHaveLength(1);
    expect(getForwards()[0].status).toBe("failed");
    expect(getForwards()[0].error).toBe("the pod went away");
  });

  it("stops a rehydrated forward through the same command", async () => {
    backend = [entry({ id: 9 })];
    await rehydrateForwards();

    await stopPortForward(9);

    expect(getForwards()).toHaveLength(0);
    expect(invokeCommandMock).toHaveBeenCalledWith("stop_port_forward", { id: 9 });
  });

  it("reports a failed listing through describeError instead of throwing", async () => {
    listFails = "handler error: list forwards timed out";

    await expect(rehydrateForwards()).resolves.toBeUndefined();

    expect(notify.error).toHaveBeenCalledWith(
      expect.any(String),
      describeError("handler error: list forwards timed out").detail,
    );
    expect(getForwards()).toHaveLength(0);
  });

  it("starts a forward with no follow-up list_forwards call, even when listing would fail", async () => {
    // The start response carries its own startedAt now, so a broken
    // list_forwards must not be able to break a start that is otherwise
    // fine — the whole point of dropping the follow-up read.
    listFails = "handler error: list forwards timed out";

    const fwd = await startPortForward(req);

    expect(fwd.startedAt).toBe(BACKEND_EPOCH + 1);
    expect(getForwards()).toHaveLength(1);
    expect(invokeCommandMock).toHaveBeenCalledTimes(1);
    expect(invokeCommandMock).toHaveBeenCalledWith("start_port_forward", expect.anything());
  });
});

describe("a tunnel that dies on its own", () => {
  /** The backend's own sequence when a forward exhausts its retries. */
  async function giveUp(reason: string) {
    const fwd = await startPortForward(req);
    statusEvent(fwd.id, "failed", reason);
    closed(fwd.id, reason);
    return fwd;
  }

  it("keeps the reason the backend gave, so the row can say why", async () => {
    await giveUp("connection refused connecting to 10.1.2.3:80");

    expect(getForwards()[0].error).toBe("connection refused connecting to 10.1.2.3:80");
  });

  it("keeps the reason from the failed status when the closure carries none", async () => {
    const fwd = await startPortForward(req);
    statusEvent(fwd.id, "failed", "pod web-1 was deleted");

    // `forward:closed` fires a moment later with nothing attached.
    closed(fwd.id);

    expect(getForwards()[0].status).toBe("failed");
    expect(getForwards()[0].error).toBe("pod web-1 was deleted");
  });

  it("takes the reason from the closure itself when that is all there is", async () => {
    const fwd = await startPortForward(req);

    // No `failed` status first: the serve loop ended on its own terms.
    closed(fwd.id, "the pod went away");

    expect(getForwards()[0].status).toBe("failed");
    expect(getForwards()[0].error).toBe("the pod went away");
  });

  it("invents no reason for a close that came without one", async () => {
    const fwd = await startPortForward(req);

    closed(fwd.id);

    expect(getForwards()[0].status).toBe("failed");
    expect(getForwards()[0].error).toBeUndefined();
  });

  it("clears a stale reason when the tunnel comes back", async () => {
    const fwd = await startPortForward(req);
    statusEvent(fwd.id, "reconnecting", "connection reset by peer");
    expect(getForwards()[0].error).toBe("connection reset by peer");

    statusEvent(fwd.id, "active");

    // A live tunnel carrying last week's excuse would be read as broken.
    expect(getForwards()[0].error).toBeUndefined();
  });

  it("wakes nobody when the closure only confirms the status before it", async () => {
    // The backend's own sequence: `forward:status` with `failed` and the
    // reason, then `forward:closed` a moment later saying the same thing.
    // The second event tells the store nothing it does not already hold, and
    // `getForwards` must hand back the SAME array — `useSyncExternalStore`
    // re-renders forever otherwise, and a mutation that rebuilt the row
    // unconditionally survived the rest of this file.
    const fwd = await startPortForward(req);
    statusEvent(fwd.id, "failed", "connection refused");
    const snapshot = getForwards();
    const row = snapshot[0];
    const changed = vi.fn();
    const unsub = subscribeForwards(changed);

    closed(fwd.id, "connection refused");

    expect(changed).not.toHaveBeenCalled();
    expect(getForwards()).toBe(snapshot);
    expect(getForwards()[0]).toBe(row);
    unsub();
  });

  it("stops listening to a tunnel that has stopped", async () => {
    const fwd = await giveUp("connection refused");

    // Nothing further is coming, and an unclosed subscription per dead row is
    // a leak the reader pays for by leaving the screen open.
    expect(handlers.has(`forward:closed:${fwd.id}`)).toBe(false);
    expect(handlers.has(`forward:status:${fwd.id}`)).toBe(false);
    expect(handlers.has(`forward:traffic:${fwd.id}`)).toBe(false);
  });

  it("keeps the snapshot when a rehydrate meets a tunnel it already knows is dead", async () => {
    // `getForwards` must hand back the SAME array until something really
    // changed: `useSyncExternalStore` re-renders forever otherwise, and every
    // mount of the forwards screen rehydrates.
    const fwd = await giveUp("connection refused");
    backend = [entry({ id: fwd.id, kind: "Pod", name: "web-1", ended: true, error: "connection refused" })];
    const snapshot = getForwards();
    const changed = vi.fn();
    const unsub = subscribeForwards(changed);

    await rehydrateForwards();
    await rehydrateForwards();

    expect(changed).not.toHaveBeenCalled();
    expect(getForwards()).toBe(snapshot);
    unsub();
  });

  it("leaves every other row's identity alone", async () => {
    const a = await startPortForward(req);
    const b = await startPortForward(otherReq);
    const bBefore = getForwards().find((f) => f.id === b.id);

    closed(a.id, "connection refused");

    expect(getForwards().find((f) => f.id === b.id)).toBe(bBefore);
    expect(getForwards().find((f) => f.id === a.id)?.status).toBe("failed");
  });

  it("is dead, where a reconnecting one is not", async () => {
    const flapping = await startPortForward(req);
    const dying = await startPortForward(otherReq);
    statusEvent(flapping.id, "reconnecting", "connection reset by peer");
    closed(dying.id, "connection refused");

    const [live, dead] = getForwards();
    // A tunnel that is reconnecting is coming back; one that gave up is gone.
    expect(isForwardEnded(live)).toBe(false);
    expect(isForwardEnded(dead)).toBe(true);
  });

  it("goes for good when the reader dismisses it, and tells the backend so", async () => {
    // Dismissal is `stop_port_forward`, not a local delete: the manager keeps
    // a gave-up forward in its map until `stop` is called, and a set of
    // dropped ids in this module cannot outlive the page that holds it.
    const fwd = await giveUp("connection refused");
    expect(getForwards()).toHaveLength(1);

    await stopPortForward(fwd.id);

    expect(getForwards()).toHaveLength(0);
    expect(invokeCommandMock).toHaveBeenCalledWith("stop_port_forward", { id: fwd.id });
  });

  it("stays gone when a listing already in flight still reports it", async () => {
    const fwd = await giveUp("connection refused");
    await stopPortForward(fwd.id);
    // The snapshot a concurrent `list_forwards` took before the stop landed.
    backend = [entry({ id: fwd.id, kind: "Pod", name: "web-1", ended: true, error: "connection refused" })];

    await rehydrateForwards();

    expect(getForwards()).toHaveLength(0);
  });
});

describe("forwardUrl", () => {
  afterEach(() => {
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  });
  it("uses localhost:port on desktop", () => {
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    expect(forwardUrl({ id: 3, localPort: 5000 })).toBe("http://localhost:5000");
  });
  it("uses the /pf proxy path on web", () => {
    expect(forwardUrl({ id: 3, localPort: 5000 })).toBe("/pf/3/");
  });
});

describe("forwardAddress", () => {
  afterEach(() => {
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  });
  it("uses localhost:port on desktop", () => {
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    expect(forwardAddress({ id: 3, localPort: 5000 })).toBe("localhost:5000");
  });
  it("uses the absolute /pf proxy URL on web", () => {
    expect(forwardAddress({ id: 3, localPort: 5000 })).toBe(`${window.location.origin}/pf/3/`);
  });
});
