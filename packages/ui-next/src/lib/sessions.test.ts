import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import type { Terminal } from "@xterm/xterm";

const startPodExec = vi.fn();
const startLocalTerminal = vi.fn();

vi.mock("@srelens/core", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@srelens/core")>();
  return {
    ...actual,
    startPodExec: (...args: unknown[]) => startPodExec(...args),
    startLocalTerminal: (...args: unknown[]) => startLocalTerminal(...args),
  };
});

// Imported after the mock so the store binds the doubles rather than the real
// Tauri-backed session openers.
const {
  SESSION_IDLE_AFTER_MS,
  __resetSessionsForTests,
  endSession,
  getSessions,
  startLocalSession,
  startPodSession,
  subscribeSessions,
  terminalFor,
} = await import("./sessions");

/**
 * A `startPodExec`/`startLocalTerminal` double. The test drives the backend
 * side through `out`/`exit`, and asserts on the handle the store was given.
 */
function fakeBackend() {
  let onData!: (chunk: string) => void;
  let onExit!: (error: string | null) => void;
  const handle = { send: vi.fn(), resize: vi.fn(), close: vi.fn() };
  const capture = (data: (c: string) => void, exit: (e: string | null) => void) => {
    onData = data;
    onExit = exit;
    return Promise.resolve(handle);
  };
  startPodExec.mockImplementation(
    (
      _context: string,
      _namespace: string,
      _pod: string,
      data: (c: string) => void,
      exit: (e: string | null) => void,
    ) => capture(data, exit),
  );
  startLocalTerminal.mockImplementation(
    (
      _context: string,
      _extra: string[],
      data: (c: string) => void,
      exit: () => void,
    ) => capture(data, () => exit()),
  );
  return {
    handle,
    out: (chunk: string) => onData(chunk),
    exit: (error: string | null = null) => onExit(error),
  };
}

/** The same double, but the test decides when the backend finishes opening —
 *  the window in which a session exists but has no far end yet. */
function deferredBackend() {
  let onData!: (chunk: string) => void;
  const handle = { send: vi.fn(), resize: vi.fn(), close: vi.fn() };
  let open!: () => void;
  const opened = new Promise<typeof handle>((resolve) => {
    open = () => resolve(handle);
  });
  startPodExec.mockImplementation(
    (_c: string, _n: string, _p: string, data: (c: string) => void) => {
      onData = data;
      return opened;
    },
  );
  return { handle, out: (chunk: string) => onData(chunk), open: () => open() };
}

/** What the emulator has actually rendered, top line first. */
function screenOf(term: Terminal | undefined, lines = 1): string[] {
  const buffer = term?.buffer.active;
  return Array.from({ length: lines }, (_, i) =>
    (buffer?.getLine(i)?.translateToString(true) ?? "").trimEnd(),
  );
}

const pod = {
  context: "kind-srelens-demo",
  namespace: "shop",
  pod: "checkout-api-5c8b7f2d9-mk3wl",
  container: "api",
};

beforeEach(() => {
  startPodExec.mockReset();
  startLocalTerminal.mockReset();
  __resetSessionsForTests();
});

afterEach(() => {
  __resetSessionsForTests();
  vi.useRealTimers();
});

describe("the session store", () => {
  it("lists a session it started, with the pod and container in its title", async () => {
    fakeBackend();
    const id = await startPodSession(pod);

    expect(getSessions()).toHaveLength(1);
    const row = getSessions()[0];
    expect(row.id).toBe(id);
    expect(row.kind).toBe("pod");
    expect(row.title).toBe("checkout-api-5c8b7f2d9-mk3wl · api");
    expect(row.context).toBe("kind-srelens-demo");
    expect(row.namespace).toBe("shop");
    expect(row.state).toBe("attached");
    expect(row.error).toBeUndefined();
    expect(row.startedAt).toBeGreaterThan(0);
  });

  it("starts a local shell with no namespace of its own", async () => {
    fakeBackend();
    const id = await startLocalSession({ context: "kind-srelens-demo" });

    const row = getSessions()[0];
    expect(row.id).toBe(id);
    expect(row.kind).toBe("local");
    expect(row.namespace).toBe("");
    expect(row.state).toBe("attached");
  });

  it("hands out the same emulator every time it is asked", async () => {
    fakeBackend();
    const id = await startPodSession(pod);

    const first = terminalFor(id);
    const second = terminalFor(id);
    expect(first).toBeDefined();
    // Identity, not equality: a store that built a fresh Terminal per call
    // would satisfy any structural check and still lose the scrollback.
    expect(second).toBe(first);
  });

  it("has no emulator for an id it never started", () => {
    expect(terminalFor(404)).toBeUndefined();
  });

  it("writes the backend's output into that session's emulator", async () => {
    const backend = fakeBackend();
    const id = await startPodSession(pod);

    backend.out("total 4\r\ndrwxr-xr-x  app\r\n");

    await vi.waitFor(() => {
      expect(screenOf(terminalFor(id), 2)).toEqual(["total 4", "drwxr-xr-x  app"]);
    });
  });

  it("keeps each session's output in its own emulator", async () => {
    const first = fakeBackend();
    const idA = await startPodSession(pod);
    const second = fakeBackend();
    const idB = await startPodSession({ ...pod, pod: "web-7" });

    first.out("from A\r\n");
    second.out("from B\r\n");

    await vi.waitFor(() => {
      expect(screenOf(terminalFor(idA))).toEqual(["from A"]);
      expect(screenOf(terminalFor(idB))).toEqual(["from B"]);
    });
  });

  it("sends what the reader types in the emulator to the session", async () => {
    const backend = fakeBackend();
    const id = await startPodSession(pod);

    terminalFor(id)?.input("ls\r");

    expect(backend.handle.send).toHaveBeenCalledWith("ls\r");
  });

  it("tells the PTY when the emulator is resized", async () => {
    const backend = fakeBackend();
    const id = await startPodSession(pod);

    terminalFor(id)?.resize(142, 44);

    expect(backend.handle.resize).toHaveBeenCalledWith(142, 44);
  });

  it("opens the PTY at the emulator's own size", async () => {
    const backend = fakeBackend();
    const id = await startPodSession(pod);

    const term = terminalFor(id);
    expect(startPodExec).toHaveBeenCalledWith(
      "kind-srelens-demo",
      "shop",
      "checkout-api-5c8b7f2d9-mk3wl",
      expect.any(Function),
      expect.any(Function),
      "api",
      undefined,
      { cols: term?.cols, rows: term?.rows },
    );
    expect(backend.handle.close).not.toHaveBeenCalled();
  });

  it("keeps a session that ended listed as closed, carrying why", async () => {
    const backend = fakeBackend();
    const id = await startPodSession(pod);

    backend.exit("handler error: container api is not running");

    expect(getSessions()).toHaveLength(1);
    const row = getSessions()[0];
    expect(row.id).toBe(id);
    expect(row.state).toBe("closed");
    // Through describeError: the reader never sees the handler prefix.
    expect(row.error).not.toContain("handler error:");
    expect(row.error).toContain("container api is not running");
  });

  it("keeps the scrollback of a session that ended", async () => {
    const backend = fakeBackend();
    const id = await startPodSession(pod);

    backend.out("last words\r\n");
    await vi.waitFor(() => expect(screenOf(terminalFor(id))).toEqual(["last words"]));
    backend.exit("gone");

    expect(getSessions()[0].state).toBe("closed");
    expect(screenOf(terminalFor(id))).toEqual(["last words"]);
  });

  it("closes cleanly with no reason when the shell simply exited", async () => {
    const backend = fakeBackend();
    await startPodSession(pod);

    backend.exit(null);

    expect(getSessions()[0].state).toBe("closed");
    expect(getSessions()[0].error).toBeUndefined();
  });

  it("leaves a closed row behind when the session could not be started", async () => {
    startPodExec.mockRejectedValue(new Error("handler error: pods is forbidden"));

    const id = await startPodSession(pod);

    const row = getSessions()[0];
    expect(row.id).toBe(id);
    expect(row.state).toBe("closed");
    // Through describeError: neither the handler prefix nor the apiserver's
    // own word for it survives to the reader.
    expect(row.error).not.toContain("handler error:");
    expect(row.error).not.toContain("forbidden");
    expect(row.error).toContain("permission");
  });

  it("has an emulator ready before the backend has finished opening", async () => {
    const backend = deferredBackend();
    const started = startPodSession(pod);

    // The row and its emulator exist while the connect is still in flight, so
    // the first prompt has somewhere to land.
    const id = getSessions()[0].id;
    backend.out("first prompt\r\n");
    backend.open();
    await started;

    await vi.waitFor(() => expect(screenOf(terminalFor(id))).toEqual(["first prompt"]));
  });

  it("closes a session the reader ended while it was still opening", async () => {
    const backend = deferredBackend();
    const started = startPodSession(pod);
    const id = getSessions()[0].id;

    endSession(id);
    backend.open();
    await started;

    // The PTY the backend went on to open has no row left to belong to; it is
    // closed rather than left running with nothing pointing at it.
    expect(backend.handle.close).toHaveBeenCalledTimes(1);
    expect(getSessions()).toEqual([]);
  });

  it("removes a session the reader ended, and disposes its emulator", async () => {
    const backend = fakeBackend();
    const id = await startPodSession(pod);
    const term = terminalFor(id);
    // Our own listener, not the store's: unwiring the store's would silence
    // its own handler and leave this one firing, so only a real dispose can
    // make the emulator inert here.
    const typed: string[] = [];
    term?.onData((d) => typed.push(d));

    endSession(id);

    expect(getSessions()).toEqual([]);
    expect(backend.handle.close).toHaveBeenCalledTimes(1);
    expect(terminalFor(id)).toBeUndefined();
    term?.input("x");
    expect(typed).toEqual([]);
  });

  it("ends a session only once, however often the reader asks", async () => {
    const backend = fakeBackend();
    const id = await startPodSession(pod);

    endSession(id);
    endSession(id);

    expect(backend.handle.close).toHaveBeenCalledTimes(1);
    expect(getSessions()).toEqual([]);
  });

  it("removes a closed row when the reader dismisses it", async () => {
    const backend = fakeBackend();
    const id = await startPodSession(pod);
    backend.exit("gone");

    endSession(id);

    expect(getSessions()).toEqual([]);
    expect(terminalFor(id)).toBeUndefined();
  });

  it("reports a session that goes idle, and again when it speaks", async () => {
    vi.useFakeTimers({ now: new Date("2026-08-25T10:00:00.000Z") });
    const backend = fakeBackend();
    const id = await startPodSession(pod);

    vi.advanceTimersByTime(SESSION_IDLE_AFTER_MS);
    expect(getSessions()[0].state).toBe("idle");

    backend.out("$ ");
    expect(getSessions()[0].state).toBe("attached");
    expect(getSessions()[0].lastOutputAt).toBe(Math.floor(Date.now() / 1000) * 1000);
    expect(terminalFor(id)).toBeDefined();
  });

  it("does not wake a closed session back up with an idle timer", async () => {
    vi.useFakeTimers({ now: new Date("2026-08-25T10:00:00.000Z") });
    const backend = fakeBackend();
    await startPodSession(pod);

    backend.exit("gone");
    vi.advanceTimersByTime(SESSION_IDLE_AFTER_MS * 2);

    expect(getSessions()[0].state).toBe("closed");
  });
});

describe("the store's snapshot", () => {
  it("is the same reference until something changes", async () => {
    fakeBackend();
    await startPodSession(pod);

    const before = getSessions();
    expect(getSessions()).toBe(before);
  });

  it("survives a chatty second of output", async () => {
    vi.useFakeTimers({ now: new Date("2026-08-25T10:00:00.000Z") });
    const backend = fakeBackend();
    await startPodSession(pod);
    backend.out("line 1\r\n");
    const before = getSessions();
    const notified = vi.fn();
    subscribeSessions(notified);

    // A shell tailing a log: many chunks, spread across real milliseconds,
    // inside one second. Firing them on a frozen clock would prove nothing —
    // millisecond stamps would hold their identity too.
    for (let i = 0; i < 50; i++) {
      vi.advanceTimersByTime(10);
      backend.out(`line ${i + 2}\r\n`);
    }
    expect(Date.now() - before[0].lastOutputAt).toBeGreaterThan(400);

    expect(getSessions()).toBe(before);
    expect(notified).not.toHaveBeenCalled();
  });

  it("re-stamps a running session once the second has turned", async () => {
    vi.useFakeTimers({ now: new Date("2026-08-25T10:00:00.000Z") });
    const backend = fakeBackend();
    await startPodSession(pod);
    backend.out("line 1\r\n");
    const before = getSessions();
    const notified = vi.fn();
    subscribeSessions(notified);

    vi.advanceTimersByTime(1000);
    backend.out("line 2\r\n");
    backend.out("line 3\r\n");

    const after = getSessions();
    expect(after).not.toBe(before);
    expect(after[0].lastOutputAt).toBe(before[0].lastOutputAt + 1000);
    expect(notified).toHaveBeenCalledTimes(1);
  });

  it("leaves an untouched row alone when another session changes", async () => {
    const first = fakeBackend();
    await startPodSession(pod);
    const second = fakeBackend();
    await startPodSession({ ...pod, pod: "web-7" });
    const before = getSessions();

    second.exit("gone");

    const after = getSessions();
    expect(after).not.toBe(before);
    // The row that did not change keeps its identity, so a rail row that is
    // not involved does not re-render.
    expect(after[0]).toBe(before[0]);
    expect(after[1]).not.toBe(before[1]);
    expect(after[1].state).toBe("closed");
    expect(first.handle.close).not.toHaveBeenCalled();
  });

  it("does not rebuild itself for a closure it has already recorded", async () => {
    const backend = fakeBackend();
    await startPodSession(pod);
    backend.exit("gone");
    const before = getSessions();
    const notified = vi.fn();
    subscribeSessions(notified);

    backend.exit("gone");

    expect(getSessions()).toBe(before);
    expect(notified).not.toHaveBeenCalled();
  });

  it("wakes subscribers when a session starts, ends and is removed", async () => {
    const backend = fakeBackend();
    const notified = vi.fn();
    const unsubscribe = subscribeSessions(notified);

    const id = await startPodSession(pod);
    expect(notified).toHaveBeenCalledTimes(1);
    backend.exit("gone");
    expect(notified).toHaveBeenCalledTimes(2);
    endSession(id);
    expect(notified).toHaveBeenCalledTimes(3);

    unsubscribe();
    fakeBackend();
    await startPodSession(pod);
    expect(notified).toHaveBeenCalledTimes(3);
  });
});
