import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

const startHelmOp = vi.fn();

vi.mock("@srelens/core", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@srelens/core")>();
  return {
    ...actual,
    startHelmOp: (...args: unknown[]) => startHelmOp(...args),
  };
});

// Imported after the mock so the store binds the double rather than the real
// Tauri-backed streamer.
const helmOps = await import("./helmOps");
const {
  __resetHelmOpsForTests,
  dismissHelmOp,
  getHelmOps,
  startHelmOperation,
  subscribeHelmOps,
} = helmOps;

/**
 * A `startHelmOp` double. The test drives the backend side through
 * `out`/`exit`, and asserts on the handle the store was given.
 */
function fakeHelm() {
  let onData!: (line: string) => void;
  let onExit!: (err: string | null) => void;
  const handle = { close: vi.fn() };
  startHelmOp.mockImplementation(
    (
      _context: string,
      _args: string[],
      data: (line: string) => void,
      exit: (err: string | null) => void,
    ) => {
      onData = data;
      onExit = exit;
      return Promise.resolve(handle);
    },
  );
  return {
    handle,
    out: (line: string) => onData(line),
    exit: (err: string | null = null) => onExit(err),
  };
}

const upgrade = {
  kind: "upgrade" as const,
  release: "checkout-api",
  namespace: "payments",
  context: "prod",
  args: ["upgrade", "checkout-api", "charts/api", "--wait"],
};

/** Push past the second the store is coalescing output into, so whatever it
 *  buffered lands in the snapshot. */
function turnTheSecond() {
  vi.advanceTimersByTime(1000);
}

beforeEach(() => {
  __resetHelmOpsForTests();
  startHelmOp.mockReset();
});

afterEach(() => {
  __resetHelmOpsForTests();
  vi.useRealTimers();
});

describe("starting an operation", () => {
  it("lists it as running, saying what it is", async () => {
    fakeHelm();

    const id = await startHelmOperation(upgrade);

    expect(getHelmOps()).toEqual([
      {
        id,
        kind: "upgrade",
        release: "checkout-api",
        namespace: "payments",
        context: "prod",
        state: "running",
        output: [],
        startedAt: expect.any(Number),
      },
    ]);
  });

  it("hands core the context, argv, kubeconfigs and values it was given", async () => {
    fakeHelm();

    await startHelmOperation({ ...upgrade, extraKubeconfigs: ["/tmp/kc"], values: "replicaCount: 2" });

    const [context, args, , , extra, values] = startHelmOp.mock.calls[0] as [
      string,
      string[],
      unknown,
      unknown,
      string[],
      string,
    ];
    expect(context).toBe("prod");
    expect(args).toEqual(["upgrade", "checkout-api", "charts/api", "--wait"]);
    expect(extra).toEqual(["/tmp/kc"]);
    expect(values).toBe("replicaCount: 2");
  });

  it("leaves a failed row carrying the reason when the backend refuses to start", async () => {
    startHelmOp.mockRejectedValue(new Error("handler error: helm binary not found"));

    const id = await startHelmOperation(upgrade);

    const [row] = getHelmOps();
    expect(row.id).toBe(id);
    expect(row.state).toBe("failed");
    // Through `describeError`: the backend's wrappers are not the reader's
    // problem.
    expect(row.error).not.toContain("handler error:");
    expect(row.error).toContain("helm binary not found");
  });
});

describe("what the operation prints", () => {
  beforeEach(() => {
    vi.useFakeTimers({ now: new Date("2026-08-26T10:00:00.000Z") });
  });

  it("accumulates every line, in order", async () => {
    const helm = fakeHelm();
    await startHelmOperation(upgrade);

    helm.out("Release \"checkout-api\" has been upgraded.");
    helm.out("NAME: checkout-api");
    helm.out("LAST DEPLOYED: Wed Aug 26");
    turnTheSecond();

    expect(getHelmOps()[0].output).toEqual([
      "Release \"checkout-api\" has been upgraded.",
      "NAME: checkout-api",
      "LAST DEPLOYED: Wed Aug 26",
    ]);
  });

  it("keeps the last lines a finished operation printed, without waiting for a clock", async () => {
    const helm = fakeHelm();
    await startHelmOperation(upgrade);

    helm.out("first");
    helm.out("second");
    helm.exit(null);

    const [row] = getHelmOps();
    expect(row.state).toBe("done");
    expect(row.output).toEqual(["first", "second"]);
  });
});

describe("how an operation ends", () => {
  it("moves to done on a clean exit and stops watching", async () => {
    const helm = fakeHelm();
    await startHelmOperation(upgrade);

    helm.exit(null);

    expect(getHelmOps()[0].state).toBe("done");
    expect(getHelmOps()[0].error).toBeUndefined();
    // `close()` unsubscribes this window from the stream. The cluster
    // mutation, clean or not, was over before this ran.
    expect(helm.handle.close).toHaveBeenCalledTimes(1);
  });

  it("moves to failed and keeps the reason, described, beside the output", async () => {
    const helm = fakeHelm();
    await startHelmOperation(upgrade);

    helm.out("Error: UPGRADE FAILED: another operation is in progress");
    helm.exit("handler error: Error: UPGRADE FAILED: another operation is in progress");

    const [row] = getHelmOps();
    expect(row.state).toBe("failed");
    expect(row.error).not.toContain("handler error:");
    expect(row.error).toBe("UPGRADE FAILED: another operation is in progress");
    // The output is the only place a failed upgrade's reason is spelled out.
    expect(row.output).toEqual(["Error: UPGRADE FAILED: another operation is in progress"]);
  });

  it("leaves a failed operation listed until the reader dismisses it", async () => {
    const helm = fakeHelm();
    const id = await startHelmOperation(upgrade);
    helm.exit("boom");
    expect(getHelmOps()).toHaveLength(1);

    dismissHelmOp(id);

    expect(getHelmOps()).toEqual([]);
  });

  it("dismisses a finished operation and drops nothing else", async () => {
    const first = fakeHelm();
    const firstId = await startHelmOperation(upgrade);
    fakeHelm();
    await startHelmOperation({ ...upgrade, release: "web", kind: "install" });
    first.exit(null);

    dismissHelmOp(firstId);

    expect(getHelmOps().map((o) => o.release)).toEqual(["web"]);
  });
});

describe("an operation in flight", () => {
  it("stays listed when the reader tries to dismiss it, because nothing here cancels", async () => {
    const helm = fakeHelm();
    const id = await startHelmOperation(upgrade);

    dismissHelmOp(id);

    // A `helm upgrade` keeps mutating the cluster whether srelens watches or
    // not. Removing the row would read as a cancel, and it is not one.
    expect(getHelmOps()).toHaveLength(1);
    expect(getHelmOps()[0].state).toBe("running");
    expect(helm.handle.close).not.toHaveBeenCalled();
  });

  it("exports no name that offers to cancel one", () => {
    const named = Object.keys(helmOps);
    expect(named).not.toHaveLength(0);
    expect(named.filter((n) => /cancel|abort|kill|terminate/i.test(n))).toEqual([]);
  });
});

describe("the store's snapshot", () => {
  it("is the same reference until something changes", async () => {
    fakeHelm();
    await startHelmOperation(upgrade);

    const before = getHelmOps();
    expect(getHelmOps()).toBe(before);
  });

  it("survives a chatty second of output, and loses none of it", async () => {
    vi.useFakeTimers({ now: new Date("2026-08-26T10:00:00.000Z") });
    const helm = fakeHelm();
    await startHelmOperation(upgrade);
    helm.out("line 1");
    const before = getHelmOps();
    const notified = vi.fn();
    subscribeHelmOps(notified);

    // A `helm upgrade --wait` narrating itself: many lines, spread across real
    // milliseconds, inside one second. Firing them on a frozen clock would
    // prove nothing — millisecond-resolution coalescing would hold its
    // identity too.
    for (let i = 0; i < 50; i++) {
      vi.advanceTimersByTime(10);
      helm.out(`line ${i + 2}`);
    }
    expect(Date.now() - before[0].startedAt).toBeGreaterThan(400);

    expect(getHelmOps()).toBe(before);
    expect(before[0].output).toEqual(["line 1"]);
    expect(notified).not.toHaveBeenCalled();

    // Buffered, not dropped: the reader sees all 51 once the second turns.
    turnTheSecond();
    expect(getHelmOps()[0].output).toHaveLength(51);
    expect(getHelmOps()[0].output[50]).toBe("line 51");
    expect(notified).toHaveBeenCalledTimes(1);
  });

  it("rebuilds once the second has turned, and once per second only", async () => {
    vi.useFakeTimers({ now: new Date("2026-08-26T10:00:00.000Z") });
    const helm = fakeHelm();
    await startHelmOperation(upgrade);
    helm.out("line 1");
    const before = getHelmOps();
    const notified = vi.fn();
    subscribeHelmOps(notified);

    vi.advanceTimersByTime(1000);
    helm.out("line 2");
    helm.out("line 3");

    const after = getHelmOps();
    expect(after).not.toBe(before);
    // The second turned, so line 2 lands at once; line 3 shares its second and
    // waits for the next turn rather than waking every subscriber again.
    expect(after[0].output).toEqual(["line 1", "line 2"]);
    expect(notified).toHaveBeenCalledTimes(1);

    turnTheSecond();
    expect(getHelmOps()[0].output).toEqual(["line 1", "line 2", "line 3"]);
    expect(notified).toHaveBeenCalledTimes(2);
  });

  it("leaves an untouched row alone when another operation changes", async () => {
    const first = fakeHelm();
    await startHelmOperation(upgrade);
    const second = fakeHelm();
    await startHelmOperation({ ...upgrade, release: "web" });
    const before = getHelmOps();

    second.exit("gone");

    const after = getHelmOps();
    expect(after).not.toBe(before);
    // The row that did not change keeps its identity, so a strip row that is
    // not involved does not re-render.
    expect(after[0]).toBe(before[0]);
    expect(after[1]).not.toBe(before[1]);
    expect(after[1].state).toBe("failed");
    expect(first.handle.close).not.toHaveBeenCalled();
  });

  it("does not rebuild itself for an ending it has already recorded", async () => {
    const helm = fakeHelm();
    await startHelmOperation(upgrade);
    helm.exit("gone");
    const before = getHelmOps();
    const notified = vi.fn();
    subscribeHelmOps(notified);

    helm.exit("gone");

    expect(getHelmOps()).toBe(before);
    expect(notified).not.toHaveBeenCalled();
  });

  it("wakes subscribers when an operation starts, ends and is dismissed", async () => {
    const helm = fakeHelm();
    const notified = vi.fn();
    const unsubscribe = subscribeHelmOps(notified);

    const id = await startHelmOperation(upgrade);
    expect(notified).toHaveBeenCalledTimes(1);
    helm.exit("boom");
    expect(notified).toHaveBeenCalledTimes(2);
    dismissHelmOp(id);
    expect(notified).toHaveBeenCalledTimes(3);

    unsubscribe();
    fakeHelm();
    await startHelmOperation(upgrade);
    expect(notified).toHaveBeenCalledTimes(3);
  });
});
